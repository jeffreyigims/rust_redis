# A Key-Value Service in Rust from Scratch 

The goal of the project is to build a key-value storage service that's written in rust and included minimal external dependencies. I.e. I implement my own version of an asynchronous input/output using a non-blocking custom-build event loop that only depends on `libc` and `std::net`. I was inspired by this guide to [build your own redis server](https://build-your-own.org/redis/#table-of-contents) in c++.

## Running the Project

### Running

Running the server and/or client:

```
cargo run --bin server 

cargo run --bin client 
```

Note: add `--release` to compile with optimizations for performance tests 

Also note the client is mostly used for performance testing right now. 

### Testing

To run all unit tests in the project:

```
cargo test -- --nocapture
```

### Debugging

To debug with `gdb` or `lldb` (on macOS):

`rust-gdb` and `rust-lldb` are wrappers over `gdb` and `lldb` that come with the rust installation and can pretty-print rust data types. 

```
cargo build --bin server
rust-{gdb, lldb} ./target/release/server 
```

### Performance Tests

The client can be used for testing `latency` (in microseconds) and `throughput` of the server. It calculates `p50`, `p95`, and `p99` percentiles as well as `min`, `max`, and `avg` request latencies.  

The client spawns multiple threads to issue requests to the server, where each request is a sequence of `SET k v`, `GET k`, `DELETE k`, where `k` and `v` are randomly generated hex of size specified by `--key-length`. The number of clients and requests are also configurables by `--clients` and `--requests` respectively. 

## Guide

The goal of this guide is to explain and highlight some key design decisions. The following sections outline some key aspects of the server implementation.

See the full codebase at https://github.com/jeffreyigims/rust_redis. The full guide can also be found in the `README`.

### Networking

The server is a process that we can spawn to serve requests on some parameterized port on our localhost. The server supports TCP-based connections and we use the `net::TcpListener` standard module as our networking interface. I'm pretty sure this is just a slim wrapper over `libc`.

A quick primer on socket programming: it's an abstraction to facilitate communication between nodes on a computer network. Sockets are endpoints (represented as file descriptors on unix-like systems) which we can bind to specific addresses (host + port) to send and receive data. 

When our server starts, it creates an initial TCP socket that's bound to some parameterized port on our localhost, and proceeds to listen on. This socket will be used to accept and process incoming requests from clients. 

```
pub fn start(&mut self) -> Result<()> {
    // define the address and port the server will listen on
    let addr: (IpAddr, u16) = ([127, 0, 0, 1].into(), self.port);

    // create a socket, bind to localhost:PORT, and listen on it
    let listener = TcpListener::bind(&addr)?;
    ...
}
```

### The Protocol 

We use a request-response simple binary-based protocol that uses length prefixes. There are pros and cons to using test vs. binary and prefixes vs. delimiters which we'll not go into here. 

The first byte is reserved for the type of request. Then comes the arguments or sequence of arguments. Each argument is defined as the first 4 bytes encoding the size `n` of the payload and the following `n` bytes which is the actual payload. 

| Field          | Size (bytes) | Description                                                                 |
|----------------|--------------|-----------------------------------------------------------------------------|
| Request Type   | 1 | The first byte is reserved for the type of request (`GET`, `SET`, or `DELETE`)                     |
| Argument Size  | 4 | Each argument starts with 4 bytes encoding the size `n` of the payload.     |
| Argument Data  | `n` | The following `n` bytes encode the actual payload for each argument |
|...|...|...|

The `GET` and `DELETE` commands require a single arg for the key while `SET` requires as second for the value. 

```
pub enum Operation {
    Get { key: String },
    Delete { key: String },
    Set { key: String, value: String },
}
```

### Concurrent Programming Model 

There's a couple approaches to perform concurrent programming and/or parallel. The most common approaches are using multi-process/multi-threading or using an event-based model. For IO heavy workloads, the latter usually wins out in terms of scalability. This is because the former requires processing each incoming request in a separate thread (in the multi-threading approach). This can lead to both memory concerns (since each thread requires a separate stack) and performance constraints, due to costly context switches and shared resource contention, especially if we need to scale to thousands of connections. We implement the latter. 

We maintain a list of current `Connection`s to the server:

```
pub struct Connection {
    pub stream: TcpStream,

    // just tracks if there's data in our write_buffer
    pub want_to_write: bool,
    // client sent 0 bytes
    pub want_close: bool,

    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,
}
```

This is how maintain the state of reach request. A `Connection` is initialized after each incoming TCP connection is `accept`ed. We maintain two buffers for incoming and outgoing data. `want_to_write` indicates we have data in our `write_buffer` we should send back the client. `want_close` indicates the client closed it's side of the connection and we should close our side.

The server implements an event loop that continuously `poll`s open connections and checks if they are ready to be read from or write to if applicable. `poll` is a syscall provided by the operating system.

```
while self.run.load(Ordering::SeqCst) {
    ...
    let mut fds: Vec<pollfd> = Vec::new();
    fds.push(pollfd {
        fd,
        events: POLLIN,
        revents: 0,
    });

    for (&fd, conn) in connections.iter() {
        fds.push(pollfd {
            fd,
            events: POLLERR | POLLIN,
            revents: 0,
        });

        if conn.want_to_write {
            fds.push(pollfd {
                fd,
                events: POLLOUT,
                revents: 0,
            });
        }
    }
    ...
}
```

We want to `poll` the file descriptor of our listening socket as well as the fds for all of our connections. We `poll` for read-readiness and unexpected errors on our listening socket's fd as well as on all of our open connections. We `poll` for write-readiness on our connections if they `want_to_write`. 

Now invoke `poll`: 

```
while self.run.load(Ordering::SeqCst) {
    ...
    // this should be the only non-blocking call
    let result = unsafe { poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
    if result == -1 {
        println!("poll call failed: {}", io::Error::last_os_error());
        return Err(anyhow::Error::from(io::Error::last_os_error()));
    }
    ...
}
```

The `poll` syscall tells the OS to check for read and/or write readiness on each of our fds (`epoll` is actually probably more efficient but let's keep it simple for now). 

Note that we have to wrap all foreign-function interface (FFI) calls in `unsafe` since the compile can't guarantee memory safety across language boundaries. This doesn't mean the calls are incorrect but that it's up to the programmer to ensure safety. 

Check the `revent` of our listening socket first and see if we can `accept` and add a new `Connection`:

```
while self.run.load(Ordering::SeqCst) {
    ...
    if fds[0].revents & POLLIN != 0 {
        println!("TcpListener is ready to accept a connection.");
        match listener.accept() {
            Ok((socket, _addr)) => {
                let fd = socket.as_raw_fd();
                connections.insert(fd, Connection::from_stream(socket));
            }
            Err(e) => Err(anyhow::Error::from(e))?,
        }
    }
    ...
}
```

Then, iterate through `Connection`s ready for action and act accordingly:

First, check and see if we encountered some unexpected error when `poll`ing our file descriptor, or if our connection `want_close` (client closed it's side of the connection). 

```
...
for fd in &fds[1..] {
    if fd.revents & POLLERR != 0 || connections.get(&fd.fd).unwrap().want_close {
        println!("Error on file descriptor: {}", fd.fd);
        // socket will be closed when it goes out of scope
        connections.remove(&fd.fd);
    }
    ...
}
...
```

If not, check for read-readiness or if a connection is ready to be read from (data arrived from the network card/interface to our OS's per-socket read buffer): 

```
...
for fd in &fds[1..] {
    ...
    if fd.revents & POLLIN != 0 {
        let conn = connections.get_mut(&fd.fd).unwrap();
        conn.read()?;
        while let Some(oper) = conn.handle_read()? {
            self.handle_request(conn, oper)?;
        }
        ...
    }
    ...
}
...
```

Assuming read-readiness, try and read as many bytes as possible:

```
pub fn read(&mut self) -> Result<()> {
    let mut buf = [0; 32 * 1024];
    let result = self.stream.read(&mut buf);
    if let Err(ref e) = result {
        // retryable error
        if e.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(());
        }
    }

    let bytes_read = result?;

    if bytes_read == 0 {
        self.want_close = true;
        return Ok(());
    }

    self.read_buffer.extend_from_slice(&buf[..bytes_read]);

    Ok(())
}
```

If a connection is ready to be read from (data arrived from the network interface to our OS's per-socket read buffer), we `handle_read`. I.e. try to parse an operation:

```
pub fn handle_read(&mut self) -> Result<Option<Operation>> {
    if self.read_buffer.is_empty() {
        return Ok(None);
    }

    let op = self.read_buffer[0];

    let Some(key) = self.try_parse(1)? else {
        return Ok(None);
    };

    let mut total_len = 1 + HEADER_SIZE + key.len();
    let oper = match op {
        0x01 => Operation::Get { key },
        0x02 => Operation::Delete { key },
        0x03 => {
            let Some(value) = self.try_parse(1 + HEADER_SIZE + key.len())? else {
                return Ok(None);
            };
            total_len += HEADER_SIZE + value.len();
            Operation::Set { key, value }
        }
        _ => return Err(anyhow!("Invalid operation")),
    };

    self.read_buffer.drain(..total_len);

    Ok(Some(oper))
}
```

Note: see the section on pipelined requests for an explanation of why we `handle_read` in a `while` loop. 

`try_parse` attempts to parse a full operation from a connection's `read_buffer`:

```
fn try_parse(&mut self, index: usize) -> Result<Option<Vec<u8>>> {
    let buf = &self.read_buffer[index..];

    if buf.len() < HEADER_SIZE {
        return Ok(None);
    }

    let message_len = u32::from_le_bytes(buf[..HEADER_SIZE].try_into()?);

    let total_len = HEADER_SIZE + message_len as usize;
    if buf.len() < total_len {
        return Ok(None);
    }

    let message = buf[HEADER_SIZE..total_len].to_vec();

    Ok(Some(message))
}
```

If we can't parse a full operation, we'll parse for read-readiness for the connection in the next loop iteration and try again. If we can, we try and execute it:

```
fn handle_request(&mut self, connection: &mut Connection, oper: Operation) -> Result<()> {
    match oper {
        Operation::Get { key } => {
            if let Some(value) = self.store.get(&key) {
                connection.write(value)
            } else {
                connection.write(b"")
            }
        }
        Operation::Delete { key } => {
            let v = self.store.remove(&key).unwrap_or("".into());
            connection.write(&v)
        }
        Operation::Set { key, value } => {
            let v = self.store.insert(key, value).unwrap_or("".into());
            connection.write(&v)
        }
    }
}
```

Nothing fancy here. We're just using rust's standard `HashMap` implementation. I hope to improve on this and roll my own. 

Depending on the operation, we'll add some data to the `write_buffer` to send back to the client. The code here is self-explanatory. 

When we call `write`, we add data to the `write_buffer` and the connection transitions to the `want_to_write` state:

```
pub fn write(&mut self, message: &[u8]) -> Result<()> {
    let len: u32 = message.len().try_into()?;
    self.write_buffer.extend_from_slice(&len.to_le_bytes());
    self.write_buffer.extend_from_slice(message);
    self.want_to_write = true;
    Ok(())
}
```

Now in the next iteration of the event loop, we'll `poll` the socket for write readiness (if our client is ready to receive our response and the OS's per-socket write buffer isn't full)

```
...
for fd in &fds[1..] {
    ...
    if fd.revents & POLLOUT != 0 {
        let conn = connections.get_mut(&fd.fd).unwrap();
        assert!(conn.want_to_write);
        conn.handle_write()?;
    }
}
...
```

Assuming write-readiness:

```
pub fn handle_write(&mut self) -> Result<()> {
    let result = self.stream.write(&self.write_buffer);
    if let Err(ref e) = result {
        // retryable error
        if e.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(());
        }
    }

    let bytes_written = result?;
    self.write_buffer.drain(..bytes_written);
    if self.write_buffer.is_empty() {
        self.want_to_write = false;
    }
    Ok(())
}
```

Attempt to flush as much data from the write buffer as we can. If our socket is temporarily unable to accept more data for some reason (e.g. buffer is full, resource availability, etc.), drain the bytes written from `write_buffer` and continue `poll`ing for write-readiness. If we flush and drain all the bytes, unset `want_to_write` to stop `poll`ing for write-readiness. We still/always `poll` for read-readiness to potentially process more requests. 

### Read-Path Optimization

A small optimization we can make in our read-path is to potentially write a response(s) back to the client in the same loop iteration, saving us a syscall to check for write-readiness in the next iteration. 

If our client is still sending pipelined requests or isn't ready to receive responses back for some other reason, we'll simple get a retryable error `WouldBlock` back from our OS in `handle_write`.

```
for fd in &fds[1..] {
    ...
    if fd.revents & POLLIN != 0 {
        let conn = connections.get_mut(&fd.fd).unwrap();
        conn.read()?;
        while let Some(oper) = conn.handle_read()? {
            self.handle_request(conn, oper)?;
        }
        /*
            We can optimize here and handle_write before the next loop iteration to potentially avoid an
            extra syscall before responding to the client. That is, if the client is ready to receive a
            response and not still sending pipelined requests.
        */
        conn.handle_write()?;
    }
    ...
}
```

### Pipelined Requests

Pipelined requests are requests that are sent by a client sequentially without waiting for responses from proceeding requests. I.e. a client may send multiple requests sequentially and some time later read their responses, expecting the same ordering of responses that was used in the requests. Pipelining can reduce IO operations, which can improve overall throughput and reduces client-side latency, since multiple requests can be processed in a single round trip to the server.    

Our implementation inherently supports pipelined requests. When a new connection is `accept`ed and ready to be read from, we attempt to process all potential requests before responding to the client. 

```
for fd in &fds[1..] {
    ...
    if fd.revents & POLLIN != 0 {
        let conn = connections.get_mut(&fd.fd).unwrap();
        conn.read()?;
        while let Some(oper) = conn.handle_read()? {
            self.handle_request(conn, oper)?;
        }
        ...
    }
    ...
}
```

### Benchmarks 

Running on my M1 MacBook Pro with 10 cores, I top out around 35k OPs/S. Run with 30 threads submitting 50 requests each with a key length of 4 bytes:

```
cargo run --bin client --release -- --clients 30 --requests 50

Finished `release` profile [optimized] target(s) in 0.25s
     Running `target/release/client --clients 30 --requests 50`
Min: 213.00, Max: 3031.00, Average: 836.87, P50: 717.00, P95: 1334.15, P99: 2933.00
OPs/S: 34474.83
```

Latency is in microseconds.

### Future Optimizations and Next Steps 

A couple optimizations I can think of which aren't implemented yet:
* We process requests on the same thread we use to accept new connections. We can decouple this and have one thread responsible for accepting new connections and a pool of worker threads for processing requests. Note this would require careful synchronization of our key-value store across threads 
* We can add better buffer management in our `Connection` struct. We currently `append` data back to the back and `remove` from the front. The former operation is pretty efficient with `Vec` since the structure reallocates exponentially to amortize allocation costs. The latter operation is not since we need to potentially "shift" all requests to the front when we remove one. We can probably think of a more clever data structure where both operations are efficient. 
* Replace `poll` with `epoll` which is more efficient for monitoring large numbers of file descriptors since it is an event-driven model. I.e. with `poll`, we iterate and check all fds per call. With `epoll`, we can register fds we're interested in up-front and only be notified on specific `revent`s.  

Feel free to suggest more!

I'll probably try and follow-up by implementing the actual key-value store from scratch as well, as described in the advanced topics section of the [original guide](https://build-your-own.org/redis/#table-of-contents).


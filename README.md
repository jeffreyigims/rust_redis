# A Key-Value Service in Rust from Scratch 

The goal of the project is to build a key-value storage service that's written in rust and included minimal external dependencies. I.e. I implement my own version of an asynchronous input/output using a non-blocking custom-build event loop that only depends on `libc` and `std::net`.

The goal of this guide is to document the steps that built up to the final server. `TODO expand on this`

The following sections outline some key aspects of the server implementation.

## Networking

The server is a process that we can spawn to serve requests on some paramtarized port on our localhost. The server supports TCP-based connections and we use the `net::TcpListener` standard module as our networking interface. I'm pretty sure this is just a slim wrapper over `libc`.

A quick primer on socket programming: it's an abstraction to facilitate communication between nodes on a computer network. Sockets are endpoints (represented as file descriptors on unix-like systems) which we can bind to specific addresses (host + port) to send and recieve data. 

When our server starts, it creates an initial TCP socket that's bound to some parameterized port on our localhost, and proceeds to listen on. This socket will be used to accept and process incoming requests from clients. 

```
pub fn start(&mut self) -> Result<()> {
    // define the address and port the server will listen on
    let addr: (IpAddr, u16) = ([127, 0, 0, 1].into(), self.port);

    // create a socket, bind to localhost:PORT, and listen on it
    let listener = TcpListener::bind(&addr)?;

    ...
```

## The Protocol 

We use a request-repspone simple binary-based protocol that uses length prefixes. There are pros and cons to using test vs. binary and prefixes vs. delimiters which we'll not go into here. 

The first 2 bytes are reserved for the type of request. Then comes the arguments or sequence of arguments. Each argument is defined as the first 4 bytes encoding the size `n` of the payload and the following `n` bytes which is the actual payload. 

| Field          | Size (bytes) | Description                                                                 |
|----------------|--------------|-----------------------------------------------------------------------------|
| Request Type   | 2 | The first 2 bytes are reserved for the type of request (`GET`, `SET`, or `DELETE`)                     |
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

## Concurrent Programming Model 

There's a couple approaches to handle concurrent IO. The most common approaches are using multi-process/multi-threading or using an event-based model. For IO heavy workloads, the latter usually wins out in terms of scalability. This is because the former requires processsing each incoming request in a separate thread (in the multi-threading approach). This can lead to both memory concerns (since each thread requires a separate stack) and performance constraints, due to costly context switches and shared resource contention, espcially if we need to scale to thousands of connections. We implement the latter. 

We maintain a list of current `Connection`s to the server:

```
pub struct Connection {
    pub stream: TcpStream,

    pub want_to_read: bool,
    pub want_to_write: bool,
    pub want_close: bool,

    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,
}
```

`TODO: add description of the state machine for 'Connection'`

The server implements an event loop that continously `poll`s open connections and checks if they are ready to be read from or write to if applicable. `poll` is a syscall provided by the operating system.

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
            events: POLLERR,
            revents: 0,
        });

        if conn.want_to_read {
            fds.push(pollfd {
                fd,
                events: POLLIN,
                revents: 0,
            });
        }

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

Check the file descriptor for our listening socket first and see if we can `accpet` and add a new `Connection`:

```
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
```

Then, iterate through `Connection`s ready for action and act accordingly:

```
...
for fd in &fds[1..] {
    if fd.revents & POLLERR != 0 || connections.get(&fd.fd).unwrap().want_close {
        println!("Error on file descriptor: {}", fd.fd);
        // socket will be closed when it goes out of scope
        connections.remove(&fd.fd);
    }
    if fd.revents & POLLIN != 0 {
        let conn = connections.get_mut(&fd.fd).unwrap();
        conn.read()?;
        while let Some(oper) = conn.handle_read()? {
            self.handle_request(conn, oper)?;
            /*
                We can optimize here and handle_write before the next loop iteration to potentially avoid an extra syscall before
                responding to the client. That is, if the client is ready to recieve a response and not still sending pipelined requests. 
            */
            conn.handle_write()?;
        }
    }
    if fd.revents & POLLOUT != 0 {
        connections.get_mut(&fd.fd).unwrap().handle_write()?;
    }
}
...
```

The read path `TOOD`

The write path `TODO`

## Pipelined Requests

Pipelined requests are requests that are sent by a client sequentially without waiting for responses from proceeding requests. I.e. a client may send multiple requests sequentially and some time later read their reponses, expecting the same ordering of responses that was used in the requests. Pipelining can reduce IO operations, which can improve overall throughput and reduces client-side latency, since multple requests can be processed in a single round trip to the server.    

Our implementation inherently supports pipelined requests. When a new connection is made, we attempt to process all requests before sending any responses. 

```
...
if fd.revents & POLLIN != 0 {
    let conn = connections.get_mut(&fd.fd).unwrap();
    conn.read()?;
    while let Some(oper) = conn.handle_read()? {
        self.handle_request(conn, oper)?;
        /*
            We can optimize here and handle_write before the next loop iteration to potentially avoid an extra syscall before
            responding to the client. That is, if the client is ready to recieve a response and not still sending pipelined requests. 
        */
        conn.handle_write()?;
    }
}
...
```

## Benchmarks 

`TODO`

## Future Optimizations and Next Steps 

A couple optimizations I can think of which aren't implemented yet:
* I suspect we spend a lot of cycles decoding the binary keys and values to ensure they're valid UTF-8 before converting and storing them as `String`s. We can probably just store them as `bytes` directly. Our server doesn't care what their UTF-8 interpretation is
* We process requests on the same thread we use to accept new connections. We can decouple this and have one thread responsible for accepting new connections and a pool of worker threads for processing requests. Note this would require careful synchronization of our key-value store across threads 
* We can add better buffer managment in our `Connection` struct. `TODO`

I'll probably try and follow-up by implementing the actual key-value store from scratch as well, as described in the advanced topics section of the [original guide](https://build-your-own.org/redis/#table-of-contents).


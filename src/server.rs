use anyhow::Result;
use clap::Parser;
use libc::{poll, pollfd, POLLERR, POLLIN, POLLOUT};
use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::net::TcpListener;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

mod connection;
use connection::Connection;
use connection::Operation;

pub struct Server {
    port: u16,
    run: Arc<AtomicBool>,
    tx: mpsc::Sender<String>,
    store: HashMap<String, String>,
}

impl Server {
    pub fn new(
        maybe_port: Option<u16>,
        run: Arc<AtomicBool>,
        tx: mpsc::Sender<String>,
    ) -> Result<Self> {
        let port = maybe_port.unwrap_or(0);
        Ok(Server {
            port,
            run,
            tx,
            store: HashMap::new(),
        })
    }

    fn handle_request(&mut self, connection: &mut Connection, oper: Operation) -> Result<()> {
        println!("Client said: {:#?}", oper);

        match oper {
            Operation::Get { key } => {
                if let Some(value) = self.store.get(&key) {
                    connection.write(value)
                } else {
                    connection.write("")
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

    pub fn start(&mut self) -> Result<()> {
        // define the address and port the server will listen on
        let addr: (IpAddr, u16) = ([127, 0, 0, 1].into(), self.port);

        // create a socket, bind to localhost:PORT, and listen on it
        let listener = TcpListener::bind(&addr)?;
        let actual_port = listener.local_addr()?.port();
        self.tx.send(actual_port.to_string())?;
        listener.set_nonblocking(true)?;
        let fd: RawFd = listener.as_raw_fd();

        let mut connections: HashMap<RawFd, Connection> = HashMap::new();
        while self.run.load(Ordering::SeqCst) {
            let mut fds: Vec<pollfd> = Vec::new();
            fds.push(pollfd {
                fd,
                events: POLLIN,
                revents: 0,
            });

            for (&fd, conn) in connections.iter() {
                let mut fd = pollfd {
                    fd,
                    events: POLLERR,
                    revents: 0,
                };

                if conn.want_to_read {
                    fd.events |= POLLIN;
                }

                if conn.want_to_write {
                    fd.events |= POLLOUT;
                }

                fds.push(fd);
            }

            // this should be the only non-blocking call
            let result = unsafe { poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
            if result == -1 {
                eprintln!("poll call failed: {}", io::Error::last_os_error());
                return Err(anyhow::Error::from(io::Error::last_os_error()));
            }

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

            for fd in &fds[1..] {
                print!("File descriptor {} is ready: ", fd.fd);
                if fd.revents & POLLERR != 0 || connections.get(&fd.fd).unwrap().want_close {
                    println!("Closing file descriptor: {}", fd.fd);
                    // socket will be closed when it goes out of scope
                    let conn = connections.remove(&fd.fd).unwrap();
                    println!("Connection closed: {:#?}", conn);
                    continue;
                }
                if fd.revents & POLLIN != 0 {
                    let conn = connections.get_mut(&fd.fd).unwrap();
                    assert!(conn.want_to_read);
                    conn.read()?;
                    while let Some(oper) = conn.handle_read()? {
                        self.handle_request(conn, oper)?;
                        /*
                            We can potentially optimize here and handle_write to avoid an extra syscall before
                            reposning to the client. If we're doing request-response, we can assume the client
                            is ready for a response, but if we're not, we'd have to check the client is ready first
                        */
                        conn.handle_write()?;
                    }
                }
                if fd.revents & POLLOUT != 0 {
                    let conn = connections.get_mut(&fd.fd).unwrap();
                    assert!(conn.want_to_write);
                    conn.handle_write()?;
                }
            }
        }

        // we don't need to close the socket connection since it's tied to the lifetime of `socket`
        Ok(())
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Port to run the server on
    #[arg(short, long, default_value_t = 3000)]
    port: u16,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (tx, _rx) = mpsc::channel();

    let mut server = Server::new(Some(args.port), Arc::new(AtomicBool::new(true)), tx)?;
    server.start()
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::thread;
    use std::time::Duration;

    // helper function to start the server in a separate thread
    fn start_test_server(
        server_running: Arc<AtomicBool>,
        tx: mpsc::Sender<String>,
    ) -> thread::JoinHandle<()> {
        let mut server = Server::new(None, server_running, tx).unwrap();
        thread::spawn(move || {
            server.start().unwrap();
        })
    }

    fn setup() -> (Arc<AtomicBool>, u16) {
        let sever_running = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();
        let _server_thread = start_test_server(Arc::clone(&sever_running), tx);
        // give the server some time to start
        thread::sleep(Duration::from_millis(100));
        let port = rx.recv().unwrap().parse().unwrap();
        println!("Server started on port {}", port);
        (sever_running, port)
    }

    fn teardown(server_running: Arc<AtomicBool>) {
        server_running.store(false, Ordering::SeqCst);
        // give the server some time to stop
        thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn test_basic_opts() -> Result<()> {
        let (server_running, port) = setup();

        let mut stream = Connection::from_port(port)?;
        stream.set(b"hello", b"world")?;
        let response = stream.read_blocking()?;
        assert_eq!(&response, "");

        stream.get(b"hello")?;
        let response = stream.read_blocking()?;
        assert_eq!(&response, "world");

        stream.delete(b"hello")?;
        let response = stream.read_blocking()?;
        assert_eq!(&response, "world");

        teardown(server_running);

        Ok(())
    }

    #[test]
    fn test_multiple_clients() -> Result<()> {
        let (server_running, port) = setup();

        let mut stream1 = Connection::from_port(port)?;
        let mut stream2 = Connection::from_port(port)?;

        stream1.set(b"hello", b"world")?;
        let response = stream1.read_blocking()?;
        assert_eq!(&response, "");
        stream2.get(b"hello")?;
        let response = stream2.read_blocking()?;
        assert_eq!(&response, "world");

        teardown(server_running);

        Ok(())
    }

    #[test]
    fn test_pipelined_requests() -> Result<()> {
        let (server_running, port) = setup();

        let mut stream = Connection::from_port(port)?;

        let buf = b"\x03\x05\x00\x00\x00hello\x05\x00\x00\x00world\x01\x05\x00\x00\x00hello\x02\x05\x00\x00\x00hello";
        stream.stream.write_all(buf)?;
        stream.stream.flush()?;

        let mut res_buf = [0; 22];
        stream.stream.read_exact(&mut res_buf)?;
        assert_eq!(
            &res_buf,
            b"\x00\x00\x00\x00\x05\x00\x00\x00world\x05\x00\x00\x00world"
        );

        teardown(server_running);

        Ok(())
    }
}

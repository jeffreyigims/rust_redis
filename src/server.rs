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

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Port to run the server on
    #[arg(short, long, default_value_t = 3000)]
    port: u16,
}

fn handle_request(connection: &mut Connection, message: &str) -> Result<()> {
    println!("Client said: {}", message);
    connection.write(message)?;
    Ok(())
}
fn start_server(port: Option<u16>, run: Arc<AtomicBool>, tx: mpsc::Sender<String>) -> Result<()> {
    // define the address and port the server will listen on
    let addr: (IpAddr, u16) = ([127, 0, 0, 1].into(), port.unwrap_or(0));

    // create a socket, bind to localhost:PORT, and listen on it
    let listener = TcpListener::bind(&addr)?;
    let port = listener.local_addr()?.port();
    tx.send(port.to_string())?;
    listener.set_nonblocking(true)?;
    let fd: RawFd = listener.as_raw_fd();

    let mut connections: HashMap<RawFd, Connection> = HashMap::new();
    while run.load(Ordering::SeqCst) {
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
            if fd.revents & POLLERR != 0 {
                println!("Error on file descriptor: {}", fd.fd);
                // socket will be closed when it goes out of scope
                connections.remove(&fd.fd);
            }
            if fd.revents & POLLIN != 0 {
                let conn = connections.get_mut(&fd.fd).unwrap();
                conn.read()?;
                while let Some(message) = conn.handle_read()? {
                    handle_request(conn, &message)?;
                    /*
                        TODO: We can potentially optimize here and handle_write to avoid an extra syscall before
                        reposning to the client. If we're doing request-response, we can assume the client
                        is ready for a response, but if we're not, we'd have to check the client is ready first
                    */
                }
            }
            if fd.revents & POLLOUT != 0 {
                connections.get_mut(&fd.fd).unwrap().handle_write()?;
            }
        }
    }

    // we don't need to close the socket connection since it's tied to the lifetime of `socket`
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (tx, _rx) = mpsc::channel();
    start_server(Some(args.port), Arc::new(AtomicBool::new(true)), tx)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    // helper function to start the server in a separate thread
    fn start_test_server(
        sever_running: Arc<AtomicBool>,
        tx: mpsc::Sender<String>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            start_server(None, sever_running, tx).unwrap();
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
    fn test_single_client() {
        let (server_running, port) = setup();

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let message = b"\x05\x00\x00\x00hello";
        stream.write_all(message).unwrap();

        let mut response = [0; 9];
        stream.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"\x05\x00\x00\x00hello");

        teardown(server_running);
    }

    #[test]
    fn test_single_client_multi_write() {
        let (server_running, port) = setup();

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let message = b"\x05\x00\x00";
        stream.write_all(message).unwrap();
        stream.flush().unwrap();

        let message = b"\x00";
        stream.write_all(message).unwrap();
        stream.flush().unwrap();

        let message = b"hel";
        stream.write_all(message).unwrap();
        stream.flush().unwrap();

        let message = b"lo";
        stream.write_all(message).unwrap();
        stream.flush().unwrap();

        let mut response = [0; 9];
        stream.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"\x05\x00\x00\x00hello");

        teardown(server_running);
    }

    #[test]
    fn test_multiple_clients() {
        let (server_running, port) = setup();

        let mut stream1 = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let message = b"\x05\x00\x00\x00";
        stream1.write_all(message).unwrap();

        let mut stream2 = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let message = b"\x05\x00\x00\x00hello";
        stream2.write_all(message).unwrap();

        let message = b"hello";
        stream1.write_all(message).unwrap();

        let mut response = [0; 9];
        stream2.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"\x05\x00\x00\x00hello");

        let mut response = [0; 9];
        stream1.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"\x05\x00\x00\x00hello");

        teardown(server_running);
    }

    #[test]
    fn test_pipelined_requests() {
        let (server_running, port) = setup();
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();

        teardown(server_running);
    }

    // we don't need to close the socket connection since it's tied to the lifetime of `socket`
}

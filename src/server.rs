use std::net::TcpListener;
use anyhow::Result;
use clap::Parser;
use std::net::IpAddr;
use std::os::fd::{AsRawFd, RawFd};
use libc::{poll, pollfd, POLLIN, POLLERR, POLLOUT};
use std::io;
use std::collections::HashMap;

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
fn main() -> Result<()> {
    let args = Args::parse();
    // define the address and port the server will listen on
    let addr: (IpAddr, u16) = ([127, 0, 0, 1].into(), args.port);

    // create a socket, bind to localhost:PORT, and listen on it 
    let listener = TcpListener::bind(&addr)?;
    listener.set_nonblocking(true)?;
    let fd: RawFd = listener.as_raw_fd();


    let mut connections: HashMap<RawFd, Connection> = HashMap::new();
    loop {
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
                },    
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
                match conn.handle_read()?
                {
                    Some(message) => {
                        handle_request(conn, &message)?;
                    },
                    None => {},
                }
            }
            if fd.revents & POLLOUT != 0 {
                connections.get_mut(&fd.fd).unwrap().handle_write()?;
            }
        }   
    }

    // we don't need to close the socket connection since it's tied to the lifetime of `socket`
}

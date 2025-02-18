use std::net::TcpListener;
use anyhow::Result;
use clap::Parser;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::io::prelude::*;
use std::net::IpAddr;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Port to run the server on
    #[arg(short, long, default_value_t = 3000)]
    port: u16,
}

fn handle_request(mut socket: TcpStream, _addr: SocketAddr) -> Result<()> {
    let mut buf = [0; 64];
    let n = socket.read(&mut buf)?;

    // let request = std::str::from_utf8(&buf[..n])?;
    if &buf[..n] == b"Hello" {
        socket.write_all(b"World")?;
    } 
    Ok(())
}
fn main() -> Result<()> {
    let args = Args::parse();
    // define the address and port the server will listen on
    let addr: (IpAddr, u16) = ([127, 0, 0, 1].into(), args.port);

    // create a soclket, bind to localhost:PORT, and listen on it 
    let listener = TcpListener::bind(&addr)?;
    loop {
        match listener.accept() {
            Ok((socket, addr)) => handle_request(socket, addr),
            Err(e) => Err(e.into()),
        }?

        // we don't need to close the socket connection since it's tied to the lifetime of `socket`
    }
}

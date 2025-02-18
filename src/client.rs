use anyhow::Result;
use clap::Parser;
use std::net::TcpStream;
use std::io::prelude::*;
use std::net::IpAddr;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Port to connect to the server
    #[arg(short, long, default_value_t = 3000)]
    port: u16,
}

fn main() -> Result<()> {
    let args = Args::parse();
    // Define the address and port the server will listen on
    let addr: (IpAddr, u16) = ([127, 0, 0, 1].into(), args.port);

    let mut stream = TcpStream::connect(addr)?;
    stream.write_all(b"Hello")?;

    let mut buffer = [0; 64];
    let n = stream.read(&mut buffer)?;

    let response = std::str::from_utf8(&buffer[..n])?;
    println!("{}", response);

    Ok(())
}

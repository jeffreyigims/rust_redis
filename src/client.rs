use anyhow::Result;
use clap::Parser;
use std::net::IpAddr;

mod connection;
use connection::Connection;

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

    let mut stream: Connection = Connection::new(addr)?;

    stream.set(b"hello", b"world")?;
    let response = stream.read_blocking()?;
    println!("Server said: {}", response);

    stream.get(b"hello")?;
    let response = stream.read_blocking()?;
    println!("Server said: {}", response);

    stream.delete(b"hello")?;
    let response = stream.read_blocking()?;
    println!("Server said: {}", response);

    Ok(())
}

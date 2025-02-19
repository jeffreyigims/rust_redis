use std::net::TcpListener;
use anyhow::Result;
use clap::Parser;
use std::net::IpAddr;
use std::thread;

mod connection;
use connection::Connection;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Port to run the server on
    #[arg(short, long, default_value_t = 3000)]
    port: u16,
}

fn handle_request(mut connection: Connection) -> Result<()> {
    let message = connection.read()?;
    println!("Client said: {}", message);

    connection.write("world!")?;
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
            Ok((socket, _addr)) => {
                thread::spawn(move || { handle_request(Connection::from_stream(socket)) });
            },    
            Err(e) => Err(anyhow::Error::from(e))?,
        }

        // we don't need to close the socket connection since it's tied to the lifetime of `socket`
    }
}

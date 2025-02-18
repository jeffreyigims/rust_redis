use anyhow::Result;
use std::net::{TcpStream, ToSocketAddrs};
use std::io::prelude::*;

const HEADER_SIZE: usize = 4;

pub struct Connection {
    stream: TcpStream,
}

impl Connection {
    pub fn new<A: ToSocketAddrs>(address: A) -> Result<Self> {
        let stream = TcpStream::connect(address)?;
        Ok(Connection { stream })
    }
    pub fn from_stream(stream: TcpStream) -> Self {
        Connection { stream }
    }
    pub fn write(&mut self, message: &str) -> Result<()> {
        let mut buf = [0; 64];

        // we should check query.len() <= 60
        let len: u32 = message.len().try_into().unwrap();
        buf[..HEADER_SIZE as usize].copy_from_slice(len.to_le_bytes().as_ref());
        buf[HEADER_SIZE..HEADER_SIZE+message.len()].copy_from_slice(message.as_bytes());

        self.stream.write_all(&buf)?;
        self.stream.flush()?;
        Ok(())
    }
    pub fn read(&mut self) -> Result<String> {
        let mut response = [0; HEADER_SIZE];
        self.stream.read_exact(&mut response)?;
        let len = u32::from_le_bytes(response);
        println!("Attempting to read {:?} bytes...", len);

        let mut buffer = vec![0; len as usize];
        self.stream.read_exact(&mut buffer)?;
        let response = std::str::from_utf8(&buffer)?;
        Ok(response.to_string())
    }
    pub fn query(&mut self, query: &str) -> Result<String> {
        self.write(query)?;
        self.read()
    }
}
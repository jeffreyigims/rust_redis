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
        let mut bytes_written = 0;
        let len: u32 = message.len().try_into()?;
        let buf = &len.to_le_bytes();
        while bytes_written < HEADER_SIZE {
            bytes_written += self.stream.write(&buf[bytes_written..])?;
        }

        bytes_written = 0;
        let buf = message.as_bytes();
        while bytes_written < message.len() {
            bytes_written += self.stream.write(&buf[bytes_written..])?;
        }
        
        Ok(())
    }
    pub fn read(&mut self) -> Result<String> {
        let mut response = [0; HEADER_SIZE];
        let mut bytes_read = 0;
        while bytes_read < HEADER_SIZE {
            bytes_read += self.stream.read(&mut response[bytes_read..])?;
        }

        let len = u32::from_le_bytes(response);
        println!("Attempting to read {:?} bytes...", len);

        let mut buffer = vec![0; len as usize];
        bytes_read = 0;
        while bytes_read < len as usize{
            bytes_read += self.stream.read(&mut buffer[bytes_read..])?;
        }
        String::from_utf8(buffer).map_err(Into::into)
    }
    pub fn query(&mut self, query: &str) -> Result<String> {
        self.write(query)?;
        self.read()
    }
}
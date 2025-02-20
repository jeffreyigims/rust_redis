use anyhow::Result;
use std::net::{TcpStream, ToSocketAddrs};
use std::io::prelude::*;

const HEADER_SIZE: usize = 4;

pub struct Connection {
    stream: TcpStream,
    
    pub want_to_read: bool,
    pub want_to_write: bool,

    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,
}

impl Connection {
    pub fn new<A: ToSocketAddrs>(address: A) -> Result<Self> {
        let stream = TcpStream::connect(address)?;
        Ok(Connection { stream, want_to_read: true, want_to_write: false, read_buffer: Vec::new(), write_buffer: Vec::new() })
    }
    pub fn from_stream(stream: TcpStream) -> Self {
        Connection { stream, want_to_read: true, want_to_write: false, read_buffer: Vec::new(), write_buffer: Vec::new() }
    }
    pub fn handle_read(&mut self) -> Result<Option<String>> {
       let mut buf = [0; 32 * 1024];
       let bytes_read = self.stream.read(&mut buf)?;

       self.read_buffer.extend_from_slice(&buf[..bytes_read]);

       if self.read_buffer.len() < HEADER_SIZE {
           return Ok(None);
       }

       let message_len = u32::from_le_bytes(self.read_buffer[..HEADER_SIZE].try_into()?);
        println!("Attempting to read {:?} bytes...", message_len);

        let total_len = HEADER_SIZE + message_len as usize;
        if self.read_buffer.len() < total_len {
            return Ok(None);
        }

        let message = String::from_utf8(self.read_buffer[HEADER_SIZE..total_len].to_vec()).map_err(anyhow::Error::from)?;
        Ok(Some(message))

    }
    pub fn handle_write(&mut self) -> Result<()> {
        let bytes_written = self.stream.write(&self.write_buffer)?;
        self.write_buffer.drain(..bytes_written);
        if self.write_buffer.is_empty() {
            self.want_to_write = false;
        }
        Ok(())
    }
    pub fn write(&mut self, message: &str) -> Result<()> {
        self.want_to_read = false;
        let len: u32 = message.len().try_into()?;
        self.write_buffer.extend_from_slice(&len.to_le_bytes());
        self.write_buffer.extend_from_slice(message.as_bytes());
        self.want_to_write = true;
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
        let len: u32 = query.len().try_into()?;
        let mut buffer = Vec::with_capacity(4 + query.len());
        buffer.extend_from_slice(&len.to_le_bytes());
        buffer.extend_from_slice(query.as_bytes());
        self.stream.write_all(&buffer)?;
        self.stream.flush()?;
        self.read()
    }
}
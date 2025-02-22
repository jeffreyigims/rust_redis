use anyhow::anyhow;
use anyhow::Result;
use libc::uint16_t;
use std::io::prelude::*;
use std::net::IpAddr;
use std::net::{TcpStream, ToSocketAddrs};

pub const HEADER_SIZE: usize = 4;

#[derive(Debug)]

pub enum Operation {
    Get { key: String },
    Delete { key: String },
    Set { key: String, value: String },
}

pub struct Connection {
    pub stream: TcpStream, // jjigims23

    pub want_to_read: bool,
    pub want_to_write: bool,
    pub want_close: bool,

    // TODO: implement efficient buffer management
    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,
}

impl Connection {
    pub fn new<A: ToSocketAddrs>(address: A) -> Result<Self> {
        let stream = TcpStream::connect(address)?;
        Ok(Connection {
            stream,
            want_to_read: true,
            want_to_write: false,
            want_close: false,
            read_buffer: Vec::new(),
            write_buffer: Vec::new(),
        })
    }

    pub fn from_port(port: u16) -> Result<Self> {
        let addr: (IpAddr, u16) = ([127, 0, 0, 1].into(), port);
        Self::new(addr)
    }

    pub fn from_stream(stream: TcpStream) -> Self {
        Connection {
            stream,
            want_to_read: true,
            want_to_write: false,
            want_close: false,
            read_buffer: Vec::new(),
            write_buffer: Vec::new(),
        }
    }

    pub fn read(&mut self) -> Result<()> {
        let mut buf = [0; 32 * 1024];
        let result = self.stream.read(&mut buf);
        println!("Read {:?} bytes...", result);
        if let Err(ref e) = result {
            // retryable error
            if e.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(());
            }
        }

        let bytes_read = result?;

        if bytes_read == 0 {
            self.want_close = true;
            return Ok(());
        }

        self.read_buffer.extend_from_slice(&buf[..bytes_read]);

        Ok(())
    }

    fn try_parse(&mut self, index: usize) -> Result<Option<String>> {
        let buf = &self.read_buffer[index..];

        if buf.len() < HEADER_SIZE {
            return Ok(None);
        }

        let message_len = u32::from_le_bytes(buf[..HEADER_SIZE].try_into()?);
        println!("Message length: {:?}", message_len);

        let total_len = HEADER_SIZE + message_len as usize;
        if buf.len() < total_len {
            return Ok(None);
        }

        println!("Total length: {:#?}", buf[HEADER_SIZE..total_len].to_vec());
        let message =
            String::from_utf8(buf[HEADER_SIZE..total_len].to_vec()).map_err(anyhow::Error::from)?;

        println!("Parsed message: {:?}", message);
        Ok(Some(message))
    }

    pub fn handle_read(&mut self) -> Result<Option<Operation>> {
        if self.read_buffer.is_empty() {
            return Ok(None);
        }

        let op = self.read_buffer[0];

        let Some(key) = self.try_parse(1)? else {
            return Ok(None);
        };

        let mut total_len = 1 + HEADER_SIZE + key.len();
        let oper = match op {
            0x01 => Operation::Get { key },
            0x02 => Operation::Delete { key },
            0x03 => {
                let Some(value) = self.try_parse(1 + HEADER_SIZE + key.len())? else {
                    return Ok(None);
                };
                total_len += HEADER_SIZE + value.len();
                Operation::Set { key, value }
            }
            _ => return Err(anyhow!("Invalid operation")),
        };

        self.read_buffer.drain(..total_len);

        Ok(Some(oper))
    }

    pub fn write(&mut self, message: &str) -> Result<()> {
        self.want_to_read = false;
        let len: u32 = message.len().try_into()?;
        self.write_buffer.extend_from_slice(&len.to_le_bytes());
        self.write_buffer.extend_from_slice(message.as_bytes());
        self.want_to_write = true;
        Ok(())
    }

    pub fn handle_write(&mut self) -> Result<()> {
        let result = self.stream.write(&self.write_buffer);
        if let Err(ref e) = result {
            // retryable error
            if e.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(());
            }
        }

        let bytes_written = result?;
        self.write_buffer.drain(..bytes_written);
        if self.write_buffer.is_empty() {
            self.want_to_write = false;
            self.want_to_read = true;
        }
        Ok(())
    }

    pub fn read_blocking(&mut self) -> Result<String> {
        let mut response = [0; HEADER_SIZE];
        let mut bytes_read = 0;
        self.stream.read_exact(&mut response[bytes_read..])?;

        let len = u32::from_le_bytes(response);
        println!("Attempting to read {:?} bytes...", len);

        let mut buffer = vec![0; len as usize];
        self.stream.read_exact(&mut buffer[bytes_read..])?;
        String::from_utf8(buffer).map_err(Into::into)
    }

    pub fn write_blocking(&mut self, message: &str) -> Result<()> {
        let len: u32 = message.len().try_into()?;
        let mut buffer = Vec::with_capacity(4 + message.len());
        buffer.extend_from_slice(&len.to_le_bytes());
        buffer.extend_from_slice(message.as_bytes());
        self.stream.write_all(&buffer)?;
        self.stream.flush().map_err(Into::into)
    }

    pub fn query(&mut self, query: &str) -> Result<String> {
        self.write_blocking(query)?;
        self.read_blocking()
    }

    pub fn get(&mut self, key: &[u8]) -> Result<()> {
        let mut buffer = Vec::new();
        buffer.push(0x01);
        buffer.extend_from_slice(&(key.len() as u32).to_le_bytes());
        buffer.extend_from_slice(key);
        self.stream.write_all(&buffer)?;
        self.stream.flush().map_err(Into::into)
    }

    pub fn set(&mut self, key: &[u8], val: &[u8]) -> Result<()> {
        let mut buffer = Vec::new();
        buffer.push(0x03);
        buffer.extend_from_slice(&(key.len() as u32).to_le_bytes());
        buffer.extend_from_slice(key);
        buffer.extend_from_slice(&(val.len() as u32).to_le_bytes());
        buffer.extend_from_slice(val);
        self.stream.write_all(&buffer)?;
        self.stream.flush().map_err(Into::into)
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<()> {
        let mut buffer = Vec::new();
        buffer.push(0x02);
        buffer.extend_from_slice(&(key.len() as u32).to_le_bytes());
        buffer.extend_from_slice(key);
        self.stream.write_all(&buffer)?;
        self.stream.flush().map_err(Into::into)
    }
}

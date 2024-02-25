use std::io::{self, BufRead, BufReader};

use lazy_static::lazy_static;
use regex::Regex;
use serialport::SerialPort;

#[derive(Debug)]
pub struct Frame {
    pub board_id: u8,
    pub sensor_id: u8,
    pub value: u16,
    pub timestamp: u32,
}

#[derive(Debug)]
pub struct FrameReader {
    port: BufReader<Box<dyn SerialPort>>,
    buf: String,
}

impl FrameReader {
    pub fn new(port: BufReader<Box<dyn SerialPort>>) -> Self {
        Self {
            port,
            buf: String::with_capacity(64),
        }
    }

    pub fn next_frame(&mut self) -> io::Result<Frame> {
        self.port.read_line(&mut self.buf)?;
        let res = self.buf.as_str().try_into();
        self.buf.clear();
        res
    }
}

impl TryFrom<&str> for Frame {
    type Error = io::Error;

    fn try_from(slice: &str) -> Result<Self, Self::Error> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"b=(\d+) s=(\d+) v=(\d+) t=(\d+)\s*").unwrap();
        }

        if let Some(cap) = RE.captures(slice) {
            let board_id: u8 = cap[1].parse().unwrap();
            let sensor_id: u8 = cap[2].parse().unwrap();
            let value: u16 = cap[3].parse().unwrap();
            let timestamp: u32 = cap[4].parse().unwrap();
            Ok(Frame {
                board_id,
                sensor_id,
                value,
                timestamp,
            })
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, slice.to_owned()))
        }
    }
}

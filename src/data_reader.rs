use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader},
    sync::mpsc::{self, TryRecvError},
    thread,
    time::Duration,
};

use lazy_static::lazy_static;
use log::error;
use regex::Regex;
use serialport::SerialPort;

enum Commands {
    Stop,
}

#[derive(Debug)]
struct Frame {
    board_id: u8,
    sensor_id: u8,
    value: u16,
    timestamp: u32,
}

#[derive(Debug, Default)]
pub struct Reader {
    comm: Option<ReaderComm>,
    status: ReaderStatus,
    /// {(board_id, sensor_id) ->  data}
    pub data: HashMap<(u8, u8), Vec<[f64; 2]>>,
}

#[derive(Debug, Default)]
enum ReaderStatus {
    Err(String),
    Running,
    #[default]
    Stopped,
}

#[derive(Debug)]
struct ReaderComm {
    command_tx: mpsc::Sender<Commands>,
    frame_rx: mpsc::Receiver<Frame>,
    status_rx: mpsc::Receiver<ReaderStatus>,
}

#[derive(Debug)]
struct FrameReader {
    port: BufReader<Box<dyn SerialPort>>,
    buf: String,
}

impl Reader {
    /// update internal state, returns duration after which this should be called again
    pub fn process(&mut self) -> Option<Duration> {
        let Some(ref mut r) = self.comm else {
            return None;
        };

        let mut ret = None;

        let e = loop {
            match r.frame_rx.try_recv() {
                Ok(f) => self
                    .data
                    .entry((f.board_id, f.sensor_id))
                    .or_default()
                    .push([f.timestamp as f64, f.value as f64]),
                Err(e) => break e,
            }
        };

        match e {
            TryRecvError::Empty => ret = Some(Duration::from_millis(100)),
            TryRecvError::Disconnected => {
                self.status = ReaderStatus::Err("Reader Disconnected unexpectedly".into())
            }
        };

        match r.status_rx.try_recv() {
            Ok(ReaderStatus::Running) => {
                self.status = ReaderStatus::Running;
                ret
            }
            Ok(s) => {
                self.status = s;
                self.comm = None;
                None
            }
            Err(TryRecvError::Disconnected) => {
                self.status = ReaderStatus::Err("Reader Disconnected unexpectedly".into());
                self.comm = None;
                None
            }
            Err(TryRecvError::Empty) => ret,
        }
    }

    pub fn start_reading(&mut self, path: &str, baud: u32) {
        if self.comm.is_none() {
            match Self::spawn_reader(path, baud) {
                Ok(r) => self.comm = Some(r),
                Err(e) => self.status = ReaderStatus::Err(format!("{e}")),
            }
        }
    }

    pub fn stop_reading(&mut self) {
        let Some(ref mut r) = self.comm else {
            return;
        };
        let _ = r.command_tx.send(Commands::Stop);
    }

    pub fn running(&self) -> bool {
        matches!(self.status, ReaderStatus::Running)
    }

    pub fn reader_status(&self) -> String {
        match self.status {
            ReaderStatus::Err(ref e) => e.to_owned(),
            ReaderStatus::Running => "Running".to_owned(),
            ReaderStatus::Stopped => "Stopped".to_owned(),
        }
    }

    fn spawn_reader(path: &str, baud: u32) -> Result<ReaderComm, io::Error> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(100))
            .open()?;

        let (frame_tx, frame_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let (status_tx, status_rx) = mpsc::channel();

        thread::spawn(|| Self::reader_main(port, frame_tx, status_tx, command_rx));
        Ok(ReaderComm {
            command_tx,
            frame_rx,
            status_rx,
        })
    }

    /// reader main function. Reads frames from the serial port and sends them into `frame_tx`
    fn reader_main(
        port: Box<dyn SerialPort>,
        frame_tx: mpsc::Sender<Frame>,
        status_tx: mpsc::Sender<ReaderStatus>,
        command_rx: mpsc::Receiver<Commands>,
    ) {
        let reader = BufReader::new(port);
        let mut reader = FrameReader::new(reader);
        let mut err_retry = 0u8;
        status_tx
            .send(ReaderStatus::Running)
            .expect("Main Thread dropped status reciver");
        loop {
            match command_rx.try_recv() {
                Ok(Commands::Stop) => {
                    status_tx
                        .send(ReaderStatus::Stopped)
                        .expect("Main Thread dropped status reciver");
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => (),
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("Main Thread dropped command sender")
                }
            }

            match reader.next_frame() {
                Ok(f) => {
                    err_retry = 0;
                    frame_tx.send(f).expect("Main Thread dropped frame reciver");
                }
                Err(e) => {
                    err_retry += 1;
                    error!("{e:?}");
                    if err_retry > 3 {
                        status_tx
                            .send(ReaderStatus::Err(e.to_string()))
                            .expect("Main Thread dropped status reciver");
                        panic!("Too many read errors")
                    }
                }
            };
        }
    }
}

impl FrameReader {
    fn new(port: BufReader<Box<dyn SerialPort>>) -> Self {
        Self {
            port,
            buf: String::with_capacity(64),
        }
    }

    fn next_frame(&mut self) -> io::Result<Frame> {
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

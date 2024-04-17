use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufRead, BufReader},
    path::Path,
    sync::mpsc::{self, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use csv::Writer;
use log::error;
use serialport::SerialPort;

enum Commands {
    Stop,
    StopLogger,
    StartLogging(Box<Path>),
}

#[derive(Debug)]
struct Frame {
    board_id: u8,
    sensor_id: u8,
    value: i16,
    timestamp: u32,
}

#[derive(Debug, Default)]
pub struct Reader {
    comm: Option<ReaderComm>,
    status: ReaderStatus,
    /// {(board_id, sensor_id) ->  data}
    pub data: HashMap<(u8, u8), Vec<[f64; 2]>>,
}

#[derive(Debug)]
enum ReaderStatus {
    LogErr(String),
    Running,
    Logging,
    Stopped(Option<String>),
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

impl Default for ReaderStatus {
    fn default() -> Self {
        ReaderStatus::Stopped(None)
    }
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
                self.status = ReaderStatus::Stopped(Some("Reader Disconnected unexpectedly".into()))
            }
        };

        match r.status_rx.try_recv() {
            Ok(ReaderStatus::Running) => {
                self.status = ReaderStatus::Running;
                ret
            }
            Ok(ReaderStatus::Logging) => {
                self.status = ReaderStatus::Logging;
                ret
            }
            Ok(s @ ReaderStatus::Stopped(_)) => {
                self.status = s;
                self.comm = None;
                None
            }
            Ok(s @ ReaderStatus::LogErr(_)) => {
                self.status = s;
                ret
            }
            Err(TryRecvError::Disconnected) => {
                self.status =
                    ReaderStatus::Stopped(Some("Reader Disconnected unexpectedly".into()));
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
                Err(e) => self.status = ReaderStatus::Stopped(Some(e.to_string())),
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
        !matches!(self.status, ReaderStatus::Stopped(_))
    }

    pub fn start_logging(&mut self, path: Box<Path>) {
        let Some(ref mut r) = self.comm else {
            return;
        };
        let _ = r.command_tx.send(Commands::StartLogging(path));
    }

    pub fn stop_logging(&mut self) {
        let Some(ref mut r) = self.comm else {
            return;
        };
        let _ = r.command_tx.send(Commands::StopLogger);
    }

    pub fn logging(&self) -> bool {
        matches!(self.status, ReaderStatus::Logging)
    }

    pub fn reader_status(&self) -> String {
        match self.status {
            ReaderStatus::LogErr(ref e) => e.to_owned(),
            ReaderStatus::Running => "Running".to_owned(),
            ReaderStatus::Logging => "Logging".to_owned(),
            ReaderStatus::Stopped(Some(ref reason)) => format!("Stopped ({reason})"),
            ReaderStatus::Stopped(_) => "Stopped".to_owned(),
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
        let mut logger: Option<Writer<File>> = None;
        let start = Instant::now();
        status_tx
            .send(ReaderStatus::Running)
            .expect("Main Thread dropped status reciver");
        loop {
            match command_rx.try_recv() {
                Ok(Commands::Stop) => {
                    if let Some(ref mut wtr) = logger {
                        let _ = wtr.flush();
                    }
                    status_tx
                        .send(ReaderStatus::Stopped(None))
                        .expect("Main Thread dropped status reciver");
                    return;
                }
                Ok(Commands::StopLogger) => {
                    let Some(ref mut wtr) = logger else {
                        continue;
                    };
                    if let Err(e) = wtr.flush() {
                        status_tx
                            .send(ReaderStatus::LogErr(e.to_string()))
                            .expect("Main Thread dropped status reciver");
                    } else {
                        status_tx
                            .send(ReaderStatus::Running)
                            .expect("Main Thread dropped status reciver");
                    }
                    logger = None;
                }
                Ok(Commands::StartLogging(path)) => {
                    if logger.is_some() {
                        continue;
                    }
                    let Ok(mut wtr) = csv::Writer::from_path(path).inspect_err(|e| {
                        status_tx
                            .send(ReaderStatus::LogErr(e.to_string()))
                            .expect("Main Thread dropped status reciver")
                    }) else {
                        continue;
                    };
                    if let Err(e) =
                        wtr.write_record(["Sensor id", "Board id", "Read Time", "Time", "Value"])
                    {
                        status_tx
                            .send(ReaderStatus::LogErr(e.to_string()))
                            .expect("Main Thread dropped status reciver");
                        continue;
                    };
                    logger = Some(wtr);
                    status_tx
                        .send(ReaderStatus::Logging)
                        .expect("Main Thread dropped status reciver");
                }
                Err(mpsc::TryRecvError::Empty) => (),
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("Main Thread dropped command sender")
                }
            }

            match reader.next_frame() {
                Ok(f) => {
                    err_retry = 0;
                    if let Some(ref mut wtr) = logger {
                        let now = start.elapsed().as_millis();
                        let slice = [
                            f.sensor_id.to_string(),
                            f.board_id.to_string(),
                            now.to_string(),
                            f.timestamp.to_string(),
                            f.value.to_string(),
                        ];
                        if let Err(e) = wtr.write_record(slice) {
                            status_tx
                                .send(ReaderStatus::LogErr(e.to_string()))
                                .expect("Main Thread dropped status reciver");
                            logger = None;
                        }
                    }
                    frame_tx.send(f).expect("Main Thread dropped frame reciver");
                }
                Err(e) => {
                    err_retry += 1;
                    error!("{e:?}");
                    if err_retry > 3 {
                        status_tx
                            .send(ReaderStatus::Stopped(Some(e.to_string())))
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
        let values: Vec<_> = slice
            .strip_prefix("\r")
            .and_then(|ok|ok.strip_suffix("\n"))
            .ok_or(io::Error::new(io::ErrorKind::InvalidData, slice.to_owned()))?
            .split(" ")
            .filter(|s| !s.is_empty())
            .collect();
        
        if values.len() != 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, slice.to_owned()));
        };

        let sensor_id: u8 = values[0].parse().unwrap();
        let board_id: u8 = values[1].parse().unwrap();
        let value: i16 = values[2].parse().unwrap();
        let timestamp: u32 = values[3].parse().unwrap();
        Ok(Frame {
            board_id,
            sensor_id,
            value,
            timestamp,
        })
    }
}

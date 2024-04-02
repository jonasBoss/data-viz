use core::slice;
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
use itertools::Itertools;
use lazy_static::lazy_static;
use log::error;
use regex::Regex;
use serialport::SerialPort;

enum Commands {
    Stop,
    StopLogger,
    StartLogging(Box<Path>),
}

#[derive(Debug)]
enum SerialData {
    Labels(Vec<String>),
    Values(Vec<i32>),
    Other(String),
}

#[derive(Debug, Default)]
pub struct Reader {
    comm: Option<ReaderComm>,
    status: ReaderStatus,
    labels: Vec<String>,
    /// {(board_id, sensor_id) ->  data}
    pub data: HashMap<String, Vec<[f64; 2]>>,
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
    frame_rx: mpsc::Receiver<SerialData>,
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
                Ok(SerialData::Labels(s))=>{
                    self.labels = s;
                    self.data.clear();
                },
                Ok(SerialData::Values(v))=>{
                    let time = v[0] as f64;
                    for (label, value) in self.labels.iter().zip(v.into_iter().skip(1)){
                        self.data.entry(label.to_owned()).or_default().push([time, value as f64])
                    }
                },
                Ok(SerialData::Other(s)) => {
                    println!("{s}");
                },
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

    pub fn labels(&self)->&[String]{
        self.labels.as_ref()
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
        frame_tx: mpsc::Sender<SerialData>,
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
            let mut waiting = true;
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
                    // if let Some(ref mut wtr) = logger {
                    //     let now = start.elapsed().as_millis();
                    //     let slice = [
                    //         f.sensor_id.to_string(),
                    //         f.board_id.to_string(),
                    //         now.to_string(),
                    //         f.timestamp.to_string(),
                    //         f.value.to_string(),
                    //     ];
                    //     if let Err(e) = wtr.write_record(slice) {
                    //         status_tx
                    //             .send(ReaderStatus::LogErr(e.to_string()))
                    //             .expect("Main Thread dropped status reciver");
                    //         logger = None;
                    //     }
                    // }
                    frame_tx.send(f).expect("Main Thread dropped frame reciver");
                }

                Err(e) => {
                    match e.kind() {
                        io::ErrorKind::TimedOut => {
                            if waiting {continue;}
                        }
                        _ => (),
                    }
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

    fn next_frame(&mut self) -> io::Result<SerialData> {
        self.port.read_line(&mut self.buf)?;
        let res = SerialData::try_from(self.buf.as_str());
        self.buf.clear();
        res
    }
}


impl SerialData{
    fn try_from(slice: &str) -> Result<SerialData, io::Error> {
        dbg!(slice);
        let Some(slice) = slice.strip_suffix("\r\n") else {
            return Ok(Self::Other(slice.to_owned()));
        };
        
        if slice.starts_with("#L "){
            println!("labels:");
            let slice = &slice[3..];
            let this = Self::Labels(slice.split("; ").map(|s|s.to_owned()).collect());
            return Ok(this);
        }

        let mut data: Vec<i32> = Vec::new();
        for substr in slice.split(" ") {
            let Ok(i) =  dbg!(substr).parse() else {
                return Ok(Self::Other(slice.to_owned()));
            };
            data.push(i);
        }
        
        return Ok(Self::Values(data));

    }
}

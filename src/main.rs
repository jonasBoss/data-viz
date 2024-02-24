use std::{
    collections::HashMap,
    io::{self, BufReader},
    sync::mpsc::{self, TryRecvError},
    thread,
    time::Duration,
};

use eframe::egui::{self, Widget};
use egui_plot::{Legend, Line, Plot, PlotPoints};

use itertools::Itertools;

mod data_reader;

use data_reader::{Frame, FrameReader, FrameReaderError};
use log::error;

fn main() -> Result<(), eframe::Error> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 740.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Data Viz",
        options,
        Box::new(move |cc| Box::new(MyApp::new(cc))),
    )
}

enum Commands {
    STOP,
}

struct MyApp {
    path: String,
    baud: u32,
    err: Option<String>,
    sensor_id: u8,
    reader_comm: Option<(mpsc::Receiver<Frame>, mpsc::Sender<Commands>)>,
    /// {sensor_id -> {board_id -> data}}
    data: HashMap<u8, HashMap<u8, Vec<[f64; 2]>>>,
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext) -> Self {
        Self {
            path: "/dev/ttyUSB0".to_owned(),
            baud: 115200,
            err: None,
            sensor_id: 1,
            reader_comm: None,
            data: Default::default(),
        }
    }

    fn spawn_reader(&mut self) -> Result<(), io::Error> {
        self.data.clear();
        let port = serialport::new(self.path.clone(), self.baud)
            .timeout(Duration::from_millis(100))
            .open()?;

        let (frame_tx, frame_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();

        thread::spawn(move || {
            let reader = BufReader::new(port);
            let mut reader = FrameReader::new(reader);

            loop {
                match reader.next_frame() {
                    Ok(f) => match frame_tx.send(f) {
                        Ok(_) => (),
                        Err(e) => {
                            error!("Main Thread has dropped the reciever");
                            panic!("{e:?}")
                        }
                    },
                    Err(e @ FrameReaderError::RawDataError(_)) => error!("{e}"),
                    Err(FrameReaderError::IOError(e)) => match e.kind() {
                        io::ErrorKind::InvalidData => error!("{e}"),
                        io::ErrorKind::TimedOut => error!("{e}"),
                        _ => {
                            error!("{e}");
                            panic!("{e:?}")
                        }
                    },
                }

                match command_rx.try_recv() {
                    Ok(Commands::STOP) => return,
                    Err(TryRecvError::Empty) => (),
                    Err(e @ TryRecvError::Disconnected) => {
                        error!("Main thread dissapeared");
                        panic!("{e:?}")
                    }
                }
            }
        });

        self.reader_comm = Some((frame_rx, command_tx));
        Ok(())
    }

    /// reads data from the mpsc
    ///
    /// **returns** true when data was recived
    fn recive_data(&mut self) -> bool {
        if let Some(ref ch) = self.reader_comm {
            loop {
                match ch.0.try_recv() {
                    Ok(f) => {
                        let v = self
                            .data
                            .entry(f.sensor_id)
                            .or_default()
                            .entry(f.board_id)
                            .or_default();
                        v.push([f.timestamp as f64, f.value as f64]);
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.reader_comm = None;
                        self.err = Some("Reader Disconected".into());
                        return false;
                    }
                    Err(TryRecvError::Empty) => {
                        return true;
                    }
                }
            }
        }
        false
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.recive_data() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::SidePanel::left("side_panel").show(ctx, |ui| {
            ui.heading("Data Viz");
            egui::Grid::new("control_area").show(ui, |ui| {
                let size = [100.0, 0.0].into();
                ui.label("Port:");
                egui::TextEdit::singleline(&mut self.path)
                    .min_size(size)
                    .show(ui);
                ui.end_row();

                ui.label("Baudrate:");
                ui.label(format!("{}", self.baud));
                ui.end_row();

                ui.label("");

                if let Some((_, ref sender)) = self.reader_comm {
                    if egui::Button::new("Stop reading")
                        .min_size(size)
                        .ui(ui)
                        .clicked()
                    {
                        let _ = sender.send(Commands::STOP);
                    }
                } else if egui::Button::new("Start reading")
                    .min_size(size)
                    .ui(ui)
                    .clicked()
                {
                    if let Err(e) = self.spawn_reader() {
                        self.err = Some(format!("{e}"));
                    }
                }

                ui.end_row();
                ui.label("");
                if egui::Button::new("Clear Data")
                    .min_size(size)
                    .ui(ui)
                    .clicked()
                {
                    self.data.clear()
                }
            });
            for id in self.data.keys().sorted() {
                let mut selected = id == &self.sensor_id;
                ui.toggle_value(&mut selected, format!("Show Sensor {id}"));
                if selected {
                    self.sensor_id = *id;
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(format!("Sensor: {}", self.sensor_id));
            if let Some(data) = self.data.get(&self.sensor_id) {
                let plot = Plot::new("sensor_plt").legend(Legend::default());
                plot.show(ui, |plt_ui| {
                    for (board_id, data) in data.iter() {
                        plt_ui.line(
                            Line::new(PlotPoints::from(data.clone()))
                                .name(format!("Board id: {board_id}")),
                        );
                    }
                });
            }
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            self.err.as_ref().map(|err| ui.label(err));
        });
    }
}

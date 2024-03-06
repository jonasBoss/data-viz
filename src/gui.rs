use std::{
    collections::{HashMap, HashSet}, io::{self, BufReader}, path::{Path, PathBuf}, sync::mpsc::{self, Receiver, Sender, TryRecvError}, thread, time::Duration
};

use dirs::home_dir;
use eframe::egui::{self, Ui, Widget};
use egui_file::FileDialog;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use itertools::Itertools;
use log::{error, info};
use serialport::SerialPort;

use crate::data_reader::{Frame, FrameReader};

enum Commands {
    STOP,
}

pub struct MyApp {
    path: String,
    baud: u32,
    err: Option<String>,

    sensors: HashSet<u8>,
    boards: HashSet<u8>,

    reader_comm: Option<(Receiver<io::Result<Frame>>, Sender<Commands>)>,
    /// {(board_id, sensor_id) ->  data}
    data: HashMap<(u8, u8), Vec<[f64; 2]>>,
    file_dialog: FileDialog,
}

/// reader main function. Reads frames from the serial port and sends them into `frame_tx`
fn reader(
    port: Box<dyn SerialPort>,
    frame_tx: Sender<io::Result<Frame>>,
    command_rx: Receiver<Commands>,
) {
    let reader = BufReader::new(port);
    let mut reader = FrameReader::new(reader);

    let mut error = false;
    loop {
        match command_rx.try_recv() {
            Ok(Commands::STOP) => break,
            Err(TryRecvError::Empty) => (),
            Err(e @ TryRecvError::Disconnected) => {
                error!("Main thread dissapeared");
                panic!("{e:?}")
            }
        }

        let msg = reader.next_frame();
        match msg {
            Ok(_) => (),
            Err(ref e) => match e.kind() {
                io::ErrorKind::InvalidData => (),
                io::ErrorKind::TimedOut => (),
                _ => {
                    error = true;
                }
            },
        }
        match frame_tx.send(msg) {
            Ok(_) => (),
            Err(e) => {
                error!("Main Thread has dropped the reciever");
                panic!("{e:?}")
            }
        }
        if error {
            break;
        }
    }
}

impl MyApp {
    pub fn new(_cc: &eframe::CreationContext) -> Self {
        let file_dialog = FileDialog::save_file(home_dir())
            .default_filename("sensor_data.csv")
            .filename_filter(Box::new(|s:&str| s.ends_with(".csv")))
            .show_files_filter(Box::new(|s:&Path| s.ends_with(".csv")));

        Self {
            path: "/dev/ttyUSB0".to_owned(),
            baud: 921_600,
            err: None,
            reader_comm: None,
            data: Default::default(),
            sensors: Default::default(),
            boards: Default::default(),
            file_dialog,
        }
    }

    fn spawn_reader(&mut self) -> Result<(), io::Error> {
        self.data.clear();
        let port = serialport::new(self.path.clone(), self.baud)
            .timeout(Duration::from_millis(100))
            .open()?;

        let (frame_tx, frame_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        thread::spawn(|| reader(port, frame_tx, command_rx));
        self.reader_comm = Some((frame_rx, command_tx));
        Ok(())
    }

    /// reads data from the mpsc
    ///
    /// **returns** the duration after which to redraw
    fn recive_data(&mut self) -> Option<Duration> {
        let ch = self.reader_comm.as_ref()?;

        loop {
            match ch.0.try_recv() {
                Ok(Ok(f)) => {
                    self.data
                        .entry((f.board_id, f.sensor_id))
                        .or_default()
                        .push([f.timestamp as f64, f.value as f64]);
                }

                Ok(Err(e)) => return self.handle_reader_error(e),

                Err(TryRecvError::Disconnected) => {
                    self.reader_comm = None;
                    self.err = Some("Reader Disconected".into());
                    error!("Reader Disconnected");
                    return None;
                }

                Err(TryRecvError::Empty) => {
                    // this is expected. we process frames faster than they arrive
                    return Some(Duration::from_millis(50));
                }
            }
        }
    }

    fn handle_reader_error(&mut self, e: io::Error) -> Option<Duration> {
        use io::ErrorKind::*;
        match e.kind() {
            InvalidData => {
                self.err = Some(format!("{e}"));
                error!("{e:?}");
                // this usually happens when the reading thread just started up,
                // and got the very first (clipped) line. We just try again!
                Some(Duration::from_millis(50))
            }
            TimedOut => {
                self.err = Some(format!("{e}"));
                error!("{e:?}");
                // no need to redraw, but keep listening
                Some(Duration::from_secs(1))
            }
            _ => {
                self.err = Some(format!("Reader encounterd an error: {e}"));
                error!("{e:?}");
                let (_, reader_tx) = self.reader_comm.as_ref().unwrap_or_else(|| unreachable!());
                let _ = reader_tx.send(Commands::STOP);
                None
            }
        }
    }

    fn show_sidebar(&mut self, ui: &mut Ui) {
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
            ui.end_row();

            ui.label("Save to CSV:");
            if egui::Button::new("Save").min_size(size).ui(ui).clicked() {
                self.file_dialog.open();
            }
        });

        ui.label("Boards:");
        for board_id in self.data.keys().map(|(b, _)| b).sorted().dedup() {
            let mut selected = self.boards.contains(board_id);
            ui.toggle_value(&mut selected, format!("Show Board {board_id}"));
            if selected {
                self.boards.insert(*board_id);
            } else {
                self.boards.remove(board_id);
            }
        }
        ui.label("Sensors:");
        for sensor_id in self.data.keys().map(|(_, s)| s).sorted().dedup() {
            let mut selected = self.sensors.contains(sensor_id);
            ui.toggle_value(&mut selected, format!("Show Sensor {sensor_id}"));
            if selected {
                self.sensors.insert(*sensor_id);
            } else {
                self.sensors.remove(sensor_id);
            }
        }
    }

    fn show_plot(&mut self, ui: &mut Ui) {
        let plot = Plot::new("sensor_plt").legend(Legend::default());
        plot.show(ui, |plt_ui| {
            for ((board_id, sensor_id), data) in self
                .data
                .iter()
                .filter(|((b, s), _)| self.boards.contains(b) && self.sensors.contains(s))
            {
                plt_ui.line(
                    Line::new(PlotPoints::from(data.clone()))
                        .name(format!("Senosr: {sensor_id} Board: {board_id}")),
                );
            }
        });
    }

    fn save_data(&self, path:&Path) -> Result<(), io::Error> {
        info!("saving to {path:?}");
        let mut wtr = csv::Writer::from_path(path)?;
        wtr.write_record(&["Sensor id", "Board id", "Time", "Value"])?;
        for slice in self.data.keys().flat_map(|(b, s)| {
            let values = self.data.get(&(*b, *s)).unwrap_or_else(||unreachable!());
            values.iter().map(|[t,v]| [s.to_string(), b.to_string(), t.to_string(), v.to_string()])
        }){
            wtr.write_record(&slice)?;
        }
        wtr.flush()
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.recive_data().is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::SidePanel::left("side_panel").show(ctx, |ui| self.show_sidebar(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.show_plot(ui));
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            self.err.as_ref().map(|err| ui.label(err));
        });

        self.file_dialog.show(ctx);
        if self.file_dialog.selected(){
            if let Some(path) =  self.file_dialog.path(){
                self.save_data(path).unwrap();
            }
        }
    }
}

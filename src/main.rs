use std::{
    collections::HashMap, default, io::{self, BufRead, BufReader}, sync::mpsc::{self, Receiver, TryRecvError}, thread, time::Duration
};

use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use log::{debug, error};
mod data_reader;

use data_reader::{Frame, FrameReader};

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

struct MyApp {
    path: String,
    baud: u32,
    err: Option<String>,
    sensor_id: u8,
    frame_rx: Option<mpsc::Receiver<Frame>>,
    /// {sensor_id -> {board_id -> data}}
    data: HashMap<u8, HashMap<u8, Vec<[f64; 2]>>> ,
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext) -> Self {
        Self {
            path: "/dev/ttyUSB0".to_owned(),
            baud: 115200,
            err: None,
            sensor_id: 1,
            frame_rx: None,
            data:  Default::default(),
        }
    }

    fn spawn_reader(&mut self) -> Result<(), io::Error> {
        self.data.clear();
        let port = serialport::new(self.path.clone(), self.baud)
            .timeout(Duration::from_millis(100))
            .open()?;

        let (frame_tx, frame_rx) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(port);
            let mut reader = FrameReader::new(reader);

            loop {
                let f = reader.next_frame().unwrap();
                frame_tx.send(f).expect("Main Thread has dropped the reciever");
            }
        });

        self.frame_rx = Some(frame_rx);
        Ok(())
    }

    /// reads data from the mpsc
    ///
    /// **returns** true when data was recived
    fn recive_data(&mut self) -> bool {
        if let Some(ref ch) = self.frame_rx {
            loop {
                match ch.try_recv() {
                    Ok(f) => {
                        let v = self.data.entry(f.sensor_id).or_default().entry(f.board_id).or_default();
                        v.push([f.timestamp as f64, f.value as f64]);
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.frame_rx = None;
                        self.err = Some("Reader Disconected".into());
                        return false;
                    }
                    Err(TryRecvError::Empty) => {
                        
                        return true;
                    },
                }
            }
        }
        return false;
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.recive_data(){
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::SidePanel::left("side_panel").show(ctx, |ui| {
            ui.heading("Data Viz");
            egui::Grid::new("control_area").show(ui, |ui|{
                ui.label("Port:");
                egui::TextEdit::singleline(&mut self.path).min_size([100.0, 0.0].into()).show(ui);
                ui.end_row();

                ui.label("Baudrate:");
                ui.label(format!("{}",self.baud));
                ui.end_row();

                ui.label("");
                if ui.button("Start reading").clicked() && self.frame_rx.is_none() {
                    if let Err(e) =  self.spawn_reader(){
                        self.err = Some(format!("{e}"));
                    }
                }
                ui.end_row();
                ui.label("");
                if ui.button("Clear Data").clicked() {
                    self.data.clear()
                }
            });
            for id in self.data.keys(){
                let mut selected = id == &self.sensor_id;
                ui.toggle_value(&mut selected, format!("Show Sensor {id}"));
                if selected {
                    self.sensor_id = *id;
                }
            }
            
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(format!("Sensor: {}", self.sensor_id));
            if let Some(data) = self.data.get(&self.sensor_id){
                let plot = Plot::new("sensor_plt").legend(Legend::default());
                plot.show(ui, |plt_ui| {
                    for (board_id, data) in data.iter() {
                        plt_ui.line(Line::new(PlotPoints::from(data.clone())).name(format!("Board id: {board_id}")));
                    }
                });
            }
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            self.err.as_ref().map(|err| ui.label(err));
        });
    }
}

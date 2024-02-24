use std::{
    default,
    io::{BufRead, BufReader},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
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
    name: String,
    age: u32,
    err: Option<String>,
    sensor_id: u8,
    frame_rx: Option<mpsc::Receiver<Frame>>,
    data: Vec<[f64; 2]>,
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext) -> Self {
        Self {
            name: "Jonas".to_owned(),
            age: 28,
            err: None,
            sensor_id: 1,
            frame_rx: None,
            data: vec![[0.0, 1.0], [2.0, 3.0], [3.0, 2.0]],
        }
    }

    fn spawn_reader(&mut self) {
        let (frame_tx, frame_rx) = mpsc::channel();
        thread::spawn(move || {
            let port = serialport::new("/dev/ttyUSB0", 115200)
                .timeout(Duration::from_millis(100))
                .open()
                .unwrap();
            let reader = BufReader::new(port);
            let mut reader = FrameReader::new(reader);

            loop {
                let f = reader.next_frame().unwrap();
                frame_tx.send(f).unwrap();
            }
        });

        self.frame_rx = Some(frame_rx);
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui::SidePanel::left("side_panel").show(ctx, |ui| {
            ui.heading("Data Viz");
            ui.horizontal(|ui| {
                let name_label = ui.label("Your name: ");
                ui.text_edit_singleline(&mut self.name)
                    .labelled_by(name_label.id);
            });
            ui.add(egui::Slider::new(&mut self.age, 0..=120).text("age"));
            if ui.button("Increment").clicked() {
                self.age += 1;
            }
            ui.label(format!("Hello {}, age {}", self.name, self.age));

            if ui.button("Run").clicked() && self.frame_rx.is_none() {
                self.spawn_reader();
            }

            if let Some(ref ch) = self.frame_rx {
                loop {
                    match ch.try_recv() {
                        Ok(f) => {
                            ui.label(format!("{f:?}"));
                        }
                        Err(TryRecvError::Disconnected) => {
                            self.frame_rx = None;
                            self.err = Some("Reader Disconected".into());
                            break;
                        }
                        Err(TryRecvError::Empty) => break,
                    }
                }
            }

            while let Some(Ok(f)) = self.frame_rx.as_ref().map(Receiver::try_recv) {
                ui.label(format!("{f:?}"));
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let plot = Plot::new("The Plot").legend(Legend::default());

            plot.show(ui, |plt_ui| {
                plt_ui.line(Line::new(PlotPoints::from(self.data.clone())).name("the data"));
            });
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            self.err.as_ref().map(|err| ui.label(err));
        });
    }
}

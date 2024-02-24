use std::{
    default, io::{BufRead, BufReader}, sync::mpsc, thread, time::Duration
};

use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use log::{debug, error};
mod data_reader;

use data_reader::{Frame, FrameReader};


fn main() -> Result<(), eframe::Error> {
    env_logger::init();

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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 740.0]),
        ..Default::default()
    };

    eframe::run_native("Data Viz", options, Box::new(move|cc| Box::new(MyApp::new(cc, frame_rx))))
}

struct MyApp {
    name: String,
    age: u32,
    frame_rx: mpsc::Receiver<Frame>,
    data: Vec<[f64; 2]>,
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext, mpsc: mpsc::Receiver<Frame> ) -> Self {
        Self {
            name: "Jonas".to_owned(),
            age: 28,
            frame_rx: mpsc,
            data: vec![[0.0, 1.0], [2.0, 3.0], [3.0, 2.0]],
        }
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

            while let Ok(f) =  self.frame_rx.try_recv() {
                ui.label(format!("{f:?}"));
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let plot = Plot::new("The Plot").legend(Legend::default());

            plot.show(ui, |plt_ui| {
                plt_ui.line(Line::new(PlotPoints::from(self.data.clone())).name("the data"));
            });
        });
    }
}

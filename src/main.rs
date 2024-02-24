use std::{
    default,
    io::{BufRead, BufReader},
    thread,
    time::Duration,
};

use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use log::{debug, error};

fn main() -> Result<(), eframe::Error> {
    env_logger::init();

    thread::spawn(|| {
        let port = serialport::new("/dev/ttyUSB0", 115200)
            .timeout(Duration::from_millis(100))
            .open()
            .unwrap();

        let mut reader = BufReader::new(port);
        let mut str_buf = String::with_capacity(64);
        loop {
            match reader.read_line(&mut str_buf) {
                Ok(c) => {
                    print!("foo:{c}: {str_buf}");
                    str_buf.clear();
                }
                Err(e) => error!("{:?}", e),
            };
        }
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 740.0]),
        ..Default::default()
    };

    eframe::run_native("Data Viz", options, Box::new(|cc| Box::new(MyApp::new(cc))))
}

struct MyApp {
    name: String,
    age: u32,
    data: Vec<[f64; 2]>,
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext) -> Self {
        Self {
            name: "Jonas".to_owned(),
            age: 28,
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
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let plot = Plot::new("The Plot").legend(Legend::default());

            plot.show(ui, |plt_ui| {
                plt_ui.line(Line::new(PlotPoints::from(self.data.clone())).name("the data"));
            });
        });
    }
}

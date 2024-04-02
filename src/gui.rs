use std::collections::HashSet;

use eframe::egui::{self, Ui, Widget};

use egui_plot::{Legend, Line, Plot, PlotPoints};

use crate::data_reader::Reader;

pub struct MyApp {
    path: String,
    baud: u32,

    labels: HashSet<String>,
    reader: Reader,
}

impl MyApp {
    pub fn new(_cc: &eframe::CreationContext) -> Self {
        Self {
            path: "/dev/ttyUSB0".to_owned(),
            baud: 38400,
            reader: Default::default(),
            labels: Default::default(),
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
            egui::Slider::new(&mut self.baud, 9600..=921_600).ui(ui);
            ui.end_row();

            ui.label("");
            if self.reader.running() {
                if egui::Button::new("Stop reading")
                    .min_size(size)
                    .ui(ui)
                    .clicked()
                {
                    self.reader.stop_reading()
                }
            } else if egui::Button::new("Start reading")
                .min_size(size)
                .ui(ui)
                .clicked()
            {
                self.reader.start_reading(&self.path, self.baud)
            }
            ui.end_row();

            ui.label("");
            if egui::Button::new("Clear Data")
                .min_size(size)
                .ui(ui)
                .clicked()
            {
                self.reader.data.clear()
            }
            ui.end_row();
        });

        ui.separator();
        ui.label("Datenreihen:");
        for label in self.reader.data.keys() {
            let mut selected = self.labels.contains(label);
            ui.toggle_value(&mut selected, label.to_string());
            if selected {
                self.labels.insert(label.to_owned());
            } else {
                self.labels.remove(label);
            }
        }
    }

    fn show_plot(&mut self, ui: &mut Ui) {
        let plot = Plot::new("sensor_plt").legend(Legend::default());
        plot.show(ui, |plt_ui| {
            for (label, data) in self
                .reader
                .data
                .iter()
                .filter(|(l, _)| self.labels.contains(*l))
            {
                plt_ui.line(Line::new(PlotPoints::from(data.clone())).name(label.to_string()));
            }
        });
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(d) = self.reader.process() {
            ctx.request_repaint_after(d);
        }

        egui::SidePanel::left("side_panel").show(ctx, |ui| self.show_sidebar(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.show_plot(ui));
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.label(self.reader.reader_status());
        });
    }
}

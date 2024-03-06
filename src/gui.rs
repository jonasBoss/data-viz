use std::{
    collections::HashSet,
    io::{self},
    path::Path,
};

use dirs::home_dir;
use eframe::egui::{self, Ui, Widget};
use egui_file::FileDialog;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use itertools::Itertools;
use log::info;

use crate::data_reader::Reader;

pub struct MyApp {
    path: String,
    baud: u32,

    sensors: HashSet<u8>,
    boards: HashSet<u8>,

    reader: Reader,
    save_dialog: FileDialog,
    log_dialog: FileDialog,
}

impl MyApp {
    pub fn new(_cc: &eframe::CreationContext) -> Self {
        let save_dialog = FileDialog::save_file(home_dir())
            .default_filename("sensor_data.csv")
            .filename_filter(Box::new(|s: &str| s.ends_with(".csv")))
            .show_files_filter(Box::new(|s: &Path| {
                s.extension().is_some_and(|ext| ext == "csv")
            }));
        let log_dialog = FileDialog::save_file(home_dir())
            .default_filename("sensor_log.csv")
            .filename_filter(Box::new(|s: &str| s.ends_with(".csv")))
            .show_files_filter(Box::new(|s: &Path| {
                s.extension().is_some_and(|ext| ext == "csv")
            }));

        Self {
            path: "/dev/ttyUSB0".to_owned(),
            baud: 921_600,
            reader: Default::default(),
            sensors: Default::default(),
            boards: Default::default(),
            save_dialog,
            log_dialog,
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
            ui.end_row();

            ui.label("Save to CSV:");
            if egui::Button::new("Save").min_size(size).ui(ui).clicked() {
                self.save_dialog.open();
            }
            ui.end_row();

            ui.label("Log Data Live:");
            if egui::Button::new("Start Logger").min_size(size).ui(ui).clicked() {
                self.log_dialog.open();
            }

        });

        ui.separator();
        ui.label("Boards:");
        for board_id in self.reader.data.keys().map(|(b, _)| b).sorted().dedup() {
            let mut selected = self.boards.contains(board_id);
            ui.toggle_value(&mut selected, format!("Show Board {board_id}"));
            if selected {
                self.boards.insert(*board_id);
            } else {
                self.boards.remove(board_id);
            }
        }
        ui.label("Sensors:");
        for sensor_id in self.reader.data.keys().map(|(_, s)| s).sorted().dedup() {
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
                .reader
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

    fn save_data(&self, path: &Path) -> Result<(), io::Error> {
        info!("saving to {path:?}");
        let mut wtr = csv::Writer::from_path(path)?;
        wtr.write_record(["Sensor id", "Board id", "Time", "Value"])?;
        for slice in self.reader.data.keys().flat_map(|(b, s)| {
            let values = self
                .reader
                .data
                .get(&(*b, *s))
                .unwrap_or_else(|| unreachable!());
            values
                .iter()
                .map(|[t, v]| [s.to_string(), b.to_string(), t.to_string(), v.to_string()])
        }) {
            wtr.write_record(&slice)?;
        }
        wtr.flush()
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

        self.save_dialog.show(ctx);
        if self.save_dialog.selected() {
            if let Some(path) = self.save_dialog.path() {
                self.save_data(path).unwrap();
            }
        }
        self.log_dialog.show(ctx);
        if self.log_dialog.selected() {
            if let Some(path) = self.log_dialog.path() {
                self.reader.start_logging(path.into());
            }
        }
    }
}

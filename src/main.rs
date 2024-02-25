use eframe::egui;
use gui::MyApp;

mod data_reader;
mod gui;

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

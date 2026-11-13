use eframe::egui;

fn main() -> eframe::Result {
    run()
}

fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: viewport(),
        ..Default::default()
    };

    eframe::run_native("Counter", options, Box::new(|_context| Ok(App::new_box())))
}

fn viewport() -> egui::ViewportBuilder {
    egui::ViewportBuilder::default().with_inner_size([300.0, 160.0])
}

struct App;
impl App {
    pub fn new_box() -> Box<Self> {
        Box::new(Self)
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _response = egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(format!("Hello world"));
        });
    }
}

use std::time::Duration;

use crate::{app::App, scan_thread::ScannerThread};

const SCAN_INTERVAL: Duration = Duration::from_secs(1);

pub mod app;
mod error;
mod scan_thread;

fn main() {
    println!("startup...");
    let result = eframe::run_native(
        "Soltop",
        options(),
        Box::new(|context| {
            let ctx = context.egui_ctx.clone();
            let scanner = ScannerThread::spawn(SCAN_INTERVAL, move || ctx.request_repaint());
            Ok(App::new_box(scanner))
        }),
    );

    if let Err(err) = result {
        eprintln!("{err}");
        return;
    }
}

fn options() -> eframe::NativeOptions {
    eframe::NativeOptions::default()
}

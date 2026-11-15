use std::{thread, time::Duration};

use crate::scan_thread::ScannerThread;
use eframe::egui::{self, Vec2, vec2};

const SCAN_INTERVAL: Duration = Duration::from_secs(2);
const VIEW_PORT_INNER_SIZE: Vec2 = vec2(300.0, 160.0);

mod error;
mod scan_thread;

fn main() {
    let mut ctx = egui::Context::default();
    let result = eframe::run_native(
        "Counter",
        options(),
        Box::new(|context| {
            ctx = context.egui_ctx.clone();
            Ok(App::new_box())
        }),
    );

    if let Err(err) = result {
        eprintln!("{err}");
        return;
    }

    println!("starting scannerThread...");
    let scanner = ScannerThread::spawn(SCAN_INTERVAL, move || ctx.request_repaint());
    let recieve_interval = SCAN_INTERVAL.div_f32(2.0);
    loop {
        match scanner.recieve() {
            Ok(None) => (),
            Ok(Some(_)) => println!("message"),
            Err(err) => eprintln!("{err}"),
        }
        thread::sleep(recieve_interval);
    }
}

fn options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(VIEW_PORT_INNER_SIZE),
        ..Default::default()
    }
}

struct App {
    tasks: Vec<Task>,
    new_task: String,
}

#[derive(Debug, Clone)]
struct Task {
    title: String,
    done: bool,
}

impl App {
    pub fn new_box() -> Box<Self> {
        Box::new(Self::new())
    }
    pub fn new() -> Self {
        Self {
            tasks: vec![],
            new_task: String::new(),
        }
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.new_task);
                if ui.button("Add").clicked() {
                    if !self.new_task.is_empty() {
                        self.tasks.push(Task {
                            title: std::mem::take(&mut self.new_task),
                            done: false,
                        });
                    }
                }
            });

            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for i in 0..self.tasks.len() {
                    ui.horizontal(|ui| {
                        let task = &mut self.tasks[i];
                        ui.checkbox(&mut task.done, &task.title);
                        if ui.small_button("x").clicked() {
                            self.tasks.remove(i);
                        }
                    });
                }
            })
        });
    }
}

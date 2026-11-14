use eframe::egui::{self, Vec2, vec2};
use sysinfo::{Components, System};

use crate::hardware_info::Hardwarescanner;

mod hardware_info;

const VIEW_PORT_INNER_SIZE: Vec2 = vec2(300.0, 160.0);

fn main() -> eframe::Result {
    run()
}

fn run() -> eframe::Result {
    // eframe::run_native(
    //     "Counter",
    //     options(),
    //     Box::new(|_context| Ok(App::new_box())),
    // )

    // let mut system = System::new_all().unwrap();
    // system.refresh_all();

    // for cpu in system.cpus() {
    //     println!("Core {}: {}%", cpu.name(), cpu.usage())
    // }

    // println!(
    //     "RAM: {} / {} bytes",
    //     bytes_to_gigabytes(system.used_memory()),
    //     bytes_to_gigabytes(system.total_memory())
    // );

    // let components = Components::new_with_refreshed_list().unwrap();
    // for c in &components {
    //     if let Some(temp) = c.temperature() {
    //         println!("{}: {:.1}°C", c.label(), temp);
    //     }
    // }

    let mut scanner = Hardwarescanner::new().unwrap();
    let mut gpu_error = None;
    let info = scanner.initinalize(&mut gpu_error);
    println!("{:#?}\n\n", info.processes());
    println!(
        "{:#?}\n\n",
        info.processes().iter().find(|p| p.gpu_memory.is_some())
    );
    println!("{:#?}\n\n", info.errors());

    Ok(())
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
        Box::new(Self {
            tasks: vec![],
            new_task: String::new(),
        })
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
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

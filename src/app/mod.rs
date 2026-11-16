use std::{collections::VecDeque, fmt::Display};

use eframe::egui;
use hardware_scanner::{
    HardwareInfo,
    model::{AsF64, Bytes, Gb, Percent},
};

use crate::{
    app::processes::ProcessView,
    scan_thread::{Message, ScannerThread},
};

mod processes;

const CPU_COLOR: egui::Color32 = egui::Color32::from_rgb(17, 125, 187);
const RAM_COLOR: egui::Color32 = egui::Color32::from_rgb(139, 18, 174);

/// Large title on the left, current value on the right (Task Manager style).
fn section_header(ui: &mut egui::Ui, title: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.heading(value);
        });
    });
}

#[derive(PartialEq, Clone, Copy, Default)]
enum Tab {
    #[default]
    Processes,
    Preformance,
}

pub struct App {
    tab: Tab,
    scanner: ScannerThread,
    fatal: Option<Box<str>>,
    process_view: ProcessView,
    gpu_note: Option<Box<str>>,
    info: Option<HardwareInfo>,
    cpu_history: History<Percent>,
    ram_history: History<Gb>,
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("tabs")
            .resizable(false)
            .exact_size(140.0)
            .show(ui, |ui| self.menu_ui(ui));
        self.notices_ui(ui);
        match self.tab {
            Tab::Processes => self.processes_ui(ui),
            Tab::Preformance => self.performace_ui(ui),
        }

        self.recieve_scan();
    }
}
impl App {
    pub fn new_box(scanner: ScannerThread) -> Box<Self> {
        Box::new(Self::new(scanner))
    }
    pub fn new(scanner: ScannerThread) -> Self {
        Self {
            scanner,
            info: None,
            fatal: None,
            gpu_note: None,
            tab: Tab::default(),
            ram_history: History::new(1000, 0.0),
            cpu_history: History::new(1000, 100.0),
            process_view: ProcessView::new(),
        }
    }

    fn menu_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Processes, "Processes");
            ui.selectable_value(&mut self.tab, Tab::Preformance, "Performance");
        });

        if let Some(info) = &self.info {
            ui.add_space(12.0);
            ui.separator();
            if let Some(cpu) = info.cpu().total_usage {
                ui.small(format!("CPU  {:.0}%", cpu.as_f32()));
            }
            if let (Some(used), Some(total)) = (info.ram().used, info.ram().total) {
                let ratio = used.as_u64() as f32 / total.as_u64().max(1) as f32;
                ui.small(format!("RAM  {:.0}%", ratio * 100.0));
            }
        }
    }

    fn notices_ui(&self, ui: &mut egui::Ui) {
        if let Some(fatal) = &self.fatal {
            ui.colored_label(ui.visuals().error_fg_color, format!("Error: {fatal}"));
        }
        if let Some(note) = &self.gpu_note {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("GPU unavailable: {note}"),
            );
        }
    }

    fn processes_ui(&mut self, ui: &mut egui::Ui) {
        let Some(info) = &mut self.info else {
            ui.spinner();
            ui.label("Scanning... ");
            return;
        };

        self.process_view.ui(info, ui);
    }

    fn load_first_info(&mut self, info: &HardwareInfo) {
        let ram = info.ram().total.unwrap_or_default().to_gb(5);
        self.ram_history.set_max(ram.as_f64());
    }

    fn load_new_info(&mut self, mut info: HardwareInfo) {
        self.process_view.sort_process(&mut info);
        if let Some(usage) = info.cpu().total_usage {
            self.cpu_history.push(usage);
        }
        if let Some(used) = info.ram().used {
            self.ram_history.push(used.to_gb(5));
        }

        self.info = Some(info)
    }

    fn performace_ui(&mut self, ui: &mut egui::Ui) {
        let Some(info) = &self.info else {
            ui.spinner();
            ui.label("Scanning... ");
            return;
        };

        let cpu = info.cpu();
        let cpu_total = match cpu.total_usage {
            None => "???%".to_string(),
            Some(usage) => format!("{:.0}%", usage.as_f32()),
        };
        section_header(ui, "CPU", &cpu_total);
        self.cpu_history.plot(ui, "plot_cpu", "CPU", CPU_COLOR);

        ui.add_space(6.0);
        egui::Grid::new("cores")
            .num_columns(4)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for (i, core) in cpu.core_usage.iter().enumerate() {
                    let bar = egui::ProgressBar::new(core.as_f32() / 100.0)
                        .fill(CPU_COLOR)
                        .show_percentage();
                    ui.add(bar);
                    if (i + 1) % 4 == 0 {
                        ui.end_row();
                    }
                }
            });

        ui.add_space(16.0);
        let ram = info.ram();
        let ram_text = match (ram.used, ram.total) {
            (Some(used), Some(total)) => format!(
                "{:.1} / {:.1} GB",
                used.to_gb(1).as_f32(),
                total.to_gb(1).as_f32()
            ),
            _ => "???".to_string(),
        };
        section_header(ui, "Memory", &ram_text);
        self.ram_history.plot(ui, "plot_ram", "RAM", RAM_COLOR);
    }

    fn recieve_scan(&mut self) {
        loop {
            let message = match self.scanner.recieve() {
                Ok(Some(val)) => val,
                Ok(None) => break,
                Err(err) => {
                    self.set_fatal(err);
                    break;
                }
            };

            match message {
                Message::First(info) => {
                    self.load_first_info(&info);
                    self.load_new_info(info);
                }
                Message::Info(info) => {
                    self.load_new_info(info);
                }
                Message::StartupError(err) => self.set_fatal(err),
                Message::GpuUnavailble(err) => self.set_gpu_note(err),
            }
        }
    }

    fn set_gpu_note(&mut self, err: impl std::error::Error) {
        self.gpu_note = Some(box_str(err))
    }

    fn set_fatal(&mut self, err: impl std::error::Error) {
        self.fatal = Some(box_str(err))
    }
}

struct History<T: AsF64> {
    max: f64,
    limit: usize,
    queue: VecDeque<T>,
}
impl<T: AsF64> History<T> {
    fn new(limit: usize, max: f64) -> Self {
        Self {
            max,
            limit,
            queue: VecDeque::with_capacity(limit),
        }
    }

    fn set_max(&mut self, max: f64) {
        self.max = max
    }

    fn push(&mut self, value: T) {
        if self.queue.len() >= self.limit {
            _ = self.queue.pop_front();
        }

        self.queue.push_back(value);
    }

    fn plot(&self, ui: &mut egui::Ui, label: &str, line_name: &str, color: egui::Color32) {
        let points: Vec<[f64; 2]> = self
            .queue
            .iter()
            .enumerate()
            .map(|(i, v)| [i as f64, v.as_f64()])
            .collect();

        egui_plot::Plot::new(label)
            .height(160.0)
            .include_y(0.0)
            .include_y(self.max)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .show_axes([false, false])
            .show_x(false)
            .show_y(false)
            .show(ui, |plot_ui| {
                plot_ui.line(
                    egui_plot::Line::new(line_name, points)
                        .color(color)
                        .width(1.5)
                        .fill(0.0)
                        .fill_alpha(0.25),
                );
            });
    }
}

fn box_str(err: impl Display) -> Box<str> {
    err.to_string().into_boxed_str()
}

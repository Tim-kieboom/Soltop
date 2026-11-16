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

        ui.strong("CPU");
        let cpu = info.cpu();
        let total = match cpu.total_usage {
            None => format!("total: ???%"),
            Some(usage) => format!("total: {:.0}%", usage.as_f32()),
        };

        ui.label(total);
        egui::Grid::new("cores").num_columns(4).show(ui, |ui| {
            for (i, core) in cpu.core_usage.iter().enumerate() {
                let bar = egui::ProgressBar::new(core.as_f32() / 100.0).show_percentage();
                ui.add(bar);
                if (i + 1) % 4 == 0 {
                    ui.end_row();
                }
            }
        });

        self.cpu_history.plot(ui, "plot_cpu", "CPU");
        self.ram_history.plot(ui, "plot_ram", "RAM");
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

    fn plot(&self, ui: &mut egui::Ui, label: &str, line_name: &str) {
        let points: Vec<[f64; 2]> = self
            .queue
            .iter()
            .enumerate()
            .map(|(i, v)| [i as f64, v.as_f64()])
            .collect();

        egui_plot::Plot::new(label)
            .height(120.0)
            .include_y(0.0)
            .include_y(self.max)
            .allow_drag(false)
            .allow_zoom(false)
            .show(ui, |plot_ui| {
                plot_ui.line(egui_plot::Line::new(line_name, points));
            });
    }
}

fn box_str(err: impl Display) -> Box<str> {
    err.to_string().into_boxed_str()
}

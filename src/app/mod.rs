use std::fmt::Display;

use eframe::egui;
use hardware_scanner::HardwareInfo;

use crate::scan_thread::{Message, ScannerThread};

mod tabs;

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
    process_view: tabs::ProcessView,
    gpu_note: Option<Box<str>>,
    info: Option<HardwareInfo>,
    performance_view: tabs::PerformanceView,
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
            Tab::Preformance => self.performance_ui(ui),
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
            process_view: tabs::ProcessView::new(),
            performance_view: tabs::PerformanceView::new(),
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

    fn performance_ui(&mut self, ui: &mut egui::Ui) {
        let Some(info) = &self.info else {
            ui.spinner();
            ui.label("Scanning... ");
            return;
        };

        self.performance_view.ui(info, ui);
    }

    fn load_new_info(&mut self, mut info: HardwareInfo) {
        self.process_view.sort_process(&mut info);
        self.performance_view.push(&info);
        self.info = Some(info)
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
                    self.performance_view.load_first_info(&info);
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

fn box_str(err: impl Display) -> Box<str> {
    err.to_string().into_boxed_str()
}

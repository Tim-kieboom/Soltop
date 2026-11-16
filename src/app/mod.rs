//! The window: a sidebar with the tabs, and whichever tab is selected.

use std::fmt::Display;

use eframe::egui;
use hardware_scanner::HardwareInfo;

use crate::scan_thread::{Message, ScannerThread};

mod tabs;

/// The tabs in the sidebar.
#[derive(PartialEq, Clone, Copy, Default)]
enum Tab {
    #[default]
    Processes,
    Preformance,
    System,
}

/// The application state: the scan thread's receiver, the latest scan, and one view per tab.
pub struct App {
    tab: Tab,
    scanner: ScannerThread,
    fatal: Option<Box<str>>,
    process_view: tabs::ProcessView,
    gpu_note: Option<Box<str>>,
    info: Option<HardwareInfo>,
    performance_view: tabs::PerformanceView,
    system_view: tabs::SystemView,
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
            Tab::System => self.system_ui(ui),
        }

        self.recieve_scan();
    }
}
impl App {
    /// For eframe, which wants the app boxed.
    pub fn new_box(scanner: ScannerThread) -> Box<Self> {
        Box::new(Self::new(scanner))
    }
    /// Starts on the Processes tab with no data; the first scan arrives a moment later.
    pub fn new(scanner: ScannerThread) -> Self {
        Self {
            scanner,
            info: None,
            fatal: None,
            gpu_note: None,
            tab: Tab::default(),
            process_view: tabs::ProcessView::new(),
            performance_view: tabs::PerformanceView::new(),
            system_view: tabs::SystemView::new(),
        }
    }

    /// The sidebar: tab buttons and a live CPU and RAM readout.
    fn menu_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Processes, "Processes");
            ui.selectable_value(&mut self.tab, Tab::Preformance, "Performance");
            ui.selectable_value(&mut self.tab, Tab::System, "System");
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

    /// Shows a fatal scanner error in red and the "no GPU" note in amber above the tab.
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

    /// The Processes tab, or a spinner until the first scan.
    fn processes_ui(&mut self, ui: &mut egui::Ui) {
        let Some(info) = &mut self.info else {
            ui.spinner();
            ui.label("Scanning... ");
            return;
        };

        self.process_view.ui(info, ui);
    }

    /// The System tab, or a spinner until the first scan.
    fn system_ui(&mut self, ui: &mut egui::Ui) {
        let Some(info) = &self.info else {
            ui.spinner();
            ui.label("Scanning... ");
            return;
        };

        self.system_view.ui(info, ui);
    }

    /// The Performance tab, or a spinner until the first scan.
    fn performance_ui(&mut self, ui: &mut egui::Ui) {
        let Some(info) = &self.info else {
            ui.spinner();
            ui.label("Scanning... ");
            return;
        };

        self.performance_view.ui(info, ui);
    }

    /// Sorts the new scan, records it in the graphs and makes it the current one.
    fn load_new_info(&mut self, mut info: HardwareInfo) {
        self.process_view.sort_process(&mut info);
        self.performance_view.push(&info);
        self.info = Some(info)
    }

    /// Reads every message the scan thread has queued. Called once per frame.
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
                Message::First { info, static_info } => {
                    self.system_view.load_first_info(static_info.system.clone());
                    self.performance_view.load_first_info(&info, static_info);
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

/// An error (or anything displayable) as a boxed string, to keep in the app state.
fn box_str(err: impl Display) -> Box<str> {
    err.to_string().into_boxed_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_str_keeps_the_message() {
        assert_eq!(&*box_str("scanner failed"), "scanner failed");
        assert_eq!(&*box_str(42), "42");
    }

    #[test]
    fn the_app_opens_on_the_processes_tab() {
        assert!(Tab::default() == Tab::Processes);
    }
}

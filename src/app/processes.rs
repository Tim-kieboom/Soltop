use std::cmp::Ordering;

use eframe::egui;
use egui_extras::{Column, TableBody, TableBuilder, TableRow};
use hardware_scanner::{
    HardwareInfo,
    model::{Bytes, Percent},
};

#[derive(Default, PartialEq, Clone, Copy)]
enum SortKey {
    #[default]
    Cpu,
    Pid,
    Ram,
    Gpu,
    Name,
}

pub struct ProcessView {
    key_sort: SortKey,
    descending: bool,
}
impl ProcessView {
    pub fn new() -> Self {
        Self {
            descending: true,
            key_sort: SortKey::default(),
        }
    }

    pub fn sort_process(&mut self, info: &mut HardwareInfo) {
        let descending = self.descending;
        match self.key_sort {
            SortKey::Pid => info
                .processes_mut()
                .sort_by(|a, b| cmp(&a.pid, &b.pid, descending)),
            SortKey::Cpu => info
                .processes_mut()
                .sort_by(|a, b| partial_cmp(&a.cpu_usage, &b.cpu_usage, descending)),
            SortKey::Ram => info
                .processes_mut()
                .sort_by(|a, b| cmp(&a.memory, &b.memory, descending)),
            SortKey::Gpu => info
                .processes_mut()
                .sort_by(|a, b| partial_cmp(&a.cpu_usage, &b.cpu_usage, descending)),
            SortKey::Name => info
                .processes_mut()
                .sort_by(|a, b| cmp(&a.file_name(), &b.file_name(), descending)),
        }
    }

    pub fn ui(&mut self, info: &mut HardwareInfo, ui: &mut egui::Ui) {
        TableBuilder::new(ui)
            .striped(true)
            .column(Column::auto())
            .column(Column::remainder())
            .column(Column::auto())
            .column(Column::auto())
            .column(Column::auto())
            .header(18.0, |header| self.header_ui(info, header))
            .body(|body| self.body_ui(info, body));
    }

    fn header_ui(&mut self, info: &mut HardwareInfo, mut header: TableRow<'_, '_>) {
        header.col(|ui| {
            self.header_col_ui(ui, info, String::new(), "PID", SortKey::Pid);
        });
        header.col(|ui| {
            self.header_col_ui(ui, info, String::new(), "Name", SortKey::Name);
        });
        header.col(|ui| {
            let cpu = format!("{:.2}%", total_cpu(info));
            self.header_col_ui(ui, info, cpu, "CPU", SortKey::Cpu);
        });
        header.col(|ui| {
            let ram = format!("{:.2} gb", total_ram(info));
            self.header_col_ui(ui, info, ram, "RAM", SortKey::Ram);
        });
        header.col(|ui| {
            let gpu = format!("{:.2}%", total_gpu(info));
            self.header_col_ui(ui, info, gpu, "GPU", SortKey::Gpu);
        });
    }

    fn body_ui(&mut self, info: &mut HardwareInfo, body: TableBody<'_>) {
        let processes = info.processes_mut();
        body.rows(18.0, processes.len(), |mut row| {
            let process = &processes[row.index()];
            row.col(|ui| {
                ui.label(process.pid.to_string());
            });
            row.col(|ui| {
                ui.label(process.name.to_string_lossy());
            });
            row.col(|ui| {
                ui.label(format!("{:.2}%", process.cpu_usage.as_f32()));
            });
            row.col(|ui| {
                ui.label(format!("{:.2} gb", process.memory.to_gb(2).as_f32()));
            });
            row.col(|ui| {
                let label = match process.gpu_memory {
                    Some(memory) => {
                        format!("{:.2} gb", memory.to_gb(2).as_f32())
                    }
                    _ => format!("__"),
                };
                ui.label(label);
            });
        });
    }

    fn header_col_ui(
        &mut self,
        ui: &mut egui::Ui,
        info: &mut HardwareInfo,
        value: String,
        label: &str,
        key: SortKey,
    ) {
        ui.vertical(|ui| {
            self.header_button_ui(ui, info, label, key);
            ui.small(value);
        });
    }

    fn header_button_ui(
        &mut self,
        ui: &mut egui::Ui,
        info: &mut HardwareInfo,
        label: &str,
        key: SortKey,
    ) {
        let arrow = if self.key_sort == key {
            if self.descending { " v" } else { " ^" }
        } else {
            ""
        };

        if ui.button(format!("{label}{arrow}")).clicked() {
            self.set_sort(key);
            self.sort_process(info);
        }
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.key_sort == key {
            self.descending = !self.descending;
        } else {
            self.key_sort = key;
            self.descending = true;
        }
    }
}

fn total_cpu(info: &HardwareInfo) -> f32 {
    info.cpu().total_usage.unwrap_or_default().as_f32()
}

fn total_ram(info: &HardwareInfo) -> f32 {
    info.ram().total.unwrap_or_default().to_gb(2).as_f32()
}

fn total_gpu(info: &HardwareInfo) -> f32 {
    let sum = info
        .gpus()
        .iter()
        .flat_map(|gpu| gpu.as_ref().map(|gpu| gpu.memory_usage).flatten())
        .map(|usage| usage.as_f32())
        .sum::<f32>();

    sum / info.gpus().len() as f32
}

fn cmp<T: Ord>(a: &T, b: &T, descending: bool) -> Ordering {
    let ord = a.cmp(b);
    if descending { ord.reverse() } else { ord }
}

fn partial_cmp<T: PartialOrd>(a: &T, b: &T, descending: bool) -> Ordering {
    let ord = a.partial_cmp(b).unwrap_or(Ordering::Equal);
    if descending { ord.reverse() } else { ord }
}

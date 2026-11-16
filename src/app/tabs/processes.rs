use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    ffi::OsStr,
    path::{Path, PathBuf},
};

use eframe::egui::{self, Align, Color32, Layout};
use egui_extras::{Column, TableBody, TableBuilder, TableRow};
use hardware_scanner::{
    HardwareInfo,
    model::{Bytes, Process},
};

#[derive(Default, PartialEq, Clone, Copy)]
enum SortKey {
    #[default]
    Cpu,
    Pid,
    Ram,
    Gpu,
    Disk,
    User,
    Name,
}

pub struct ProcessView {
    key_sort: SortKey,
    descending: bool,
    /// Executable paths whose group is currently expanded.
    expanded: HashSet<PathBuf>,
}

/// All processes that share one executable path.
struct Group<'a> {
    path: &'a Path,
    /// Indices into the process list, in the list's current sort order.
    indices: Vec<usize>,
    first_pid: u32,
    /// The shared owner, or `None` when the group's processes have different owners.
    user: Option<&'a str>,
    stats: Stats,
}

/// The numbers shown for one row, whether it is a process or a whole group.
#[derive(Clone, Copy)]
struct Stats {
    cpu: f32,
    memory: u64,
    /// Read plus write, bytes per second.
    disk: u64,
    /// Highest GPU load (percentages are not additive).
    gpu: Option<f32>,
}
impl Group<'_> {
    fn name(&self) -> &OsStr {
        self.path.file_name().unwrap_or(self.path.as_os_str())
    }
}

#[derive(Clone, Copy)]
enum RowKind {
    Group(usize),
    Single(usize),
    Child(usize),
}
impl ProcessView {
    pub fn new() -> Self {
        Self {
            descending: true,
            key_sort: SortKey::default(),
            expanded: HashSet::new(),
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
                .sort_by(|a, b| partial_cmp(&a.gpu_usage, &b.gpu_usage, descending)),
            SortKey::Disk => info
                .processes_mut()
                .sort_by(|a, b| cmp(&a.disk_rate(), &b.disk_rate(), descending)),
            SortKey::User => info
                .processes_mut()
                .sort_by(|a, b| cmp(&a.user, &b.user, descending)),
            SortKey::Name => info
                .processes_mut()
                .sort_by(|a, b| cmp(&a.file_name(), &b.file_name(), descending)),
        }
    }

    pub fn ui(&mut self, info: &mut HardwareInfo, ui: &mut egui::Ui) {
        TableBuilder::new(ui)
            .striped(true)
            .column(Column::auto().at_least(60.0))
            .column(Column::remainder().clip(true))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(80.0))
            .header(34.0, |header| self.header_ui(info, header))
            .body(|body| self.body_ui(info, body));
    }

    fn header_ui(&mut self, info: &mut HardwareInfo, mut header: TableRow<'_, '_>) {
        header.col(|ui| {
            self.header_col_ui(ui, info, String::new(), "PID", SortKey::Pid, Align::Min);
        });
        header.col(|ui| {
            self.header_col_ui(ui, info, String::new(), "Name", SortKey::Name, Align::Min);
        });
        header.col(|ui| {
            self.header_col_ui(ui, info, String::new(), "User", SortKey::User, Align::Min);
        });
        header.col(|ui| {
            let cpu = format!("{:.2}%", total_cpu(info));
            self.header_col_ui(ui, info, cpu, "CPU", SortKey::Cpu, Align::Max);
        });
        header.col(|ui| {
            let ram = ram_in_use(info);
            self.header_col_ui(ui, info, ram, "RAM", SortKey::Ram, Align::Max);
        });
        header.col(|ui| {
            let disk = rate_text(info.processes().iter().map(Process::disk_rate).sum());
            self.header_col_ui(ui, info, disk, "Disk", SortKey::Disk, Align::Max);
        });
        header.col(|ui| {
            let gpu = match total_gpu(info) {
                Some(usage) => format!("{usage:.0}%"),
                None => "__".to_string(),
            };
            self.header_col_ui(ui, info, gpu, "GPU", SortKey::Gpu, Align::Max);
        });
    }

    fn body_ui(&mut self, info: &HardwareInfo, body: TableBody<'_>) {
        let ram_total = info.ram().total.unwrap_or_default().as_u64().max(1) as f32;
        let processes = info.processes();

        let mut groups = build_groups(processes);
        self.sort_groups(&mut groups);
        let rows = self.flatten(&groups);

        body.rows(24.0, rows.len(), |mut row| match rows[row.index()] {
            RowKind::Single(i) => {
                let process = &processes[i];
                row.col(|ui| pid_cell(ui, process));
                row.col(|ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        name_label(ui, process);
                    });
                });
                row.col(|ui| user_cell(ui, process.user.as_deref()));
                stat_cells(&mut row, process_stats(process), ram_total);
            }
            RowKind::Child(i) => {
                let process = &processes[i];
                row.col(|ui| pid_cell(ui, process));
                row.col(|ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add_space(20.0);
                        name_label(ui, process);
                    });
                });
                row.col(|ui| user_cell(ui, process.user.as_deref()));
                stat_cells(&mut row, process_stats(process), ram_total);
            }
            RowKind::Group(g) => {
                let group = &groups[g];
                let open = self.expanded.contains(group.path);
                row.col(|_| {});
                row.col(|ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let arrow = if open { "\u{25BC}" } else { "\u{25B6}" };
                        let text = format!(
                            "{arrow} {} ({})",
                            group.name().to_string_lossy(),
                            group.indices.len()
                        );
                        let label = egui::Label::new(text)
                            .truncate()
                            .sense(egui::Sense::click());
                        let response = ui
                            .add(label)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(group.path.to_string_lossy());
                        if response.clicked() && !self.expanded.remove(group.path) {
                            self.expanded.insert(group.path.to_path_buf());
                        }
                    });
                });
                row.col(|ui| user_cell(ui, group.user.or(Some("multiple"))));
                stat_cells(&mut row, group.stats, ram_total);
            }
        });
    }

    fn sort_groups(&self, groups: &mut [Group<'_>]) {
        let d = self.descending;
        match self.key_sort {
            SortKey::Pid => groups.sort_by(|a, b| cmp(&a.first_pid, &b.first_pid, d)),
            SortKey::Cpu => groups.sort_by(|a, b| partial_cmp(&a.stats.cpu, &b.stats.cpu, d)),
            SortKey::Ram => groups.sort_by(|a, b| cmp(&a.stats.memory, &b.stats.memory, d)),
            SortKey::Gpu => groups.sort_by(|a, b| partial_cmp(&a.stats.gpu, &b.stats.gpu, d)),
            SortKey::Disk => groups.sort_by(|a, b| cmp(&a.stats.disk, &b.stats.disk, d)),
            SortKey::User => groups.sort_by(|a, b| cmp(&a.user, &b.user, d)),
            SortKey::Name => groups.sort_by(|a, b| cmp(&a.name(), &b.name(), d)),
        }
    }

    /// Groups of one process are plain rows; larger groups get a header row,
    /// followed by their children when expanded.
    fn flatten(&self, groups: &[Group<'_>]) -> Vec<RowKind> {
        let mut rows = Vec::new();
        for (g, group) in groups.iter().enumerate() {
            if group.indices.len() == 1 {
                rows.push(RowKind::Single(group.indices[0]));
                continue;
            }
            rows.push(RowKind::Group(g));
            if self.expanded.contains(group.path) {
                rows.extend(group.indices.iter().map(|&i| RowKind::Child(i)));
            }
        }
        rows
    }

    fn header_col_ui(
        &mut self,
        ui: &mut egui::Ui,
        info: &mut HardwareInfo,
        value: String,
        label: &str,
        key: SortKey,
        align: Align,
    ) {
        ui.with_layout(Layout::top_down(align), |ui| {
            self.header_button_ui(ui, info, label, key);
            ui.strong(value);
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

        let button = egui::Button::new(format!("{label}{arrow}")).frame(false);
        if ui.add(button).clicked() {
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

fn build_groups(processes: &[Process]) -> Vec<Group<'_>> {
    let mut groups: Vec<Group<'_>> = Vec::new();
    let mut by_path: HashMap<&Path, usize> = HashMap::new();

    for (i, process) in processes.iter().enumerate() {
        let path: &Path = &process.name;
        let stats = process_stats(process);

        match by_path.get(path) {
            Some(&g) => {
                let group = &mut groups[g];
                group.indices.push(i);
                group.first_pid = group.first_pid.min(process.pid.as_u32());
                if group.user != process.user.as_deref() {
                    group.user = None;
                }
                group.stats.cpu += stats.cpu;
                group.stats.memory += stats.memory;
                group.stats.disk += stats.disk;
                group.stats.gpu = match (group.stats.gpu, stats.gpu) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
            }
            None => {
                by_path.insert(path, groups.len());
                groups.push(Group {
                    path,
                    indices: vec![i],
                    first_pid: process.pid.as_u32(),
                    user: process.user.as_deref(),
                    stats,
                });
            }
        }
    }
    groups
}

fn process_stats(process: &Process) -> Stats {
    Stats {
        cpu: process.cpu_usage.as_f32(),
        memory: process.memory.as_u64(),
        disk: process.disk_rate(),
        gpu: process.gpu_usage.map(|usage| usage.as_f32()),
    }
}

fn pid_cell(ui: &mut egui::Ui, process: &Process) {
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.label(process.pid.to_string());
    });
}

/// The file name, with a hover card showing the path, command line and process details.
fn name_label(ui: &mut egui::Ui, process: &Process) {
    ui.add(egui::Label::new(process.file_name().to_string_lossy()).truncate())
        .on_hover_ui(|ui| process_tooltip(ui, process));
}

fn process_tooltip(ui: &mut egui::Ui, process: &Process) {
    ui.set_max_width(520.0);
    ui.strong(process.name.to_string_lossy());
    if !process.command.is_empty() {
        ui.label(&*process.command);
    }
    ui.separator();

    let parent = process
        .parent
        .map_or("none".to_string(), |parent| parent.to_string());
    let threads = process
        .threads
        .map_or("???".to_string(), |threads| threads.to_string());
    let rows = [
        ("PID", process.pid.to_string()),
        ("Parent PID", parent),
        ("Status", process.state.label().to_string()),
        ("Threads", threads),
        ("Running for", duration_text(process.run_time)),
        ("Disk read", rate_text(process.disk_read.as_u64())),
        ("Disk write", rate_text(process.disk_write.as_u64())),
    ];
    egui::Grid::new("process_tooltip")
        .num_columns(2)
        .spacing([16.0, 2.0])
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.weak(label);
                ui.label(value);
                ui.end_row();
            }
        });
}

fn user_cell(ui: &mut egui::Ui, user: Option<&str>) {
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add(egui::Label::new(user.unwrap_or("__")).truncate());
    });
}

fn stat_cells(row: &mut TableRow<'_, '_>, stats: Stats, ram_total: f32) {
    row.col(|ui| heat_cell(ui, stats.cpu / 100.0, format!("{:.1}%", stats.cpu)));
    row.col(|ui| {
        let gb = Bytes::new(stats.memory).to_gb(2).as_f32();
        heat_cell(ui, stats.memory as f32 / ram_total * 4.0, format!("{gb:.2} GB"));
    });
    row.col(|ui| {
        // 100 MB/s is a fully tinted cell.
        heat_cell(ui, stats.disk as f32 / 100e6, rate_text(stats.disk));
    });
    row.col(|ui| match stats.gpu {
        Some(usage) => heat_cell(ui, usage / 100.0, format!("{usage:.0}%")),
        None => heat_cell(ui, 0.0, "__".to_string()),
    });
}

/// Right-aligned cell with a Task-Manager-style amber tint that grows with `ratio` (0..=1).
fn heat_cell(ui: &mut egui::Ui, ratio: f32, text: String) {
    let alpha = (ratio.clamp(0.0, 1.0).sqrt() * 120.0) as u8;
    if alpha > 0 {
        let color = Color32::from_rgba_unmultiplied(255, 170, 0, alpha);
        ui.painter().rect_filled(ui.max_rect(), 0.0, color);
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.label(text);
    });
}

/// Bytes per second as "12.3 MB/s".
fn rate_text(bytes_per_second: u64) -> String {
    let bytes = bytes_per_second as f64;
    if bytes >= 1e9 {
        format!("{:.1} GB/s", bytes / 1e9)
    } else if bytes >= 1e6 {
        format!("{:.1} MB/s", bytes / 1e6)
    } else if bytes >= 1e3 {
        format!("{:.0} KB/s", bytes / 1e3)
    } else {
        format!("{bytes:.0} B/s")
    }
}

/// "2d 03:04:05" for long-running processes, "03:04:05" otherwise.
fn duration_text(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    let (days, hours, minutes, seconds) =
        (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60, secs % 60);
    if days > 0 {
        format!("{days}d {hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}

fn total_cpu(info: &HardwareInfo) -> f32 {
    info.cpu().total_usage.unwrap_or_default().as_f32()
}

/// Memory currently in use, as "12.8 GB (10%)".
fn ram_in_use(info: &HardwareInfo) -> String {
    let ram = info.ram();
    let (Some(used), Some(total)) = (ram.used, ram.total) else {
        return "__".to_string();
    };

    let percent = used.as_u64() as f32 / total.as_u64().max(1) as f32 * 100.0;
    format!("{:.1} GB ({percent:.0}%)", used.to_gb(1).as_f32())
}

/// Average GPU load across the GPUs that reported one; `None` when no GPU did.
fn total_gpu(info: &HardwareInfo) -> Option<f32> {
    let loads: Vec<f32> = info
        .gpus()
        .iter()
        .flatten()
        .filter_map(|gpu| gpu.gpu)
        .map(|usage| usage.as_f32())
        .collect();

    if loads.is_empty() {
        None
    } else {
        Some(loads.iter().sum::<f32>() / loads.len() as f32)
    }
}

fn cmp<T: Ord>(a: &T, b: &T, descending: bool) -> Ordering {
    let ord = a.cmp(b);
    if descending { ord.reverse() } else { ord }
}

fn partial_cmp<T: PartialOrd>(a: &T, b: &T, descending: bool) -> Ordering {
    let ord = a.partial_cmp(b).unwrap_or(Ordering::Equal);
    if descending { ord.reverse() } else { ord }
}

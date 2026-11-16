use std::collections::VecDeque;

use eframe::egui::{self, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke};
use hardware_scanner::{
    HardwareInfo,
    model::{AsF64, Gb, GpuInfo, Percent},
};

const CPU_COLOR: Color32 = Color32::from_rgb(17, 125, 187);
const RAM_COLOR: Color32 = Color32::from_rgb(139, 18, 174);
const GPU_COLOR: Color32 = Color32::from_rgb(60, 160, 90);

/// How many of the newest samples the mini graphs in the device list show.
const MINI_POINTS: usize = 60;

#[derive(Clone, Copy, PartialEq, Default)]
enum Device {
    #[default]
    Cpu,
    Memory,
    Gpu(usize),
}

struct GpuHistory {
    load: History<Percent>,
    memory: History<Percent>,
}
impl GpuHistory {
    fn new() -> Self {
        Self {
            load: History::new(1000, 100.0),
            memory: History::new(1000, 100.0),
        }
    }
}

/// What one entry of the device list shows.
struct Card {
    title: String,
    value: String,
    color: Color32,
    values: Vec<f64>,
    max: f64,
}

pub struct PerformanceView {
    selected: Device,
    cpu_history: History<Percent>,
    ram_history: History<Gb>,
    gpu_histories: Vec<GpuHistory>,
}
impl PerformanceView {
    pub fn new() -> Self {
        Self {
            selected: Device::default(),
            ram_history: History::new(1000, 0.0),
            cpu_history: History::new(1000, 100.0),
            gpu_histories: Vec::new(),
        }
    }

    /// Called once with the first scan: the RAM graph's upper bound is the machine's total RAM.
    pub fn load_first_info(&mut self, info: &HardwareInfo) {
        let ram = info.ram().total.unwrap_or_default().to_gb(5);
        self.ram_history.set_max(ram.as_f64());
    }

    /// Records one scan into the history graphs.
    pub fn push(&mut self, info: &HardwareInfo) {
        if let Some(usage) = info.cpu().total_usage {
            self.cpu_history.push(usage);
        }
        if let Some(used) = info.ram().used {
            self.ram_history.push(used.to_gb(5));
        }

        while self.gpu_histories.len() < info.gpus().len() {
            self.gpu_histories.push(GpuHistory::new());
        }
        for (history, gpu) in self.gpu_histories.iter_mut().zip(info.gpus()) {
            let Some(gpu) = gpu else { continue };
            if let Some(load) = gpu.gpu {
                history.load.push(load);
            }
            if let Some(memory) = gpu.memory_usage {
                history.memory.push(memory);
            }
        }
    }

    pub fn ui(&mut self, info: &HardwareInfo, ui: &mut egui::Ui) {
        egui::Panel::left("perf_devices")
            .resizable(false)
            .exact_size(230.0)
            .show(ui, |ui| self.list_ui(info, ui));

        egui::ScrollArea::vertical().show(ui, |ui| match self.selected_device(info) {
            Device::Cpu => self.cpu_ui(info, ui),
            Device::Memory => self.ram_ui(info, ui),
            Device::Gpu(i) => self.gpu_ui(info, i, ui),
        });
    }

    /// The selected device, falling back to the CPU if its GPU disappeared.
    fn selected_device(&self, info: &HardwareInfo) -> Device {
        match self.selected {
            Device::Gpu(i) if !matches!(info.gpus().get(i), Some(Some(_))) => Device::Cpu,
            device => device,
        }
    }

    fn list_ui(&mut self, info: &HardwareInfo, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let selected = self.selected_device(info);

        let mut devices = vec![Device::Cpu, Device::Memory];
        devices.extend(
            info.gpus()
                .iter()
                .enumerate()
                .filter(|(_, gpu)| gpu.is_some())
                .map(|(i, _)| Device::Gpu(i)),
        );

        for device in devices {
            let card = self.card(info, device);
            if device_card(ui, &card, device == selected) {
                self.selected = device;
            }
        }
    }

    fn card(&self, info: &HardwareInfo, device: Device) -> Card {
        match device {
            Device::Cpu => Card {
                title: "CPU".into(),
                value: percent_text(info.cpu().total_usage),
                color: CPU_COLOR,
                values: self.cpu_history.recent(MINI_POINTS),
                max: self.cpu_history.max,
            },
            Device::Memory => {
                let ram = info.ram();
                let value = match (ram.used, ram.total) {
                    (Some(used), Some(total)) => format!(
                        "{:.1}/{:.1} GB ({:.0}%)",
                        used.to_gb(1).as_f32(),
                        total.to_gb(1).as_f32(),
                        used.as_u64() as f32 / total.as_u64().max(1) as f32 * 100.0
                    ),
                    _ => "???".into(),
                };
                Card {
                    title: "Memory".into(),
                    value,
                    color: RAM_COLOR,
                    values: self.ram_history.recent(MINI_POINTS),
                    max: self.ram_history.max,
                }
            }
            Device::Gpu(i) => {
                let gpu = info.gpus().get(i).and_then(|gpu| gpu.as_ref());
                let mut value = percent_text(gpu.and_then(|gpu| gpu.gpu));
                if let Some(temp) = gpu.and_then(|gpu| gpu.temperature) {
                    value.push_str(&format!("  {:.0}\u{b0}C", temp.as_f32()));
                }
                Card {
                    title: format!("GPU {i}"),
                    value,
                    color: GPU_COLOR,
                    values: self
                        .gpu_histories
                        .get(i)
                        .map(|history| history.load.recent(MINI_POINTS))
                        .unwrap_or_default(),
                    max: 100.0,
                }
            }
        }
    }

    fn cpu_ui(&self, info: &HardwareInfo, ui: &mut egui::Ui) {
        let cpu = info.cpu();
        section_header(ui, "CPU", &percent_text(cpu.total_usage));
        self.cpu_history.plot(ui, "plot_cpu", "CPU", CPU_COLOR);

        ui.add_space(8.0);
        stats_ui(
            ui,
            &[
                ("Utilization", percent_text(cpu.total_usage)),
                ("Cores", cpu.core_usage.len().to_string()),
                (
                    "Temperature",
                    cpu.temperature
                        .map_or("???".into(), |t| format!("{:.0}\u{b0}C", t.as_f32())),
                ),
            ],
        );

        ui.add_space(12.0);
        ui.strong("Per core");
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
    }

    fn ram_ui(&self, info: &HardwareInfo, ui: &mut egui::Ui) {
        let ram = info.ram();
        let used = ram.used.unwrap_or_default();
        let total = ram.total.unwrap_or_default();
        let available = total.as_u64().saturating_sub(used.as_u64());
        let gb = |bytes: u64| {
            format!(
                "{:.1} GB",
                hardware_scanner::model::Bytes::new(bytes).to_gb(1).as_f32()
            )
        };

        section_header(ui, "Memory", &gb(total.as_u64()));
        self.ram_history.plot(ui, "plot_ram", "RAM", RAM_COLOR);

        ui.add_space(8.0);
        stats_ui(
            ui,
            &[
                ("In use", gb(used.as_u64())),
                ("Available", gb(available)),
                ("Total", gb(total.as_u64())),
                (
                    "Temperature",
                    ram.temperature
                        .map_or("???".into(), |t| format!("{:.0}\u{b0}C", t.as_f32())),
                ),
            ],
        );
    }

    fn gpu_ui(&self, info: &HardwareInfo, index: usize, ui: &mut egui::Ui) {
        let Some(Some(gpu)) = info.gpus().get(index) else {
            return;
        };
        let Some(history) = self.gpu_histories.get(index) else {
            return;
        };

        section_header(ui, &format!("GPU {index}"), &percent_text(gpu.gpu));
        history
            .load
            .plot(ui, format!("plot_gpu_{index}"), "GPU", GPU_COLOR);

        ui.add_space(8.0);
        ui.strong("Memory usage");
        history
            .memory
            .plot(ui, format!("plot_gpu_mem_{index}"), "VRAM", GPU_COLOR);

        ui.add_space(8.0);
        stats_ui(ui, &gpu_stats(gpu));
    }
}

fn gpu_stats(gpu: &GpuInfo) -> Vec<(&'static str, String)> {
    let total = gpu.memory_total;
    let used_gb = match (gpu.memory_usage, total) {
        (Some(usage), Some(total)) => Some(usage.as_f32() / 100.0 * total.to_gb(2).as_f32()),
        _ => None,
    };

    vec![
        ("Utilization", percent_text(gpu.gpu)),
        (
            "Memory used",
            used_gb.map_or("???".into(), |gb| format!("{gb:.1} GB")),
        ),
        (
            "Memory total",
            total.map_or("???".into(), |t| format!("{:.1} GB", t.to_gb(1).as_f32())),
        ),
        (
            "Temperature",
            gpu.temperature
                .map_or("???".into(), |t| format!("{:.0}\u{b0}C", t.as_f32())),
        ),
    ]
}

fn percent_text(usage: Option<Percent>) -> String {
    usage.map_or("???%".into(), |usage| format!("{:.0}%", usage.as_f32()))
}

/// Large title on the left, current value on the right (Task Manager style).
fn section_header(ui: &mut egui::Ui, title: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.heading(value);
        });
    });
}

/// A row of "small label over big value" tiles.
fn stats_ui(ui: &mut egui::Ui, stats: &[(&str, String)]) {
    ui.horizontal_wrapped(|ui| {
        for (label, value) in stats {
            ui.vertical(|ui| {
                ui.small(*label);
                ui.heading(value);
            });
            ui.add_space(24.0);
        }
    });
}

/// One clickable entry in the device list: mini graph on the left, title and value on the right.
/// Returns true when it was clicked.
fn device_card(ui: &mut egui::Ui, card: &Card, selected: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 62.0), Sense::click());

    let visuals = ui.visuals();
    let background = if selected {
        visuals.selection.bg_fill.gamma_multiply(0.6)
    } else if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let title_color = visuals.strong_text_color();
    let value_color = visuals.weak_text_color();

    let painter = ui.painter();
    painter.rect_filled(rect.shrink(1.0), 4.0, background);

    let graph = Rect::from_min_size(
        rect.min + egui::vec2(8.0, 8.0),
        egui::vec2(80.0, rect.height() - 16.0),
    );
    paint_sparkline(painter, graph, &card.values, card.max, card.color);

    let text_x = graph.right() + 10.0;
    painter.text(
        Pos2::new(text_x, rect.top() + 10.0),
        Align2::LEFT_TOP,
        &card.title,
        FontId::proportional(14.0),
        title_color,
    );
    painter.text(
        Pos2::new(text_x, rect.top() + 32.0),
        Align2::LEFT_TOP,
        &card.value,
        FontId::proportional(12.0),
        value_color,
    );

    response.clicked()
}

/// Draws a small filled line graph into `rect`, newest sample on the right.
fn paint_sparkline(painter: &egui::Painter, rect: Rect, values: &[f64], max: f64, color: Color32) {
    painter.rect_filled(rect, 0.0, color.gamma_multiply(0.08));
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, color.gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    if values.len() < 2 || max <= 0.0 {
        return;
    }

    let step = rect.width() / (MINI_POINTS - 1) as f32;
    let x_start = rect.right() - step * (values.len() - 1) as f32;
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let ratio = (value / max).clamp(0.0, 1.0) as f32;
            Pos2::new(
                x_start + step * i as f32,
                rect.bottom() - ratio * rect.height(),
            )
        })
        .collect();

    let fill = color.gamma_multiply(0.25);
    let mut mesh = Mesh::default();
    for pair in points.windows(2) {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pair[0], fill);
        mesh.colored_vertex(pair[1], fill);
        mesh.colored_vertex(Pos2::new(pair[1].x, rect.bottom()), fill);
        mesh.colored_vertex(Pos2::new(pair[0].x, rect.bottom()), fill);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::line(points, Stroke::new(1.0, color)));
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

    /// The newest `count` samples, oldest first.
    fn recent(&self, count: usize) -> Vec<f64> {
        let skip = self.queue.len().saturating_sub(count);
        self.queue.iter().skip(skip).map(|v| v.as_f64()).collect()
    }

    fn plot(
        &self,
        ui: &mut egui::Ui,
        id: impl std::hash::Hash + std::fmt::Debug,
        line_name: &str,
        color: Color32,
    ) {
        let points: Vec<[f64; 2]> = self
            .queue
            .iter()
            .enumerate()
            .map(|(i, v)| [i as f64, v.as_f64()])
            .collect();

        egui_plot::Plot::new(egui::Id::unique(id))
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

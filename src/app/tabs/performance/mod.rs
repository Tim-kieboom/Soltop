use eframe::egui::{self, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke};
use hardware_scanner::{
    HardwareInfo,
    model::{AsF64, Bytes, DiskKind, Gb, Mhz, NetKind, Percent, StaticInfo, Watts},
};

use crate::app::tabs::performance::history::History;

mod history;

const CPU_COLOR: Color32 = Color32::from_rgb(17, 125, 187);
const RAM_COLOR: Color32 = Color32::from_rgb(139, 18, 174);
const GPU_COLOR: Color32 = Color32::from_rgb(60, 160, 90);
const DISK_COLOR: Color32 = Color32::from_rgb(214, 148, 0);
const NET_COLOR: Color32 = Color32::from_rgb(30, 160, 160);
/// Unused part of a volume's usage bar.
const VOLUME_EMPTY_COLOR: Color32 = Color32::from_gray(58);

/// How many of the newest samples the mini graphs in the device list show.
const MINI_POINTS: usize = 60;

const CORE_TILE_MIN_WIDTH: f32 = 130.0;
const CORE_TILE_HEIGHT: f32 = 56.0;
const CORE_TILE_SPACING: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Default)]
enum Device {
    #[default]
    Cpu,
    Memory,
    Disk(usize),
    Net(usize),
    Gpu(usize),
}

struct DiskHistory {
    active: History<Percent>,
    read: History<Bytes>,
    write: History<Bytes>,
}
impl DiskHistory {
    fn new() -> Self {
        Self {
            active: History::new(1000, 100.0),
            // A max of 0 lets the transfer graphs scale to the data.
            read: History::new(1000, 0.0),
            write: History::new(1000, 0.0),
        }
    }
}

struct NetHistory {
    receive: History<Bytes>,
    send: History<Bytes>,
}
impl NetHistory {
    fn new() -> Self {
        Self {
            // A max of 0 lets the graphs scale to the data.
            receive: History::new(1000, 0.0),
            send: History::new(1000, 0.0),
        }
    }
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
    static_info: StaticInfo,
    cpu_history: History<Percent>,
    core_histories: Vec<History<Percent>>,
    ram_history: History<Gb>,
    gpu_histories: Vec<GpuHistory>,
    disk_histories: Vec<DiskHistory>,
    net_histories: Vec<NetHistory>,
}
impl PerformanceView {
    pub fn new() -> Self {
        Self {
            selected: Device::default(),
            static_info: StaticInfo::default(),
            core_histories: Vec::new(),
            ram_history: History::new(1000, 0.0),
            cpu_history: History::new(1000, 100.0),
            gpu_histories: Vec::new(),
            disk_histories: Vec::new(),
            net_histories: Vec::new(),
        }
    }

    /// Called once with the first scan: the RAM graph's upper bound is the machine's total RAM,
    /// and the static hardware facts are kept for the detail views.
    pub fn load_first_info(&mut self, info: &HardwareInfo, static_info: StaticInfo) {
        let ram = info.ram().total.unwrap_or_default().to_gb(5);
        self.ram_history.set_max(ram.as_f64());
        self.static_info = static_info;
    }

    /// Records one scan into the history graphs.
    pub fn push(&mut self, info: &HardwareInfo) {
        if let Some(usage) = info.cpu().total_usage {
            self.cpu_history.push(usage);
        }
        if let Some(used) = info.ram().used {
            self.ram_history.push(used.to_gb(5));
        }

        let cores = &info.cpu().core_usage;
        while self.core_histories.len() < cores.len() {
            self.core_histories.push(History::new(1000, 100.0));
        }
        for (history, usage) in self.core_histories.iter_mut().zip(cores) {
            history.push(*usage);
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

        while self.disk_histories.len() < info.disks().len() {
            self.disk_histories.push(DiskHistory::new());
        }
        for (history, disk) in self.disk_histories.iter_mut().zip(info.disks()) {
            if let Some(active) = disk.active_time {
                history.active.push(active);
            }
            if let Some(read) = disk.read_rate {
                history.read.push(read);
            }
            if let Some(write) = disk.write_rate {
                history.write.push(write);
            }
        }

        while self.net_histories.len() < info.nets().len() {
            self.net_histories.push(NetHistory::new());
        }
        for (history, net) in self.net_histories.iter_mut().zip(info.nets()) {
            if let Some(receive) = net.receive_rate {
                history.receive.push(receive);
            }
            if let Some(send) = net.send_rate {
                history.send.push(send);
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
            Device::Disk(i) => self.disk_ui(info, i, ui),
            Device::Net(i) => self.net_ui(info, i, ui),
            Device::Gpu(i) => self.gpu_ui(info, i, ui),
        });
    }

    /// The selected device, falling back to the CPU if its GPU disappeared.
    fn selected_device(&self, info: &HardwareInfo) -> Device {
        match self.selected {
            Device::Gpu(i) if !matches!(info.gpus().get(i), Some(Some(_))) => Device::Cpu,
            Device::Disk(i) if i >= info.disks().len() => Device::Cpu,
            Device::Net(i) if i >= info.nets().len() => Device::Cpu,
            device => device,
        }
    }

    fn list_ui(&mut self, info: &HardwareInfo, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let selected = self.selected_device(info);

        let mut devices = vec![Device::Cpu, Device::Memory];
        devices.extend((0..info.disks().len()).map(Device::Disk));
        devices.extend((0..info.nets().len()).map(Device::Net));
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
                max: self.cpu_history.max(),
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
                    max: self.ram_history.max(),
                }
            }
            Device::Disk(i) => {
                let disk = info.disks().get(i);
                let kind = self.disk_static(info, i).map_or("", |facts| kind_text(facts.kind));
                let active = percent_text(disk.and_then(|disk| disk.active_time));
                Card {
                    title: format!("Disk {i}"),
                    value: format!("{kind}  {active}").trim().to_string(),
                    color: DISK_COLOR,
                    values: self
                        .disk_histories
                        .get(i)
                        .map(|history| history.active.recent(MINI_POINTS))
                        .unwrap_or_default(),
                    max: 100.0,
                }
            }
            Device::Net(i) => {
                let net = info.nets().get(i);
                let history = self.net_histories.get(i);
                // One graph for both directions, scaled to its own peak (at least 100 KB/s so
                // idle noise doesn't fill the box).
                let values: Vec<f64> = history.map_or(Vec::new(), |history| {
                    let receive = history.receive.recent(MINI_POINTS);
                    let send = history.send.recent(MINI_POINTS);
                    receive.iter().zip(&send).map(|(r, s)| r + s).collect()
                });
                let max = values.iter().copied().fold(100_000.0, f64::max);
                let rate = |rate: Option<Bytes>| rate.map_or("?".into(), rate_short);
                Card {
                    title: self.net_title(info, i),
                    value: format!(
                        "R {}  S {}",
                        rate(net.and_then(|net| net.receive_rate)),
                        rate(net.and_then(|net| net.send_rate))
                    ),
                    color: NET_COLOR,
                    values,
                    max,
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
        let facts = &self.static_info.cpu;
        let name: &str = if facts.name.is_empty() { "CPU" } else { &facts.name };
        section_header(ui, "CPU", name);
        self.cpu_history.plot(ui, "plot_cpu", "CPU", CPU_COLOR);

        ui.add_space(8.0);
        stats_ui(
            ui,
            &[
                ("Utilization", percent_text(cpu.total_usage)),
                ("Speed", cpu.frequency.map_or("???".into(), ghz_text)),
                ("Processes", info.processes().len().to_string()),
                ("Up time", cpu.uptime.map_or("???".into(), uptime_text)),
                (
                    "Temperature",
                    cpu.temperature
                        .map_or("???".into(), |t| format!("{:.0}\u{b0}C", t.as_f32())),
                ),
            ],
        );

        ui.add_space(4.0);
        stats_ui(
            ui,
            &[
                ("Base speed", facts.base_clock.map_or("???".into(), ghz_text)),
                ("Max speed", facts.max_clock.map_or("???".into(), ghz_text)),
                (
                    "Cores",
                    facts.physical_cores.map_or("???".into(), |n| n.to_string()),
                ),
                ("Logical processors", facts.logical_cores.to_string()),
            ],
        );

        ui.add_space(4.0);
        stats_ui(
            ui,
            &[
                ("L1 data cache", cache_text(facts.cache.l1_data)),
                ("L1 instruction cache", cache_text(facts.cache.l1_instruction)),
                ("L2 cache", cache_text(facts.cache.l2)),
                ("L3 cache", cache_text(facts.cache.l3)),
            ],
        );

        ui.add_space(12.0);
        ui.strong("Logical processors");
        ui.add_space(4.0);
        self.cores_ui(&cpu.core_usage, ui);
    }

    /// One small live graph per core; the column count adapts to the available width.
    fn cores_ui(&self, cores: &[Percent], ui: &mut egui::Ui) {
        let spacing = CORE_TILE_SPACING;
        let available = ui.available_width();
        let columns = (((available + spacing) / (CORE_TILE_MIN_WIDTH + spacing)).floor() as usize)
            .max(1);
        let width = (available - spacing * (columns - 1) as f32) / columns as f32;

        for (row, chunk) in cores.chunks(columns).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = spacing;
                for (offset, usage) in chunk.iter().enumerate() {
                    let index = row * columns + offset;
                    let values = self
                        .core_histories
                        .get(index)
                        .map(|history| history.recent(MINI_POINTS))
                        .unwrap_or_default();
                    core_tile(ui, width, index, *usage, &values);
                }
            });
        }
    }

    fn ram_ui(&self, info: &HardwareInfo, ui: &mut egui::Ui) {
        let ram = info.ram();
        let used = ram.used.unwrap_or_default();
        let total = ram.total.unwrap_or_default();
        let available = ram
            .available
            .unwrap_or_else(|| Bytes::new(total.as_u64().saturating_sub(used.as_u64())));

        section_header(ui, "Memory", &gb_text(ram.total));
        self.ram_history.plot(ui, "plot_ram", "RAM", RAM_COLOR);

        ui.add_space(8.0);
        stats_ui(
            ui,
            &[
                ("In use", gb_text(ram.used)),
                ("Available", gb_text(Some(available))),
                ("Cached", gb_text(ram.cached)),
                (
                    "Temperature",
                    ram.temperature
                        .map_or("???".into(), |t| format!("{:.0}\u{b0}C", t.as_f32())),
                ),
            ],
        );

        ui.add_space(4.0);
        stats_ui(
            ui,
            &[
                ("Swap in use", swap_text(ram.swap_used, ram.swap_total)),
                ("Total", gb_text(ram.total)),
            ],
        );
    }

    /// The static facts of the disk at `index`, matched by id so a reordered list can't mix them up.
    fn disk_static(&self, info: &HardwareInfo, index: usize) -> Option<&hardware_scanner::model::DiskStatic> {
        let id = &info.disks().get(index)?.id;
        self.static_info.disks.iter().find(|facts| facts.id == *id)
    }

    fn disk_ui(&self, info: &HardwareInfo, index: usize, ui: &mut egui::Ui) {
        let Some(disk) = info.disks().get(index) else {
            return;
        };
        let Some(history) = self.disk_histories.get(index) else {
            return;
        };
        let facts = self.disk_static(info, index);

        let model = facts.map_or("", |facts| &*facts.model);
        section_header(ui, &format!("Disk {index} ({})", disk.id), model);
        history
            .active
            .plot(ui, format!("plot_disk_{index}"), "Active time", DISK_COLOR);

        ui.add_space(8.0);
        ui.strong("Read");
        history
            .read
            .plot(ui, format!("plot_disk_read_{index}"), "Read", DISK_COLOR);
        ui.add_space(8.0);
        ui.strong("Write");
        history
            .write
            .plot(ui, format!("plot_disk_write_{index}"), "Write", DISK_COLOR);

        ui.add_space(8.0);
        stats_ui(
            ui,
            &[
                ("Active time", percent_text(disk.active_time)),
                (
                    "Average response time",
                    disk.response_ms.map_or("???".into(), |ms| format!("{ms:.1} ms")),
                ),
                ("Read speed", disk.read_rate.map_or("???".into(), rate_text)),
                ("Write speed", disk.write_rate.map_or("???".into(), rate_text)),
            ],
        );

        ui.add_space(4.0);
        stats_ui(
            ui,
            &[
                ("Capacity", facts.map_or("???".into(), |facts| capacity_text(facts.size))),
                ("Type", facts.map_or("???", |facts| kind_text(facts.kind)).to_string()),
                (
                    "Temperature",
                    disk.temperature
                        .map_or("???".into(), |t| format!("{:.0}\u{b0}C", t.as_f32())),
                ),
            ],
        );

        if !disk.volumes.is_empty() {
            ui.add_space(12.0);
            ui.strong("Volumes");
            ui.add_space(4.0);
        }
        for volume in &disk.volumes {
            let used = volume.used.as_u64() as f32;
            let total = volume.total.as_u64().max(1) as f32;
            ui.label(format!(
                "{}  ({})",
                volume.mount.display(),
                volume.file_system
            ));
            // The empty part of a progress bar is drawn with `extreme_bg_color`.
            ui.scope(|ui| {
                ui.visuals_mut().extreme_bg_color = VOLUME_EMPTY_COLOR;
                ui.add(
                    egui::ProgressBar::new(used / total)
                        .fill(DISK_COLOR.gamma_multiply(0.55))
                        .text(format!(
                            "{} / {}",
                            capacity_text(volume.used),
                            capacity_text(volume.total)
                        )),
                );
            });
            ui.add_space(4.0);
        }
    }

    fn net_static(&self, info: &HardwareInfo, index: usize) -> Option<&hardware_scanner::model::NetStatic> {
        let id = &info.nets().get(index)?.id;
        self.static_info.nets.iter().find(|facts| facts.id == *id)
    }

    /// "Ethernet" or "Wi-Fi", falling back to the interface name.
    fn net_title(&self, info: &HardwareInfo, index: usize) -> String {
        match self.net_static(info, index).map(|facts| facts.kind) {
            Some(NetKind::Ethernet) => "Ethernet".into(),
            Some(NetKind::Wifi) => "Wi-Fi".into(),
            None => format!("Network {index}"),
        }
    }

    fn net_ui(&self, info: &HardwareInfo, index: usize, ui: &mut egui::Ui) {
        let Some(net) = info.nets().get(index) else {
            return;
        };
        let Some(history) = self.net_histories.get(index) else {
            return;
        };
        let facts = self.net_static(info, index);
        let is_wifi = facts.is_some_and(|facts| facts.kind == NetKind::Wifi);

        section_header(
            ui,
            &self.net_title(info, index),
            &net.id,
        );
        ui.strong("Receive");
        history
            .receive
            .plot(ui, format!("plot_net_rx_{index}"), "Receive", NET_COLOR);
        ui.add_space(8.0);
        ui.strong("Send");
        history
            .send
            .plot(ui, format!("plot_net_tx_{index}"), "Send", NET_COLOR);

        ui.add_space(8.0);
        let mut tiles = vec![
            ("Receive", net.receive_rate.map_or("???".into(), rate_text)),
            ("Send", net.send_rate.map_or("???".into(), rate_text)),
            (
                "Link speed",
                net.link_speed.map_or("???".into(), link_speed_text),
            ),
            (
                "Status",
                if net.connected { "Connected" } else { "Disconnected" }.to_string(),
            ),
        ];
        if is_wifi {
            tiles.push((
                "Signal",
                net.signal_dbm.map_or("???".into(), |dbm| format!("{dbm} dBm")),
            ));
        }
        stats_ui(ui, &tiles);

        ui.add_space(4.0);
        if let Some(mac) = facts.and_then(|facts| facts.mac.as_deref()) {
            stats_ui(ui, &[("MAC address", mac.to_string())]);
        }
        if !net.addresses.is_empty() {
            ui.add_space(4.0);
            ui.strong("IP addresses");
            for address in &net.addresses {
                ui.label(address.to_string());
            }
        }
    }

    fn gpu_ui(&self, info: &HardwareInfo, index: usize, ui: &mut egui::Ui) {
        let Some(Some(gpu)) = info.gpus().get(index) else {
            return;
        };
        let Some(history) = self.gpu_histories.get(index) else {
            return;
        };
        let facts = self
            .static_info
            .gpus
            .get(index)
            .and_then(|facts| facts.as_ref());

        let name = facts
            .map(|facts| &*facts.name)
            .filter(|name| !name.is_empty())
            .unwrap_or("");
        section_header(ui, &format!("GPU {index}"), name);
        history
            .load
            .plot(ui, format!("plot_gpu_{index}"), "GPU", GPU_COLOR);

        ui.add_space(8.0);
        ui.strong("Memory usage");
        history
            .memory
            .plot(ui, format!("plot_gpu_mem_{index}"), "VRAM", GPU_COLOR);

        ui.add_space(8.0);
        let power_limit = facts.and_then(|facts| facts.power_limit);
        stats_ui(
            ui,
            &[
                ("Utilization", percent_text(gpu.gpu)),
                (
                    "Memory",
                    format!("{} / {}", gb_text(gpu.memory_used), gb_text(gpu.memory_total)),
                ),
                (
                    "Temperature",
                    gpu.temperature
                        .map_or("???".into(), |t| format!("{:.0}\u{b0}C", t.as_f32())),
                ),
                ("Power", power_text(gpu.power, power_limit)),
                ("Fan", gpu.fan.map_or("n/a".into(), |p| percent_text(Some(p)))),
            ],
        );

        ui.add_space(4.0);
        stats_ui(
            ui,
            &[
                ("Core clock", gpu.core_clock.map_or("???".into(), mhz_text)),
                ("Memory clock", gpu.memory_clock.map_or("???".into(), mhz_text)),
                ("Encoder", gpu.encoder.map_or("???".into(), |p| percent_text(Some(p)))),
                ("Decoder", gpu.decoder.map_or("???".into(), |p| percent_text(Some(p)))),
            ],
        );

        ui.add_space(4.0);
        let (max_gen, max_width) = facts.map_or((None, None), |facts| {
            (facts.pcie_max_gen, facts.pcie_max_width)
        });
        stats_ui(
            ui,
            &[
                ("PCIe now", pcie_text(gpu.pcie_gen, gpu.pcie_width)),
                ("PCIe max", pcie_text(max_gen, max_width)),
                (
                    "Driver",
                    self.static_info
                        .gpu_driver
                        .as_deref()
                        .unwrap_or("???")
                        .to_string(),
                ),
            ],
        );
    }
}

/// Bits per second as "1.0 Gbps" or "866 Mbps".
fn link_speed_text(bits_per_second: u64) -> String {
    let bits = bits_per_second as f64;
    if bits >= 1e9 {
        format!("{:.1} Gbps", bits / 1e9)
    } else {
        format!("{:.0} Mbps", bits / 1e6)
    }
}

/// Compact rate for the narrow device cards: "3.4M", "812K", "0".
fn rate_short(rate: Bytes) -> String {
    let bytes = rate.as_u64() as f64;
    if bytes >= 1e6 {
        format!("{:.1}M", bytes / 1e6)
    } else if bytes >= 1e3 {
        format!("{:.0}K", bytes / 1e3)
    } else {
        format!("{bytes:.0}")
    }
}

fn kind_text(kind: DiskKind) -> &'static str {
    match kind {
        DiskKind::Nvme => "NVMe",
        DiskKind::Ssd => "SSD",
        DiskKind::Hdd => "HDD",
        DiskKind::Unknown => "",
    }
}

/// Decimal units, as disks are sold: "500.1 GB", "2.0 TB".
fn capacity_text(size: Bytes) -> String {
    let bytes = size.as_u64() as f64;
    if bytes >= 1e12 {
        format!("{:.1} TB", bytes / 1e12)
    } else {
        format!("{:.1} GB", bytes / 1e9)
    }
}

/// Transfer rate in bytes per second, "12.3 MB/s".
fn rate_text(rate: Bytes) -> String {
    let bytes = rate.as_u64() as f64;
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

fn gb_text(size: Option<Bytes>) -> String {
    size.map_or("???".into(), |size| format!("{:.1} GB", size.to_gb(1).as_f32()))
}

fn swap_text(used: Option<Bytes>, total: Option<Bytes>) -> String {
    match (used, total) {
        (_, Some(total)) if total.as_u64() == 0 => "None".into(),
        (Some(used), Some(total)) => format!("{} / {}", gb_text(Some(used)), gb_text(Some(total))),
        _ => "???".into(),
    }
}

fn mhz_text(clock: Mhz) -> String {
    format!("{} MHz", clock.as_u64())
}

fn power_text(power: Option<Watts>, limit: Option<Watts>) -> String {
    match (power, limit) {
        (Some(power), Some(limit)) => format!("{:.0} / {:.0} W", power.as_f32(), limit.as_f32()),
        (Some(power), None) => format!("{:.0} W", power.as_f32()),
        _ => "???".into(),
    }
}

fn pcie_text(generation: Option<u32>, width: Option<u32>) -> String {
    match (generation, width) {
        (Some(generation), Some(width)) => format!("Gen {generation} x{width}"),
        _ => "???".into(),
    }
}

fn ghz_text(clock: Mhz) -> String {
    format!("{:.2} GHz", clock.as_u64() as f32 / 1000.0)
}

/// Binary units, as the kernel reports cache sizes ("32 KB", "32 MB").
fn cache_text(size: Option<Bytes>) -> String {
    let Some(size) = size else {
        return "???".into();
    };
    let bytes = size.as_u64();
    if bytes >= 1024 * 1024 {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes / 1024)
    }
}

/// "d:hh:mm:ss", like Task Manager's Up time.
fn uptime_text(uptime: std::time::Duration) -> String {
    let secs = uptime.as_secs();
    format!(
        "{}:{:02}:{:02}:{:02}",
        secs / 86_400,
        secs % 86_400 / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
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

/// One core: a live graph with the core number top-left and its current load top-right.
fn core_tile(ui: &mut egui::Ui, width: f32, index: usize, usage: Percent, values: &[f64]) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, CORE_TILE_HEIGHT), Sense::hover());

    let title_color = ui.visuals().strong_text_color();
    let label_color = ui.visuals().weak_text_color();

    let painter = ui.painter();
    paint_sparkline(painter, rect, values, 100.0, CPU_COLOR);
    painter.text(
        rect.left_top() + egui::vec2(6.0, 4.0),
        Align2::LEFT_TOP,
        format!("Core {index}"),
        FontId::proportional(11.0),
        label_color,
    );
    painter.text(
        rect.right_top() + egui::vec2(-6.0, 4.0),
        Align2::RIGHT_TOP,
        format!("{:.0}%", usage.as_f32()),
        FontId::proportional(12.0),
        title_color,
    );
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

//! Reads hardware and process information from a Linux machine: CPU, memory, GPUs (NVIDIA),
//! disks, network interfaces, battery and running processes.
//!
//! [`Hardwarescanner`] does the reading. Call [`Hardwarescanner::initinalize`] once, send
//! [`Hardwarescanner::static_info`] to whoever displays the data, then call
//! [`Hardwarescanner::scan`] on a timer. Parts that fail (no GPU, unreadable sensor) are
//! reported in [`HardwareInfo::errors`] and leave only their own fields empty.

use std::time::Duration;

use crate::{
    disk_scanner::DiskScanner,
    gpu_scanner::NvidiaScanner,
    net_scanner::NetScanner,
    model::{
        CpuInfo, DiskInfo, GpuInfo, NetInfo, SystemInfo, Percent, Process, RamInfo, ScanError, ScanResult, StaticInfo, safe_div,
    },
    system_scanner::SystemScanner,
};

mod battery_scanner;
mod disk_scanner;
mod gpu_scanner;
mod net_scanner;
pub mod model;
mod sensor_table;
mod sysfs;
mod system_scanner;

/// Everything measured in one scan. Rebuilt from scratch on every [`Hardwarescanner::scan`].
#[derive(Debug)]
pub struct HardwareInfo {
    cpu: CpuInfo,
    ram: RamInfo,
    processes: Vec<Process>,
    gpus: Vec<Option<GpuInfo>>,
    disks: Vec<DiskInfo>,
    nets: Vec<NetInfo>,
    system: SystemInfo,
    errors: Vec<ScanError>,
}
impl HardwareInfo {
    fn new() -> Self {
        Self {
            ram: RamInfo::default(),
            cpu: CpuInfo::default(),
            gpus: vec![],
            disks: vec![],
            nets: vec![],
            system: SystemInfo::default(),
            errors: vec![],
            processes: vec![],
        }
    }

    /// CPU load, temperature, clock and uptime.
    pub fn cpu(&self) -> &CpuInfo {
        &self.cpu
    }

    /// Memory and swap.
    pub fn ram(&self) -> &RamInfo {
        &self.ram
    }

    /// One entry per GPU index; `None` for a GPU that could not be read this scan.
    pub fn gpus(&self) -> &[Option<GpuInfo>] {
        &self.gpus
    }

    /// Physical disks, in the same order as [`StaticInfo::disks`].
    pub fn disks(&self) -> &[DiskInfo] {
        &self.disks
    }

    /// Physical network interfaces, in the same order as [`StaticInfo::nets`].
    pub fn nets(&self) -> &[NetInfo] {
        &self.nets
    }

    /// Load average and battery.
    pub fn system(&self) -> &SystemInfo {
        &self.system
    }

    /// All running processes (not threads), in no particular order.
    pub fn processes(&self) -> &[Process] {
        &self.processes
    }

    /// The same list, mutable so the UI can sort it in place.
    pub fn processes_mut(&mut self) -> &mut [Process] {
        &mut self.processes
    }

    /// Problems met during this scan.
    pub fn errors(&self) -> &[ScanError] {
        &self.errors
    }

    /// Removes and returns the errors, leaving the list empty.
    pub fn take_errors(&mut self) -> Vec<ScanError> {
        std::mem::take(&mut self.errors)
    }

    /// Empties every field so the next scan starts from a blank slate.
    fn clear(&mut self) {
        self.gpus.clear();
        self.disks.clear();
        self.nets.clear();
        self.system = SystemInfo::default();
        self.errors.clear();
        self.processes.clear();
        self.cpu.core_usage.clear();
        self.cpu.temperature = None;
        self.cpu.total_usage = None;
        self.cpu.frequency = None;
        self.cpu.uptime = None;
        self.ram = RamInfo::default();
    }
}
impl Default for HardwareInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Owns all the individual scanners and the state they carry between scans (previous counters,
/// GPU sample cursors).
pub struct Hardwarescanner {
    cpu_rams: SystemScanner,
    gpus: Option<NvidiaScanner>,
    disks: DiskScanner,
    nets: NetScanner,
}
impl Hardwarescanner {
    /// Fails only when sysinfo cannot start. Missing GPUs or sensors are not errors here.
    pub fn new() -> ScanResult<Self> {
        Ok(Self {
            gpus: None,
            disks: DiskScanner::new(),
            nets: NetScanner::new(),
            cpu_rams: SystemScanner::new()?,
        })
    }

    /// Facts that never change at runtime. Call after `initinalize`.
    pub fn static_info(&self) -> StaticInfo {
        StaticInfo {
            cpu: self.cpu_rams.cpu_static(),
            gpu_driver: self
                .gpus
                .as_ref()
                .and_then(|gpus| gpus.driver_version())
                .map(Into::into),
            gpus: self
                .gpus
                .as_ref()
                .map(|gpus| gpus.static_infos())
                .unwrap_or_default(),
            disks: self.disks.static_infos(),
            nets: self.nets.static_infos(),
            system: self.cpu_rams.system_static(),
        }
    }

    /// Loads the GPU scanner and performs two scans one second apart, so the returned info
    /// already has CPU load, process load and the transfer rates that need two samples.
    ///
    /// When no GPU can be used, the reason goes into `gpu_error` and scanning continues without.
    pub fn initinalize(&mut self, gpu_error: &mut Option<ScanError>) -> HardwareInfo {
        let mut info = HardwareInfo::new();
        self.load_gpu(gpu_error);
        self.scan(&mut info);
        std::thread::sleep(Duration::from_secs(1));
        self.scan(&mut info);
        info
    }

    /// Replaces the contents of `info` with a fresh scan.
    pub fn scan(&mut self, info: &mut HardwareInfo) {
        self.cpu_rams.scan();
        info.clear();

        if let Err(err) =
            self.cpu_rams
                .processes(&mut info.processes, &mut info.errors, self.gpus.as_mut())
        {
            info.errors.push(err.into());
        }

        self.scan_ram(info);
        self.scan_cpu(info);
        if let Some(gpu_scanner) = &self.gpus {
            self.scan_gpu(info, gpu_scanner)
        }
        self.disks.scan(&mut info.disks);
        self.nets.scan(&mut info.nets);
        info.system.load = self.cpu_rams.load_average();
        info.system.battery = battery_scanner::read_battery();
    }

    fn scan_ram(&self, info: &mut HardwareInfo) {
        info.ram.used = Some(self.cpu_rams.ram_usage());
        info.ram.total = Some(self.cpu_rams.ram_total());
        info.ram.temperature = self.cpu_rams.ram_temprature();
        info.ram.available = Some(self.cpu_rams.ram_available());
        info.ram.cached = self.cpu_rams.ram_cached();
        info.ram.swap_used = Some(self.cpu_rams.swap_used());
        info.ram.swap_total = Some(self.cpu_rams.swap_total());
    }

    fn scan_cpu(&self, info: &mut HardwareInfo) {
        let mut total = 0.0;
        for cpu in self.cpu_rams.cpus() {
            let usage = cpu.usage();
            total += usage.as_f32();
            info.cpu.core_usage.push(usage)
        }
        total = safe_div(total, self.cpu_rams.cpu_count() as f32);
        info.cpu.total_usage = Some(Percent::new(total));
        info.cpu.temperature = self.cpu_rams.cpu_temprature();
        info.cpu.frequency = self.cpu_rams.cpu_frequency();
        info.cpu.uptime = self.cpu_rams.uptime();
    }

    fn scan_gpu(&self, info: &mut HardwareInfo, gpu_scanner: &NvidiaScanner) {
        let gpus = match gpu_scanner.gpu_iter() {
            Ok(val) => val,
            Err(err) => {
                let gpu_len = gpu_scanner.gpu_count().unwrap_or(info.gpus.capacity());
                for _ in 0..gpu_len {
                    info.gpus.push(None);
                }
                info.errors.push(err.into());
                return;
            }
        };

        for gpu in gpus {
            let gpu = match gpu {
                Ok(val) => val,
                Err(err) => {
                    info.errors.push(err.into());
                    continue;
                }
            };

            let mut gpu_info = GpuInfo::default();
            if let Err(err) = gpu.scan(&mut gpu_info) {
                info.errors.push(err.into());
                info.gpus.push(None);
                continue;
            }

            info.gpus.push(Some(gpu_info));
        }
    }

    fn load_gpu(&mut self, gpu_error: &mut Option<ScanError>) {
        let nvidia = match NvidiaScanner::new() {
            Ok(val) => val,
            Err(err) => {
                *gpu_error = Some(err.into());
                return;
            }
        };
        self.gpus = Some(nvidia);
    }
}

#[cfg(test)]
mod tests {
    use nvml_wrapper::error::NvmlError;

    use super::*;
    use crate::model::{Degree, LoadAverage};

    fn some_error() -> ScanError {
        ScanError::Gpu(NvmlError::NotFound)
    }

    #[test]
    fn a_new_info_is_empty() {
        let info = HardwareInfo::default();
        assert!(info.processes().is_empty());
        assert!(info.gpus().is_empty());
        assert!(info.disks().is_empty());
        assert!(info.nets().is_empty());
        assert!(info.errors().is_empty());
        assert!(info.cpu().core_usage.is_empty());
        assert!(info.system().battery.is_none());
    }

    #[test]
    fn take_errors_hands_them_over_and_empties_the_list() {
        let mut info = HardwareInfo::default();
        info.errors.push(some_error());
        info.errors.push(some_error());

        assert_eq!(info.take_errors().len(), 2);
        assert!(info.errors().is_empty());
        assert!(info.take_errors().is_empty());
    }

    #[test]
    fn clear_resets_every_live_field() {
        let mut info = HardwareInfo::default();
        info.cpu.core_usage.push(Percent::new(10.0));
        info.cpu.total_usage = Some(Percent::new(10.0));
        info.cpu.temperature = Some(Degree::new(50.0));
        info.gpus.push(None);
        info.disks.push(DiskInfo::default());
        info.nets.push(NetInfo::default());
        info.errors.push(some_error());
        info.system.load = Some(LoadAverage::default());

        info.clear();

        assert!(info.cpu().core_usage.is_empty());
        assert!(info.cpu().total_usage.is_none());
        assert!(info.cpu().temperature.is_none());
        assert!(info.gpus().is_empty());
        assert!(info.disks().is_empty());
        assert!(info.nets().is_empty());
        assert!(info.errors().is_empty());
        assert!(info.system().load.is_none());
    }

    /// Runs the real scanners against this machine, so it only checks things that are true on
    /// any Linux box: memory exists, cores exist, and the test process itself is listed.
    #[test]
    fn a_real_scan_sees_memory_cores_and_this_process() {
        let mut scanner = Hardwarescanner::new().expect("sysinfo should start");
        let mut info = HardwareInfo::default();
        scanner.scan(&mut info);

        assert!(info.ram().total.is_some_and(|total| total.as_u64() > 0));
        assert!(!info.cpu().core_usage.is_empty());

        let own = sysinfo::Pid::from_u32(std::process::id());
        assert!(
            info.processes().iter().any(|process| process.pid == own),
            "the test process {own} is missing from the process list"
        );

        let facts = scanner.static_info();
        assert!(facts.cpu.logical_cores > 0);
        assert_eq!(facts.disks.len(), info.disks().len());
    }
}

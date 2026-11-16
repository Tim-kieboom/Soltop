use std::time::Duration;

use crate::{
    disk_scanner::DiskScanner,
    gpu_scanner::NvidiaScanner,
    model::{
        CpuInfo, DiskInfo, GpuInfo, Percent, Process, RamInfo, ScanError, ScanResult, StaticInfo, safe_div,
    },
    system_scanner::SystemScanner,
};

mod disk_scanner;
mod gpu_scanner;
pub mod model;
mod sensor_table;
mod sysfs;
mod system_scanner;

#[derive(Debug)]
pub struct HardwareInfo {
    cpu: CpuInfo,
    ram: RamInfo,
    processes: Vec<Process>,
    gpus: Vec<Option<GpuInfo>>,
    disks: Vec<DiskInfo>,
    errors: Vec<ScanError>,
}
impl HardwareInfo {
    fn new() -> Self {
        Self {
            ram: RamInfo::default(),
            cpu: CpuInfo::default(),
            gpus: vec![],
            disks: vec![],
            errors: vec![],
            processes: vec![],
        }
    }

    pub fn cpu(&self) -> &CpuInfo {
        &self.cpu
    }

    pub fn ram(&self) -> &RamInfo {
        &self.ram
    }

    pub fn gpus(&self) -> &[Option<GpuInfo>] {
        &self.gpus
    }

    pub fn disks(&self) -> &[DiskInfo] {
        &self.disks
    }

    pub fn processes(&self) -> &[Process] {
        &self.processes
    }

    pub fn processes_mut(&mut self) -> &mut [Process] {
        &mut self.processes
    }

    pub fn errors(&self) -> &[ScanError] {
        &self.errors
    }

    pub fn take_errors(&mut self) -> Vec<ScanError> {
        std::mem::take(&mut self.errors)
    }

    fn clear(&mut self) {
        self.gpus.clear();
        self.disks.clear();
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

pub struct Hardwarescanner {
    cpu_rams: SystemScanner,
    gpus: Option<NvidiaScanner>,
    disks: DiskScanner,
}
impl Hardwarescanner {
    pub fn new() -> ScanResult<Self> {
        Ok(Self {
            gpus: None,
            disks: DiskScanner::new(),
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
        }
    }

    pub fn initinalize(&mut self, gpu_error: &mut Option<ScanError>) -> HardwareInfo {
        let mut info = HardwareInfo::new();
        self.load_gpu(gpu_error);
        self.scan(&mut info);
        std::thread::sleep(Duration::from_secs(1));
        self.scan(&mut info);
        info
    }

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

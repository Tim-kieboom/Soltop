use std::{ffi::OsStr, time::Duration};

use nvml_wrapper::error::NvmlError;
use thiserror::Error;

use crate::hardware_info::{
    gpu_scanner::NvidiaScanner,
    system_scanner::SystemScanner,
    values::{Bytes, Degree, GpuNumbers, Percent, safe_div},
};

mod gpu_scanner;
mod sensor_table;
mod system_scanner;
pub mod values;

type SysResult<T = ()> = Result<T, ScanError>;

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("{0}")]
    Sysinfo(#[from] sysinfo::Error),
    #[error("{0}")]
    Gpu(#[from] NvmlError),
}

#[derive(Debug, Default)]
pub struct CpuInfo {
    pub core_usage: Vec<Percent>,
    pub temperature: Option<Degree>,
    pub total_usage: Option<Percent>,
}

#[derive(Debug, Default)]
pub struct RamInfo {
    pub used: Option<Bytes>,
    pub total: Option<Bytes>,
    pub temperature: Option<Degree>,
}

#[derive(Debug, Default)]
pub struct GpuInfo {
    pub gpu: Option<Percent>,
    pub memory_usage: Option<Percent>,
    pub memory_total: Option<Bytes>,
    pub temperature: Option<Degree>,
}

#[derive(Debug)]
pub struct Process {
    pub pid: sysinfo::Pid,
    pub memory: Bytes,
    pub name: Box<OsStr>,
    pub cpu_usage: Percent,
    pub gpu_memory: Option<Bytes>,
    pub gpu_usage: Option<Percent>,
    pub gpu_number: Option<GpuNumbers>,
}

pub struct Hardwarescanner {
    cpu_rams: SystemScanner,
    gpus: Option<NvidiaScanner>,
}
impl Hardwarescanner {
    pub fn new() -> SysResult<Self> {
        Ok(Self {
            gpus: None,
            cpu_rams: SystemScanner::new()?,
        })
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
    }

    fn scan_ram(&self, info: &mut HardwareInfo) {
        info.ram.used = Some(self.cpu_rams.ram_usage());
        info.ram.total = Some(self.cpu_rams.ram_total());
        info.ram.temperature = self.cpu_rams.ram_temprature();
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

#[derive(Debug)]
pub struct HardwareInfo {
    cpu: CpuInfo,
    ram: RamInfo,
    processes: Vec<Process>,
    gpus: Vec<Option<GpuInfo>>,
    errors: Vec<ScanError>,
}
impl HardwareInfo {
    pub fn cpu(&self) -> &CpuInfo {
        &self.cpu
    }

    pub fn ram(&self) -> &RamInfo {
        &self.ram
    }

    pub fn gpus(&self) -> &[Option<GpuInfo>] {
        &self.gpus
    }

    pub fn processes(&self) -> &[Process] {
        &self.processes
    }

    pub fn errors(&self) -> &[ScanError] {
        &self.errors
    }

    fn new() -> Self {
        Self {
            ram: RamInfo::default(),
            cpu: CpuInfo::default(),
            gpus: vec![],
            errors: vec![],
            processes: vec![],
        }
    }

    fn clear(&mut self) {
        self.gpus.clear();
        self.errors.clear();
        self.processes.clear();
        self.cpu.core_usage.clear();
        self.cpu.temperature = None;
        self.cpu.total_usage = None;
        self.ram = RamInfo::default();
    }
}

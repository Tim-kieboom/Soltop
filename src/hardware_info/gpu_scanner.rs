use std::collections::HashMap;

use nvml_wrapper::{
    Device, Nvml,
    enum_wrappers::device::TemperatureSensor,
    enums::device::UsedGpuMemory,
    error::NvmlError,
    struct_wrappers::device::{ProcessInfo, ProcessUtilizationSample},
};

use crate::hardware_info::{
    ScanError,
    values::{Bytes, Degree, GpuNumbers, Percent},
};

pub type GpuResult<T = ()> = Result<T, NvmlError>;

pub struct GpuProcess {
    pub pid: sysinfo::Pid,
    pub gpu_numbers: GpuNumbers,
    pub gpu_usage: Option<Percent>,
    pub memory_usage: Option<Bytes>,
}

pub struct NvidiaScanner {
    nvml: Nvml,
    processes: HashMap<sysinfo::Pid, GpuProcess>,
}
impl NvidiaScanner {
    pub fn new() -> GpuResult<Self> {
        let nvml = Nvml::init()?;
        Ok(Self {
            nvml,
            processes: HashMap::new(),
        })
    }

    pub fn gpu_count(&self) -> GpuResult<usize> {
        self.nvml.device_count().map(|c| c as usize)
    }

    pub fn gpu_iter(&self) -> GpuResult<impl Iterator<Item = GpuResult<NvidiaGpu<'_>>>> {
        Ok((0..self.gpu_count()?)
            .into_iter()
            .map(|i| self.get_gpu(i as u32).map(NvidiaGpu)))
    }

    pub fn gpu_processes<'a>(
        &'a mut self,
        errors: &mut Vec<ScanError>,
    ) -> GpuResult<&'a HashMap<sysinfo::Pid, GpuProcess>> {
        self.processes.clear();

        for gpu_number in 0..self.gpu_count()? {
            let device = self.get_gpu(gpu_number as u32)?;
            let stats_result = device.process_utilization_stats(None);
            let compute_result = device.running_compute_processes();
            let graphics_result = device.running_graphics_processes();

            if let Err(err) = self.memory_process(gpu_number, compute_result, graphics_result) {
                errors.push(err.into());
            }

            if let Err(err) = self.usage_process(stats_result) {
                errors.push(err.into());
            }
        }
        Ok(&self.processes)
    }

    pub fn memory_process<'a>(
        &mut self,
        gpu_number: usize,
        compute_result: GpuResult<Vec<ProcessInfo>>,
        graphics_result: GpuResult<Vec<ProcessInfo>>,
    ) -> GpuResult {
        let mut processes = vec![];
        processes = compute_result?;
        processes.extend(graphics_result?);

        for process in processes {
            let (pid, process) = to_gpu_process_entry(process, gpu_number);

            match self.processes.get_mut(&pid) {
                Some(old_process) => old_process.combine(process),
                None => _ = self.processes.insert(pid, process),
            }
        }
        Ok(())
    }

    pub fn usage_process<'a>(
        &mut self,
        stats: GpuResult<Vec<ProcessUtilizationSample>>,
    ) -> GpuResult {
        for stat in stats? {
            let pid = sysinfo::Pid::from_u32(stat.pid);
            let usage = Percent::new(stat.sm_util as f32);
            if let Some(process) = self.processes.get_mut(&pid) {
                process.gpu_usage = Some(usage)
            }
        }
        Ok(())
    }

    fn get_gpu(&self, index: u32) -> GpuResult<Device<'_>> {
        self.nvml.device_by_index(index)
    }
}

pub struct NvidiaGpu<'a>(nvml_wrapper::Device<'a>);
impl<'a> NvidiaGpu<'a> {
    pub fn scan(&self, info: &mut super::GpuInfo) -> GpuResult {
        let memory = self.0.memory_info()?;
        info.memory_total = Some(Bytes::new(memory.total));
        info.memory_usage = Some(Percent::new(
            (memory.used as f32 / memory.total as f32) * 100.0,
        ));
        info.gpu = Some(
            self.0
                .utilization_rates()
                .map(|u| Percent::new(u.gpu as f32))?,
        );

        info.temperature = Some(
            self.0
                .temperature(TemperatureSensor::Gpu)
                .map(|t| Degree::new(t as f32))?,
        );

        Ok(())
    }
}

impl GpuProcess {
    fn combine(&mut self, other: Self) {
        if self.gpu_numbers.has_overlap(other.gpu_numbers) {
            return;
        }

        debug_assert_eq!(self.pid, other.pid);
        self.gpu_numbers = self.gpu_numbers.union(other.gpu_numbers);
        self.memory_usage = match (self.memory_usage, other.memory_usage) {
            (None, None) => None,
            (None, usage) | (usage, None) => usage,
            (Some(a), Some(b)) => Some(Bytes::new(a.as_u64() + b.as_u64())),
        };
    }
}

fn to_gpu_process_entry(process: ProcessInfo, gpu_number: usize) -> (sysinfo::Pid, GpuProcess) {
    let pid = sysinfo::Pid::from_u32(process.pid);
    let memory_usage = match process.used_gpu_memory {
        UsedGpuMemory::Unavailable => None,
        UsedGpuMemory::Used(bytes) => Some(Bytes::new(bytes)),
    };
    (
        pid,
        GpuProcess {
            pid,
            memory_usage,
            gpu_usage: None,
            gpu_numbers: GpuNumbers::from_index(gpu_number),
        },
    )
}

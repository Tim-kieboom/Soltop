use std::collections::HashMap;

use nvml_wrapper::{
    Device, Nvml,
    enum_wrappers::device::{Clock, TemperatureSensor},
    enums::device::UsedGpuMemory,
    error::NvmlError,
    struct_wrappers::device::{ProcessInfo, ProcessUtilizationSample},
};

use crate::{
    ScanError,
    model::{Bytes, Degree, GpuNumbers, GpuStatic, Mhz, Percent, Watts},
};

/// Result of an NVML call.
pub type GpuResult<T = ()> = Result<T, NvmlError>;

/// A process using one or more GPUs, merged across all of them.
pub struct GpuProcess {
    pub pid: sysinfo::Pid,
    /// Every GPU the process uses.
    pub gpu_numbers: GpuNumbers,
    /// Video memory held, summed over GPUs; `None` when the driver does not report it.
    pub memory_usage: Option<Bytes>,
    /// SM utilization of the newest NVML sample; `None` when NVML had no sample this tick.
    pub gpu_usage: Option<Percent>,
}

/// Reads NVIDIA GPUs through NVML.
pub struct NvidiaScanner {
    nvml: Nvml,
    processes: HashMap<sysinfo::Pid, GpuProcess>,
    /// Newest utilization-sample timestamp seen per GPU index, passed back to NVML
    /// so it only returns samples newer than the previous scan.
    last_seen: HashMap<usize, u64>,
}
impl NvidiaScanner {
    /// Loads NVML. Fails when there is no NVIDIA driver or library on the machine.
    pub fn new() -> GpuResult<Self> {
        let nvml = Nvml::init()?;
        Ok(Self {
            nvml,
            processes: HashMap::new(),
            last_seen: HashMap::new(),
        })
    }

    /// Number of NVIDIA GPUs.
    pub fn gpu_count(&self) -> GpuResult<usize> {
        self.nvml.device_count().map(|c| c as usize)
    }

    /// One entry per GPU index, each either a usable GPU or the error from opening it.
    pub fn gpu_iter(&self) -> GpuResult<impl Iterator<Item = GpuResult<NvidiaGpu<'_>>>> {
        Ok((0..self.gpu_count()?)
            .into_iter()
            .map(|i| self.get_gpu(i as u32).map(NvidiaGpu)))
    }

    /// Processes using any GPU, merged by pid, with video memory and utilization.
    ///
    /// Advances the per-GPU utilization cursor, so call it exactly once per scan. Failures
    /// of single memory or utilization queries are pushed to `errors` and the rest of the data
    /// is still returned, but failing to count or open a GPU fails the whole call.
    pub fn gpu_processes<'a>(
        &'a mut self,
        errors: &mut Vec<ScanError>,
    ) -> GpuResult<&'a HashMap<sysinfo::Pid, GpuProcess>> {
        self.processes.clear();

        for gpu_number in 0..self.gpu_count()? {
            let device = self.get_gpu(gpu_number as u32)?;
            let compute_result = device.running_compute_processes();
            let graphics_result = device.running_graphics_processes();
            let last_seen = self.last_seen.get(&gpu_number).copied().unwrap_or(0);
            let utilization_result = device.process_utilization_stats(last_seen);

            self.memory_process(gpu_number, compute_result, errors);
            self.memory_process(gpu_number, graphics_result, errors);
            self.utilization_process(gpu_number, utilization_result, errors);
        }
        Ok(&self.processes)
    }

    fn memory_process(
        &mut self,
        gpu_number: usize,
        result: GpuResult<Vec<ProcessInfo>>,
        errors: &mut Vec<ScanError>,
    ) {
        let processes = match result {
            Ok(processes) => processes,
            Err(err) => {
                errors.push(err.into());
                return;
            }
        };

        for process in processes {
            let (pid, process) = to_gpu_process_entry(process, gpu_number);

            match self.processes.get_mut(&pid) {
                Some(old_process) => old_process.combine(process),
                None => _ = self.processes.insert(pid, process),
            }
        }
    }

    /// Merges per-process SM utilization into the process map.
    /// `NotFound` just means NVML had no new samples since `last_seen`, so it is not an error.
    fn utilization_process(
        &mut self,
        gpu_number: usize,
        result: GpuResult<Vec<ProcessUtilizationSample>>,
        errors: &mut Vec<ScanError>,
    ) {
        let samples = match result {
            Ok(samples) => samples,
            Err(NvmlError::NotFound) => return,
            Err(err) => {
                errors.push(err.into());
                return;
            }
        };

        let last_seen = self.last_seen.get(&gpu_number).copied().unwrap_or(0);
        let (newest, max_timestamp) = newest_per_pid(&samples, last_seen);
        self.last_seen.insert(gpu_number, max_timestamp);
        merge_utilization(&mut self.processes, gpu_number, newest);
    }

    /// The installed NVIDIA driver version, e.g. "570.133.07".
    pub fn driver_version(&self) -> Option<String> {
        self.nvml.sys_driver_version().ok()
    }

    /// Static facts per GPU index; `None` for a GPU that could not be opened.
    pub fn static_infos(&self) -> Vec<Option<GpuStatic>> {
        let count = self.gpu_count().unwrap_or(0);
        (0..count)
            .map(|i| {
                let device = self.get_gpu(i as u32).ok()?;
                Some(NvidiaGpu(device).static_info())
            })
            .collect()
    }

    fn get_gpu(&self, index: u32) -> GpuResult<Device<'_>> {
        self.nvml.device_by_index(index)
    }
}

/// One opened NVIDIA GPU.
pub struct NvidiaGpu<'a>(nvml_wrapper::Device<'a>);
impl<'a> NvidiaGpu<'a> {
    /// Fills `info` with the GPU's current numbers.
    ///
    /// Load, memory and temperature are required: if one fails the whole call fails. Clocks,
    /// power, fan, encoder, decoder and PCIe are optional and stay `None` when unsupported.
    pub fn scan(&self, info: &mut super::GpuInfo) -> GpuResult {
        let memory = self.0.memory_info()?;
        info.memory_total = Some(Bytes::new(memory.total));
        info.memory_usage = Some(Percent::new(
            (memory.used as f32 / memory.total as f32) * 100.0,
        ));
        info.memory_used = Some(Bytes::new(memory.used));
        info.gpu = Some(
            self.0
                .utilization_rates()
                .map(|u| Percent::new(u.gpu as f32))?,
        );

        // Not every GPU supports every metric (fan on laptops, encoder on some), so a
        // failing query only leaves that field empty instead of failing the whole scan.
        info.core_clock = self.0.clock_info(Clock::Graphics).ok().map(|m| Mhz::new(m as u64));
        info.memory_clock = self.0.clock_info(Clock::Memory).ok().map(|m| Mhz::new(m as u64));
        info.power = self.0.power_usage().ok().map(milliwatts);
        info.fan = self.0.fan_speed(0).ok().map(|p| Percent::new(p as f32));
        info.encoder = self
            .0
            .encoder_utilization()
            .ok()
            .map(|u| Percent::new(u.utilization as f32));
        info.decoder = self
            .0
            .decoder_utilization()
            .ok()
            .map(|u| Percent::new(u.utilization as f32));
        info.pcie_gen = self.0.current_pcie_link_gen().ok();
        info.pcie_width = self.0.current_pcie_link_width().ok();

        info.temperature = Some(
            self.0
                .temperature(TemperatureSensor::Gpu)
                .map(|t| Degree::new(t as f32))?,
        );

        Ok(())
    }
}

impl<'a> NvidiaGpu<'a> {
    /// The GPU's name, power limit and maximum PCIe link.
    pub fn static_info(&self) -> GpuStatic {
        GpuStatic {
            name: self.0.name().unwrap_or_default().into(),
            power_limit: self.0.enforced_power_limit().ok().map(milliwatts),
            pcie_max_gen: self.0.max_pcie_link_gen().ok(),
            pcie_max_width: self.0.max_pcie_link_width().ok(),
        }
    }
}

/// NVML reports power in milliwatts.
fn milliwatts(mw: u32) -> Watts {
    Watts::new(mw as f32 / 1000.0)
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
        self.gpu_usage = max_usage(self.gpu_usage, other.gpu_usage);
    }
}

/// Newest sample per pid as `(timestamp, sm_util)`, plus the highest timestamp seen
/// (never lower than `last_seen`, so the cursor can't move backwards).
fn newest_per_pid(
    samples: &[ProcessUtilizationSample],
    last_seen: u64,
) -> (HashMap<u32, (u64, u32)>, u64) {
    let mut newest: HashMap<u32, (u64, u32)> = HashMap::new();
    let mut max_timestamp = last_seen;
    for sample in samples {
        max_timestamp = max_timestamp.max(sample.timestamp);
        let entry = newest
            .entry(sample.pid)
            .or_insert((sample.timestamp, sample.sm_util));
        if sample.timestamp >= entry.0 {
            *entry = (sample.timestamp, sample.sm_util);
        }
    }
    (newest, max_timestamp)
}

/// Writes utilization into the process map, creating entries for pids that had no
/// memory entry and marking `gpu_number` as one of the process's GPUs.
fn merge_utilization(
    processes: &mut HashMap<sysinfo::Pid, GpuProcess>,
    gpu_number: usize,
    newest: HashMap<u32, (u64, u32)>,
) {
    for (pid, (_, sm_util)) in newest {
        let pid = sysinfo::Pid::from_u32(pid);
        let usage = Percent::new(sm_util as f32);
        let gpu_numbers = GpuNumbers::from_index(gpu_number);

        let entry = processes.entry(pid).or_insert_with(|| GpuProcess {
            pid,
            gpu_numbers,
            memory_usage: None,
            gpu_usage: None,
        });
        entry.gpu_numbers = entry.gpu_numbers.union(gpu_numbers);
        entry.gpu_usage = max_usage(entry.gpu_usage, Some(usage));
    }
}

/// Percentages across GPUs are not additive, so keep the highest one.
fn max_usage(a: Option<Percent>, b: Option<Percent>) -> Option<Percent> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a.as_f32() >= b.as_f32() { a } else { b }),
        (a, b) => a.or(b),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32, timestamp: u64, sm_util: u32) -> ProcessUtilizationSample {
        ProcessUtilizationSample {
            pid,
            timestamp,
            sm_util,
            mem_util: 0,
            enc_util: 0,
            dec_util: 0,
        }
    }

    fn gpu_process(pid: u32, gpu: usize, mem: Option<u64>, usage: Option<f32>) -> GpuProcess {
        GpuProcess {
            pid: sysinfo::Pid::from_u32(pid),
            gpu_numbers: GpuNumbers::from_index(gpu),
            memory_usage: mem.map(Bytes::new),
            gpu_usage: usage.map(Percent::new),
        }
    }

    fn pid(p: u32) -> sysinfo::Pid {
        sysinfo::Pid::from_u32(p)
    }

    #[test]
    fn newest_per_pid_keeps_latest_sample() {
        let samples = [sample(1, 100, 10), sample(1, 300, 70), sample(1, 200, 40)];
        let (newest, _) = newest_per_pid(&samples, 0);
        assert_eq!(newest[&1], (300, 70));
    }

    #[test]
    fn newest_per_pid_tracks_each_pid_separately() {
        let samples = [sample(1, 100, 10), sample(2, 200, 90)];
        let (newest, _) = newest_per_pid(&samples, 0);
        assert_eq!(newest.len(), 2);
        assert_eq!(newest[&1], (100, 10));
        assert_eq!(newest[&2], (200, 90));
    }

    #[test]
    fn newest_per_pid_returns_max_timestamp() {
        let samples = [sample(1, 100, 10), sample(2, 500, 20), sample(3, 300, 30)];
        let (_, max) = newest_per_pid(&samples, 0);
        assert_eq!(max, 500);
    }

    #[test]
    fn newest_per_pid_cursor_never_moves_backwards() {
        let (_, max) = newest_per_pid(&[sample(1, 100, 10)], 1_000);
        assert_eq!(max, 1_000);
        let (newest, max) = newest_per_pid(&[], 1_000);
        assert!(newest.is_empty());
        assert_eq!(max, 1_000);
    }

    #[test]
    fn merge_utilization_creates_entry_for_unknown_pid() {
        let mut processes = HashMap::new();
        merge_utilization(&mut processes, 1, HashMap::from([(42, (10, 55))]));

        let p = &processes[&pid(42)];
        assert_eq!(p.gpu_usage, Some(Percent::new(55.0)));
        assert_eq!(p.memory_usage, None);
        assert_eq!(p.gpu_numbers.as_u64(), GpuNumbers::from_index(1).as_u64());
    }

    #[test]
    fn merge_utilization_keeps_existing_memory() {
        let mut processes = HashMap::new();
        processes.insert(pid(7), gpu_process(7, 0, Some(2048), None));
        merge_utilization(&mut processes, 0, HashMap::from([(7, (10, 30))]));

        let p = &processes[&pid(7)];
        assert_eq!(p.memory_usage, Some(Bytes::new(2048)));
        assert_eq!(p.gpu_usage, Some(Percent::new(30.0)));
    }

    #[test]
    fn merge_utilization_takes_max_across_gpus_and_unions_numbers() {
        let mut processes = HashMap::new();
        merge_utilization(&mut processes, 0, HashMap::from([(7, (10, 80))]));
        merge_utilization(&mut processes, 1, HashMap::from([(7, (10, 20))]));

        let p = &processes[&pid(7)];
        assert_eq!(p.gpu_usage, Some(Percent::new(80.0)));
        assert!(p.gpu_numbers.contains(0));
        assert!(p.gpu_numbers.contains(1));
    }

    #[test]
    fn max_usage_handles_none() {
        let a = Some(Percent::new(10.0));
        let b = Some(Percent::new(20.0));
        assert_eq!(max_usage(None, None), None);
        assert_eq!(max_usage(a, None), a);
        assert_eq!(max_usage(None, b), b);
        assert_eq!(max_usage(a, b), b);
        assert_eq!(max_usage(b, a), b);
    }

    #[test]
    fn combine_sums_memory_across_different_gpus() {
        let mut a = gpu_process(1, 0, Some(100), Some(10.0));
        a.combine(gpu_process(1, 1, Some(50), Some(40.0)));

        assert_eq!(a.memory_usage, Some(Bytes::new(150)));
        assert_eq!(a.gpu_usage, Some(Percent::new(40.0)));
        assert!(a.gpu_numbers.contains(0) && a.gpu_numbers.contains(1));
    }

    #[test]
    fn combine_does_not_double_count_same_gpu() {
        // same pid listed as both compute and graphics process on one GPU
        let mut a = gpu_process(1, 0, Some(100), None);
        a.combine(gpu_process(1, 0, Some(100), None));

        assert_eq!(a.memory_usage, Some(Bytes::new(100)));
    }

    #[test]
    fn entry_maps_unavailable_memory_to_none() {
        let info = ProcessInfo {
            pid: 9,
            used_gpu_memory: UsedGpuMemory::Unavailable,
            gpu_instance_id: None,
            compute_instance_id: None,
        };
        let (p, entry) = to_gpu_process_entry(info, 2);

        assert_eq!(p, pid(9));
        assert_eq!(entry.memory_usage, None);
        assert_eq!(entry.gpu_usage, None);
        assert!(entry.gpu_numbers.contains(2));
    }

    #[test]
    fn milliwatts_converts_to_watts() {
        assert_eq!(milliwatts(85_500), Watts::new(85.5));
        assert_eq!(milliwatts(0), Watts::new(0.0));
    }
}

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    ScanError,
    gpu_scanner::{GpuProcess, GpuResult, NvidiaScanner},
    model::{Bytes, Degree, Percent, safe_div},
    sensor_table::{self, HwmonRule, MAX_VALID_C, MIN_VALID_C, TargetRules},
};

pub type CpuRamResult<T = ()> = Result<T, sysinfo::Error>;

const HWMON_DIR: &str = "/sys/class/hwmon";
const THERMAL_DIR: &str = "/sys/class/thermal";

pub struct SystemScanner {
    system: sysinfo::System,
    /// Sensor files found once at startup (millidegrees C).
    cpu_sensors: Vec<PathBuf>,
    ram_sensors: Vec<PathBuf>,
}
impl SystemScanner {
    pub fn new() -> CpuRamResult<Self> {
        Ok(Self {
            system: sysinfo::System::new()?,
            cpu_sensors: find_sensors(&sensor_table::CPU),
            ram_sensors: find_sensors(&sensor_table::RAM),
        })
    }

    pub fn cpu_count(&self) -> usize {
        self.system.cpus().len()
    }

    pub fn scan(&mut self) {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.system
            .refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    }

    pub fn processes(
        &self,
        processes: &mut Vec<super::Process>,
        errors: &mut Vec<ScanError>,
        gpu_scanner: Option<&mut NvidiaScanner>,
    ) -> GpuResult {
        let gpu_process_store = gpu_process_store(errors, gpu_scanner);
        processes.extend(
            self.system
                .processes()
                .values()
                .map(|process| self.to_process(process, gpu_process_store)),
        );
        Ok(())
    }

    pub fn cpus(&self) -> impl Iterator<Item = Cpu<'_>> {
        self.system.cpus().iter().map(Cpu)
    }

    pub fn cpu_temprature(&self) -> Option<Degree> {
        read_max(&self.cpu_sensors)
    }

    pub fn ram_temprature(&self) -> Option<Degree> {
        read_max(&self.ram_sensors)
    }

    pub fn ram_usage(&self) -> Bytes {
        Bytes::new(self.system.used_memory())
    }
    pub fn ram_total(&self) -> Bytes {
        Bytes::new(self.system.total_memory())
    }

    fn to_process(
        &self,
        process: &sysinfo::Process,
        gpus: Option<&HashMap<sysinfo::Pid, GpuProcess>>,
    ) -> super::Process {
        let pid = process.pid();
        let name = process.exe().unwrap_or(Path::new(process.name())).into();

        let memory = Bytes::new(process.memory());
        let cpu_usage = Percent::new(safe_div(process.cpu_usage(), self.cpu_count() as f32));

        let gpu = gpus.map(|g| g.get(&pid)).flatten();
        let gpu_number = gpu.map(|g| g.gpu_numbers);
        let gpu_memory = gpu.map(|g| g.memory_usage).flatten();
        let gpu_usage = gpu.map(|g| g.gpu_usage).flatten();

        super::Process {
            gpu_usage,
            pid,
            name,
            memory,
            cpu_usage,
            gpu_number,
            gpu_memory,
        }
    }
}

pub struct Cpu<'a>(&'a sysinfo::Cpu);
impl<'a> Cpu<'a> {
    pub fn usage(&self) -> Percent {
        Percent::new(self.0.usage())
    }
}

/// Walks the rule table in priority order: the first rule that yields any sensor wins.
/// hwmon rules are tried first, then reliable thermal zones, then unreliable ones.
fn find_sensors(rules: &TargetRules) -> Vec<PathBuf> {
    for rule in rules.hwmon {
        let found = hwmon_sensors(rule);
        if !found.is_empty() {
            return found;
        }
    }

    let reliable = rules.thermal_zones.iter().filter(|z| z.reliable);
    let unreliable = rules.thermal_zones.iter().filter(|z| !z.reliable);
    for zone in reliable.chain(unreliable) {
        let found = thermal_zone_sensors(zone.zone_type);
        if !found.is_empty() {
            return found;
        }
    }

    Vec::new()
}

/// Every hwmon device whose `name` equals the rule's driver contributes its selected inputs.
fn hwmon_sensors(rule: &HwmonRule) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in read_dir_paths(HWMON_DIR) {
        if read_trimmed(&dir.join("name")).as_deref() != Some(rule.driver) {
            continue;
        }
        found.extend(select_inputs(labeled_inputs(&dir), rule));
    }
    found
}

/// All `tempN_input` files in a hwmon dir, paired with their `tempN_label` (empty if none).
fn labeled_inputs(dir: &Path) -> Vec<(String, PathBuf)> {
    read_dir_paths(dir)
        .into_iter()
        .filter_map(|path| {
            let file = path.file_name()?.to_str()?;
            let id = file.strip_suffix("_input")?;
            if !id.starts_with("temp") {
                return None;
            }
            let label = read_trimmed(&dir.join(format!("{id}_label"))).unwrap_or_default();
            Some((label, path))
        })
        .collect()
}

/// Applies the rule's `preferred` then `fallback` label prefixes.
/// With both lists empty, every input is used (unlabeled sensors such as spd5118).
fn select_inputs(inputs: Vec<(String, PathBuf)>, rule: &HwmonRule) -> Vec<PathBuf> {
    if rule.preferred.is_empty() && rule.fallback.is_empty() {
        return inputs.into_iter().map(|(_, path)| path).collect();
    }

    for prefixes in [rule.preferred, rule.fallback] {
        for prefix in prefixes {
            let matched: Vec<PathBuf> = inputs
                .iter()
                .filter(|(label, _)| label.starts_with(prefix))
                .map(|(_, path)| path.clone())
                .collect();
            if !matched.is_empty() {
                return matched;
            }
        }
    }

    Vec::new()
}

fn thermal_zone_sensors(zone_type: &str) -> Vec<PathBuf> {
    read_dir_paths(THERMAL_DIR)
        .into_iter()
        .filter(|dir| read_trimmed(&dir.join("type")).as_deref() == Some(zone_type))
        .map(|dir| dir.join("temp"))
        .filter(|path| path.exists())
        .collect()
}

fn gpu_process_store<'a>(
    errors: &mut Vec<ScanError>,
    gpu_scanner: Option<&'a mut NvidiaScanner>,
) -> Option<&'a HashMap<sysinfo::Pid, GpuProcess>> {
    match gpu_scanner?.gpu_processes(errors) {
        Ok(store) => Some(store),
        Err(err) => {
            errors.push(err.into());
            None
        }
    }
}

/// Reads every sensor, drops unreadable or out-of-range values, returns the hottest in °C.
fn read_max(paths: &[PathBuf]) -> Option<Degree> {
    paths
        .iter()
        .filter_map(|path| read_trimmed(path)?.parse::<f32>().ok())
        .map(|millidegrees| millidegrees / 1000.0)
        .filter(|celsius| (MIN_VALID_C..=MAX_VALID_C).contains(celsius))
        .reduce(f32::max)
        .map(Degree::new)
}

fn read_dir_paths(dir: impl AsRef<Path>) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

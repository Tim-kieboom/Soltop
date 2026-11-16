use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    ScanError,
    gpu_scanner::{GpuProcess, GpuResult, NvidiaScanner},
    model::{
        Bytes, CpuCache, CpuStatic, Degree, LoadAverage, Mhz, Percent, ProcessState, SystemStatic,
        safe_div,
    },
    sensor_table::{self, HwmonRule, MAX_VALID_C, MIN_VALID_C, TargetRules},
    sysfs::{read_dir_paths, read_trimmed},
};

pub type CpuRamResult<T = ()> = Result<T, sysinfo::Error>;

const HWMON_DIR: &str = "/sys/class/hwmon";
const THERMAL_DIR: &str = "/sys/class/thermal";
const CPU0_DIR: &str = "/sys/devices/system/cpu/cpu0";
const MEMINFO: &str = "/proc/meminfo";
/// Longer command lines are cut off; some (browsers, Java) run to many kilobytes.
const MAX_COMMAND_CHARS: usize = 400;

/// CPU, memory, process and host facts, read through sysinfo plus the sysfs and /proc files
/// that sysinfo does not cover (temperatures, cache sizes, page cache).
pub struct SystemScanner {
    system: sysinfo::System,
    /// Sensor files found once at startup (millidegrees C).
    cpu_sensors: Vec<PathBuf>,
    ram_sensors: Vec<PathBuf>,
    users: Option<sysinfo::Users>,
    last_scan: Option<Instant>,
    /// Seconds between the last two scans, used to turn per-refresh disk bytes into rates.
    scan_seconds: f64,
}
impl SystemScanner {
    /// Finds the temperature sensors once. Fails only when sysinfo itself cannot start.
    pub fn new() -> CpuRamResult<Self> {
        Ok(Self {
            system: sysinfo::System::new()?,
            cpu_sensors: find_sensors(&sensor_table::CPU),
            ram_sensors: find_sensors(&sensor_table::RAM),
            users: sysinfo::Users::new_with_refreshed_list().ok(),
            last_scan: None,
            scan_seconds: 1.0,
        })
    }

    /// Number of logical cores; 0 until the first [`scan`](Self::scan).
    pub fn cpu_count(&self) -> usize {
        self.system.cpus().len()
    }

    /// Refreshes CPU, memory and process data. Call once per tick before reading anything.
    ///
    /// CPU and per-process usage are deltas, so they only mean something from the second
    /// call on.
    pub fn scan(&mut self) {
        self.system.refresh_cpu_usage();
        self.system.refresh_cpu_frequency();
        self.system.refresh_memory();

        let now = Instant::now();
        if let Some(last) = self.last_scan {
            self.scan_seconds = now.duration_since(last).as_secs_f64().max(0.001);
        }
        self.last_scan = Some(now);

        // The default refresh leaves out the command line and the owner.
        let kind = sysinfo::ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu()
            .with_disk_usage()
            .with_exe(sysinfo::UpdateKind::OnlyIfNotSet)
            .with_cmd(sysinfo::UpdateKind::OnlyIfNotSet)
            .with_user(sysinfo::UpdateKind::OnlyIfNotSet)
            .with_tasks();
        self.system
            .refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, kind);
    }

    /// Appends every process (threads excluded) to `processes`, joined with its GPU usage when
    /// a GPU scanner is available. GPU failures go to `errors` and do not stop the process list.
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
                // Userland threads are listed as processes and repeat their parent's
                // full memory, so counting them multiplies RAM by the thread count.
                .filter(|process| process.thread_kind() != Some(sysinfo::ThreadKind::Userland))
                .map(|process| self.to_process(process, gpu_process_store)),
        );
        Ok(())
    }

    /// The logical cores, in order.
    pub fn cpus(&self) -> impl Iterator<Item = Cpu<'_>> {
        self.system.cpus().iter().map(Cpu)
    }

    /// Host name, OS, kernel and architecture. Fields sysinfo cannot read stay empty.
    pub fn system_static(&self) -> SystemStatic {
        use sysinfo::System;
        let os = System::long_os_version()
            .or_else(|_| System::name())
            .unwrap_or_default();

        SystemStatic {
            hostname: System::host_name().unwrap_or_default().into(),
            os: os.into(),
            kernel: System::kernel_version().unwrap_or_default().into(),
            architecture: System::cpu_arch().into(),
        }
    }

    /// The 1, 5 and 15 minute load averages.
    pub fn load_average(&self) -> Option<LoadAverage> {
        let load = sysinfo::System::load_average().ok()?;
        Some(LoadAverage {
            one: load.one as f32,
            five: load.five as f32,
            fifteen: load.fifteen as f32,
        })
    }

    /// Average current clock over the cores that report one.
    pub fn cpu_frequency(&self) -> Option<Mhz> {
        let clocks: Vec<u64> = self
            .system
            .cpus()
            .iter()
            .map(|cpu| cpu.frequency())
            .filter(|&mhz| mhz > 0)
            .collect();

        if clocks.is_empty() {
            return None;
        }
        Some(Mhz::new(clocks.iter().sum::<u64>() / clocks.len() as u64))
    }

    /// Time since boot.
    pub fn uptime(&self) -> Option<std::time::Duration> {
        sysinfo::System::uptime().ok().map(std::time::Duration::from_secs)
    }

    /// Facts that don't change at runtime. Call after the first scan so sysinfo has the CPU list.
    pub fn cpu_static(&self) -> CpuStatic {
        let cpus = self.system.cpus();
        let first = cpus.first();
        let cpu_dir = Path::new(CPU0_DIR);

        CpuStatic {
            name: first.map_or("", |cpu| cpu.brand().trim()).into(),
            vendor: first.map_or("", |cpu| cpu.vendor_id().trim()).into(),
            physical_cores: sysinfo::System::physical_core_count().ok(),
            logical_cores: cpus.len(),
            cache: read_cpu_cache(cpu_dir),
            base_clock: read_khz_as_mhz(&cpu_dir.join("cpufreq/base_frequency")),
            max_clock: read_khz_as_mhz(&cpu_dir.join("cpufreq/cpuinfo_max_freq")),
        }
    }

    /// Hottest CPU sensor in °C, or `None` when no known sensor exists or all readings are invalid.
    pub fn cpu_temprature(&self) -> Option<Degree> {
        read_max(&self.cpu_sensors)
    }

    /// Hottest DIMM sensor in °C; most boards have none.
    pub fn ram_temprature(&self) -> Option<Degree> {
        read_max(&self.ram_sensors)
    }

    /// Memory in use.
    pub fn ram_usage(&self) -> Bytes {
        Bytes::new(self.system.used_memory())
    }
    /// Installed memory.
    pub fn ram_total(&self) -> Bytes {
        Bytes::new(self.system.total_memory())
    }
    /// Memory available to programs without swapping.
    pub fn ram_available(&self) -> Bytes {
        Bytes::new(self.system.available_memory())
    }
    /// Swap in use.
    pub fn swap_used(&self) -> Bytes {
        Bytes::new(self.system.used_swap())
    }
    /// Configured swap; zero when there is none.
    pub fn swap_total(&self) -> Bytes {
        Bytes::new(self.system.total_swap())
    }

    /// Page cache plus buffers (what Task Manager calls "Cached"); `None` if /proc/meminfo is unreadable.
    pub fn ram_cached(&self) -> Option<Bytes> {
        let meminfo = fs::read_to_string(MEMINFO).ok()?;
        let cached = meminfo_kib(&meminfo, "Cached:")?;
        let buffers = meminfo_kib(&meminfo, "Buffers:").unwrap_or(0);
        Some(Bytes::new((cached + buffers) * 1024))
    }

    /// The owner's name, or the numeric uid when no name is known.
    fn user_name(&self, process: &sysinfo::Process) -> Option<Box<str>> {
        let uid = process.user_id()?;
        let name = self
            .users
            .as_ref()
            .and_then(|users| users.get_user_by_id(uid))
            .map(|user| user.name().to_string());
        Some(name.unwrap_or_else(|| (**uid).to_string()).into())
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

        let disk = process.disk_usage();
        let per_second = |bytes: u64| Bytes::new((bytes as f64 / self.scan_seconds) as u64);

        super::Process {
            user: self.user_name(process),
            command: command_line(process),
            parent: process.parent(),
            state: state_of(process.status()),
            run_time: Duration::from_secs(process.run_time()),
            threads: process.tasks().map(|tasks| tasks.len()),
            disk_read: per_second(disk.read_bytes),
            disk_write: per_second(disk.written_bytes),
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

/// The command line as one string, cut off at [`MAX_COMMAND_CHARS`] characters.
fn command_line(process: &sysinfo::Process) -> Box<str> {
    let joined = process
        .cmd()
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    joined.chars().take(MAX_COMMAND_CHARS).collect::<String>().into()
}

/// Maps sysinfo's process status onto the smaller set the UI shows.
fn state_of(status: sysinfo::ProcessStatus) -> ProcessState {
    use sysinfo::ProcessStatus as S;
    match status {
        S::Run => ProcessState::Running,
        S::Sleep => ProcessState::Sleeping,
        S::UninterruptibleDiskSleep => ProcessState::DiskSleep,
        S::Idle => ProcessState::Idle,
        S::Stop | S::Tracing => ProcessState::Stopped,
        S::Zombie => ProcessState::Zombie,
        _ => ProcessState::Other,
    }
}

/// One logical core.
pub struct Cpu<'a>(&'a sysinfo::Cpu);
impl<'a> Cpu<'a> {
    /// Load since the previous scan.
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

/// Reads `cache/index*/{level,type,size}` of one CPU.
fn read_cpu_cache(cpu_dir: &Path) -> CpuCache {
    let mut cache = CpuCache::default();

    for dir in read_dir_paths(cpu_dir.join("cache")) {
        let is_index = dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("index"));
        if !is_index {
            continue;
        }

        let level = read_trimmed(&dir.join("level")).and_then(|v| v.parse::<u8>().ok());
        let kind = read_trimmed(&dir.join("type"));
        let size = read_trimmed(&dir.join("size"))
            .and_then(|v| parse_cache_size(&v))
            .map(Bytes::new);

        match (level, kind.as_deref()) {
            (Some(1), Some("Data")) => cache.l1_data = size,
            (Some(1), Some("Instruction")) => cache.l1_instruction = size,
            (Some(2), _) => cache.l2 = size,
            (Some(3), _) => cache.l3 = size,
            _ => {}
        }
    }
    cache
}

/// Parses sysfs sizes such as "32K", "1024K" or "32M" into bytes.
fn parse_cache_size(text: &str) -> Option<u64> {
    let text = text.trim();
    if let Some(kib) = text.strip_suffix('K') {
        return kib.parse::<u64>().ok().map(|n| n * 1024);
    }
    if let Some(mib) = text.strip_suffix('M') {
        return mib.parse::<u64>().ok().map(|n| n * 1024 * 1024);
    }
    text.parse().ok()
}

/// Value in kiB of a `/proc/meminfo` line such as `Cached:   1234 kB`.
fn meminfo_kib(meminfo: &str, key: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix(key))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn read_khz_as_mhz(path: &Path) -> Option<Mhz> {
    read_trimmed(path)?
        .parse::<u64>()
        .ok()
        .map(|khz| Mhz::new(khz / 1000))
}

/// The per-pid GPU data for this scan, or `None` (with the error recorded) when it failed.
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::sysfs::test_util::temp_dir;

    fn rule(preferred: &'static [&'static str], fallback: &'static [&'static str]) -> HwmonRule {
        HwmonRule {
            driver: "test",
            preferred,
            fallback,
        }
    }

    fn inputs(labels: &[&str]) -> Vec<(String, PathBuf)> {
        labels
            .iter()
            .enumerate()
            .map(|(i, label)| (label.to_string(), PathBuf::from(format!("/sensor{i}"))))
            .collect()
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|name| PathBuf::from(*name)).collect()
    }

    #[test]
    fn parse_cache_size_understands_kib_and_mib() {
        assert_eq!(parse_cache_size("32K"), Some(32 * 1024));
        assert_eq!(parse_cache_size("32M"), Some(32 * 1024 * 1024));
    }

    #[test]
    fn parse_cache_size_accepts_plain_bytes_and_whitespace() {
        assert_eq!(parse_cache_size("512"), Some(512));
        assert_eq!(parse_cache_size(" 8K\n"), Some(8 * 1024));
    }

    #[test]
    fn parse_cache_size_rejects_garbage() {
        assert_eq!(parse_cache_size(""), None);
        assert_eq!(parse_cache_size("K"), None);
        assert_eq!(parse_cache_size("lots"), None);
    }

    #[test]
    fn meminfo_kib_reads_the_exact_key() {
        let meminfo = "MemTotal:       32768 kB\nSwapCached:        99 kB\nCached:          1024 kB\n";
        assert_eq!(meminfo_kib(meminfo, "Cached:"), Some(1024));
        assert_eq!(meminfo_kib(meminfo, "MemTotal:"), Some(32768));
    }

    #[test]
    fn meminfo_kib_of_a_missing_or_garbled_key_is_none() {
        assert_eq!(meminfo_kib("MemTotal: 1 kB\n", "Buffers:"), None);
        assert_eq!(meminfo_kib("Cached: lots kB\n", "Cached:"), None);
        assert_eq!(meminfo_kib("", "Cached:"), None);
    }

    #[test]
    fn process_status_maps_onto_the_ui_states() {
        use sysinfo::ProcessStatus as S;
        assert_eq!(state_of(S::Run), ProcessState::Running);
        assert_eq!(state_of(S::Sleep), ProcessState::Sleeping);
        assert_eq!(state_of(S::UninterruptibleDiskSleep), ProcessState::DiskSleep);
        assert_eq!(state_of(S::Idle), ProcessState::Idle);
        assert_eq!(state_of(S::Stop), ProcessState::Stopped);
        assert_eq!(state_of(S::Tracing), ProcessState::Stopped);
        assert_eq!(state_of(S::Zombie), ProcessState::Zombie);
        assert_eq!(state_of(S::Dead), ProcessState::Other);
    }

    #[test]
    fn select_inputs_prefers_the_preferred_label() {
        let selected = select_inputs(
            inputs(&["Core 0", "Package id 0", "Core 1"]),
            &rule(&["Package id"], &["Core"]),
        );
        assert_eq!(selected, paths(&["/sensor1"]));
    }

    #[test]
    fn select_inputs_falls_back_and_keeps_every_match() {
        let selected = select_inputs(
            inputs(&["Core 0", "Core 1", "Fan"]),
            &rule(&["Package id"], &["Core"]),
        );
        assert_eq!(selected, paths(&["/sensor0", "/sensor1"]));
    }

    #[test]
    fn select_inputs_tries_preferred_prefixes_in_order() {
        let selected = select_inputs(
            inputs(&["Tctl", "Tdie"]),
            &rule(&["Tdie", "Tctl"], &[]),
        );
        assert_eq!(selected, paths(&["/sensor1"]));
    }

    #[test]
    fn select_inputs_without_a_match_is_empty() {
        let selected = select_inputs(inputs(&["Fan"]), &rule(&["Package id"], &["Core"]));
        assert!(selected.is_empty());
    }

    #[test]
    fn select_inputs_with_no_prefixes_takes_everything() {
        let selected = select_inputs(inputs(&["", "", ""]), &rule(&[], &[]));
        assert_eq!(selected, paths(&["/sensor0", "/sensor1", "/sensor2"]));
    }

    #[test]
    fn labeled_inputs_pairs_inputs_with_their_labels() {
        let dir = temp_dir("hwmon");
        fs::write(dir.join("temp1_input"), "45000\n").unwrap();
        fs::write(dir.join("temp1_label"), "Package id 0\n").unwrap();
        fs::write(dir.join("temp2_input"), "50000\n").unwrap();
        // not temperature inputs: wrong prefix, wrong suffix, a label on its own
        fs::write(dir.join("fan1_input"), "900\n").unwrap();
        fs::write(dir.join("temp1_max"), "100000\n").unwrap();

        // read_dir order is unspecified; sorting puts the unlabeled input ("") first
        let mut found = labeled_inputs(&dir);
        found.sort();
        assert_eq!(
            found,
            vec![
                (String::new(), dir.join("temp2_input")),
                ("Package id 0".to_string(), dir.join("temp1_input")),
            ]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_max_returns_the_hottest_valid_reading() {
        let dir = temp_dir("read_max");
        let files: Vec<PathBuf> = ["45000", "61500", "garbage", "151000", "-41000"]
            .iter()
            .enumerate()
            .map(|(i, content)| {
                let path = dir.join(format!("t{i}"));
                fs::write(&path, content).unwrap();
                path
            })
            .collect();

        // 151 °C and -41 °C are outside the valid window, "garbage" does not parse
        assert_eq!(read_max(&files), Some(Degree::new(61.5)));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_max_accepts_the_window_edges() {
        let dir = temp_dir("edges");
        let hot = dir.join("hot");
        fs::write(&hot, "150000").unwrap();

        assert_eq!(read_max(&[hot]), Some(Degree::new(150.0)));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_max_of_nothing_usable_is_none() {
        let dir = temp_dir("none");
        assert_eq!(read_max(&[]), None);
        assert_eq!(read_max(&[dir.join("missing")]), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_cpu_cache_sorts_each_cache_into_its_slot() {
        let dir = temp_dir("cache");
        for (index, level, kind, size) in [
            (0, "1", "Data", "32K"),
            (1, "1", "Instruction", "64K"),
            (2, "2", "Unified", "1024K"),
            (3, "3", "Unified", "32768K"),
        ] {
            let cache = dir.join(format!("cache/index{index}"));
            fs::create_dir_all(&cache).unwrap();
            fs::write(cache.join("level"), format!("{level}\n")).unwrap();
            fs::write(cache.join("type"), format!("{kind}\n")).unwrap();
            fs::write(cache.join("size"), format!("{size}\n")).unwrap();
        }
        // not a cache index, must be ignored
        fs::create_dir_all(dir.join("cache/power")).unwrap();

        let cache = read_cpu_cache(&dir);
        assert_eq!(cache.l1_data, Some(Bytes::new(32 * 1024)));
        assert_eq!(cache.l1_instruction, Some(Bytes::new(64 * 1024)));
        assert_eq!(cache.l2, Some(Bytes::new(1024 * 1024)));
        assert_eq!(cache.l3, Some(Bytes::new(32 * 1024 * 1024)));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_cpu_cache_without_sysfs_files_is_all_none() {
        let dir = temp_dir("nocache");

        let cache = read_cpu_cache(&dir);
        assert_eq!(cache.l1_data, None);
        assert_eq!(cache.l2, None);
        assert_eq!(cache.l3, None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_khz_as_mhz_converts_the_unit() {
        let dir = temp_dir("khz");
        let file = dir.join("freq");
        fs::write(&file, "3400000\n").unwrap();

        assert_eq!(read_khz_as_mhz(&file), Some(Mhz::new(3400)));
        assert_eq!(read_khz_as_mhz(&dir.join("missing")), None);
        fs::write(&file, "fast").unwrap();
        assert_eq!(read_khz_as_mhz(&file), None);
        fs::remove_dir_all(dir).unwrap();
    }
}

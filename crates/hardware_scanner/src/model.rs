//! Plain data types shared by the scanners and the UI: unit newtypes, live readings that are
//! rebuilt on every scan, and static facts that are collected once.

use std::{ffi::OsStr, fmt::Debug, net::IpAddr, path::Path, time::Duration};

use nvml_wrapper::error::NvmlError;
use thiserror::Error;

/// Process id, re-exported so users of [`Process`] can build and compare pids without
/// depending on sysinfo themselves.
pub use sysinfo::Pid;

macro_rules! newtype {
    ($(#[$meta:meta])* $vis:vis struct $name:ident($ty:ty) { $as:ident }) => {
        $(#[$meta])*
        $vis struct $name($ty);

        impl $name {
            #[allow(unused)]
            $vis fn new(v: $ty) -> Self {
                Self(v)
            }

            #[allow(unused)]
            $vis fn $as(&self) -> $ty {
                self.0
            }
        }

        impl AsF64 for $name {
            fn as_f64(&self) -> f64 {
                self.0 as f64
            }
        }
    };
}

newtype!(
    /// A size in bytes, or a rate in bytes per second.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Bytes(u64) { as_u64 }
);
newtype!(
    /// A size in decimal gigabytes (10^9 bytes), as produced by [`Bytes::to_gb`].
    #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
    pub struct Gb(f32) { as_f32 }
);
newtype!(
    /// A temperature in degrees Celsius.
    #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
    pub struct Degree(f32) { as_f32 }
);
newtype!(
    /// A percentage on a 0.0 to 100.0 scale (not 0.0 to 1.0).
    #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
    pub struct Percent(f32) { as_f32 }
);
newtype!(
    /// A clock frequency in megahertz.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Mhz(u64) { as_u64 }
);
newtype!(
    /// Power in watts.
    #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
    pub struct Watts(f32) { as_f32 }
);

/// Converts a unit type to `f64`, so histories and plots can treat bytes, percentages and
/// gigabytes alike.
pub trait AsF64 {
    fn as_f64(&self) -> f64;
}

impl Bytes {
    /// Converts to decimal gigabytes (10^9 bytes), rounded to `decimal` digits.
    ///
    /// The math runs in `f32`, so the last digits of very large values are approximate.
    pub fn to_gb(self, decimal: u8) -> Gb {
        let deci = 10_f32.powi(decimal as i32);
        let gb = ((self.0 as f32 / 1000000000.0) * deci).round() / deci;
        Gb(gb)
    }
}

/// Result of a scanner operation.
pub type ScanResult<T = ()> = Result<T, ScanError>;

/// Why a scan step failed. Scans report these in [`crate::HardwareInfo::errors`] and carry on
/// instead of aborting, so one broken sensor never blanks the whole UI.
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("sysinfo error: {0}")]
    Sysinfo(#[from] sysinfo::Error),
    #[error("gpu error: {0}")]
    Gpu(#[from] NvmlError),
}

/// Live CPU numbers, rebuilt on every scan.
#[derive(Debug, Default)]
pub struct CpuInfo {
    /// Load per logical core, in core order.
    pub core_usage: Vec<Percent>,
    /// Hottest CPU sensor; `None` when no known sensor was found.
    pub temperature: Option<Degree>,
    /// Average load over all cores.
    pub total_usage: Option<Percent>,
    /// Average current clock over all cores.
    pub frequency: Option<Mhz>,
    /// Time since boot.
    pub uptime: Option<Duration>,
}

/// Hardware facts that never change while the program runs.
/// Collected once at startup and sent along with the first scan.
#[derive(Debug, Default, Clone)]
pub struct StaticInfo {
    pub cpu: CpuStatic,
    pub gpu_driver: Option<Box<str>>,
    /// One entry per GPU index, `None` when that GPU could not be queried.
    pub gpus: Vec<Option<GpuStatic>>,
    pub disks: Vec<DiskStatic>,
    pub nets: Vec<NetStatic>,
    pub system: SystemStatic,
}

/// Host and operating system facts.
#[derive(Debug, Default, Clone)]
pub struct SystemStatic {
    pub hostname: Box<str>,
    /// Distribution and version, e.g. "Linux NixOS 25.11".
    pub os: Box<str>,
    pub kernel: Box<str>,
    pub architecture: Box<str>,
}

/// Averages of the number of runnable tasks over the last 1, 5 and 15 minutes.
#[derive(Debug, Default, Clone, Copy)]
pub struct LoadAverage {
    pub one: f32,
    pub five: f32,
    pub fifteen: f32,
}

/// Charge state of a battery, as the kernel reports it in `power_supply/*/status`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BatteryState {
    Charging,
    Discharging,
    Full,
    NotCharging,
    #[default]
    Unknown,
}
impl BatteryState {
    /// Human-readable name for the state.
    pub fn label(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "Discharging",
            Self::Full => "Full",
            Self::NotCharging => "Not charging",
            Self::Unknown => "Unknown",
        }
    }
}

/// One battery's current state.
#[derive(Debug, Clone, Copy)]
pub struct BatteryInfo {
    /// Charge level.
    pub percent: Percent,
    pub state: BatteryState,
    /// Current charge or discharge rate.
    pub power: Option<Watts>,
    /// Time until empty (discharging) or full (charging).
    pub time_remaining: Option<Duration>,
}

/// Machine-wide live values that are not tied to one device.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemInfo {
    /// `None` when the load average could not be read.
    pub load: Option<LoadAverage>,
    /// `None` on machines without a battery.
    pub battery: Option<BatteryInfo>,
}

/// What kind of drive a disk is, judged from its name and the kernel's rotational flag.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DiskKind {
    Nvme,
    Ssd,
    Hdd,
    #[default]
    Unknown,
}

/// A physical disk (not a partition), identified by its kernel name such as `nvme0n1`.
#[derive(Debug, Default, Clone)]
pub struct DiskStatic {
    pub id: Box<str>,
    pub model: Box<str>,
    pub size: Bytes,
    pub kind: DiskKind,
}

/// Facts about one GPU that never change while the program runs.
#[derive(Debug, Default, Clone)]
pub struct GpuStatic {
    pub name: Box<str>,
    /// Enforced power limit.
    pub power_limit: Option<Watts>,
    /// Highest PCIe generation the GPU and slot support.
    pub pcie_max_gen: Option<u32>,
    /// Widest PCIe link (x1, x4, x8, x16) the GPU and slot support.
    pub pcie_max_width: Option<u32>,
}

/// CPU facts that never change while the program runs.
#[derive(Debug, Default, Clone)]
pub struct CpuStatic {
    /// Marketing name, e.g. "AMD Ryzen 9 7950X". Empty when unknown.
    pub name: Box<str>,
    /// Vendor id, e.g. "AuthenticAMD".
    pub vendor: Box<str>,
    /// Physical core count; `None` when it could not be determined.
    pub physical_cores: Option<usize>,
    /// Logical core count (hardware threads).
    pub logical_cores: usize,
    pub cache: CpuCache,
    /// Base clock; only some drivers (e.g. `intel_pstate`) report it.
    pub base_clock: Option<Mhz>,
    /// Highest boost clock.
    pub max_clock: Option<Mhz>,
}

/// L1 is per core, L2 and L3 as reported for cpu0 (L3 is usually shared).
#[derive(Debug, Default, Clone, Copy)]
pub struct CpuCache {
    pub l1_data: Option<Bytes>,
    pub l1_instruction: Option<Bytes>,
    pub l2: Option<Bytes>,
    pub l3: Option<Bytes>,
}

/// Whether an interface is wired or wireless.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum NetKind {
    #[default]
    Ethernet,
    Wifi,
}

/// A physical network interface (not virtual ones such as docker0 or veth), e.g. `enp5s0`.
#[derive(Debug, Default, Clone)]
pub struct NetStatic {
    /// Kernel interface name.
    pub id: Box<str>,
    pub kind: NetKind,
    /// MAC address, `None` when the kernel reports all zeros.
    pub mac: Option<Box<str>>,
}

/// Live numbers for the interface with the same `id` as a [`NetStatic`].
#[derive(Debug, Default, Clone)]
pub struct NetInfo {
    /// Kernel interface name, matching [`NetStatic::id`].
    pub id: Box<str>,
    /// Whether the interface is up and can pass packets.
    pub connected: bool,
    /// Bytes per second since the previous scan; `None` on the very first scan.
    pub receive_rate: Option<Bytes>,
    /// Bytes per second since the previous scan.
    pub send_rate: Option<Bytes>,
    /// Negotiated link speed in bits per second, when the driver reports one.
    pub link_speed: Option<u64>,
    /// IP addresses, IPv4 first.
    pub addresses: Vec<IpAddr>,
    /// Wi-Fi signal strength in dBm.
    pub signal_dbm: Option<i32>,
}

/// Live numbers for the disk with the same `id` as a [`DiskStatic`].
#[derive(Debug, Default, Clone)]
pub struct DiskInfo {
    /// Kernel device name, matching [`DiskStatic::id`].
    pub id: Box<str>,
    /// Bytes per second since the previous scan; `None` on the very first scan.
    pub read_rate: Option<Bytes>,
    /// Bytes per second since the previous scan.
    pub write_rate: Option<Bytes>,
    /// Share of the last interval during which the disk had requests in flight.
    pub active_time: Option<Percent>,
    /// Average time per completed request since the previous scan, in milliseconds.
    pub response_ms: Option<f32>,
    /// Drive temperature when the kernel exposes a sensor for it.
    pub temperature: Option<Degree>,
    /// Mounted filesystems on this disk.
    pub volumes: Vec<Volume>,
}

/// A mounted filesystem living on a disk.
#[derive(Debug, Clone)]
pub struct Volume {
    /// Where the filesystem is mounted.
    pub mount: Box<Path>,
    /// Filesystem type, e.g. "ext4".
    pub file_system: Box<str>,
    pub total: Bytes,
    pub used: Bytes,
}

/// Live memory numbers, rebuilt on every scan.
#[derive(Debug, Default)]
pub struct RamInfo {
    /// Memory in use (total minus available).
    pub used: Option<Bytes>,
    /// Installed memory.
    pub total: Option<Bytes>,
    /// Memory that programs could use without swapping.
    pub available: Option<Bytes>,
    /// Page cache plus buffers, from /proc/meminfo.
    pub cached: Option<Bytes>,
    pub swap_used: Option<Bytes>,
    /// Zero when no swap is configured.
    pub swap_total: Option<Bytes>,
    /// DIMM temperature; rare, most boards have no sensor.
    pub temperature: Option<Degree>,
}

/// Live numbers of one GPU, rebuilt on every scan. Every field except the core ones
/// (load, memory, temperature) is optional because GPUs differ in what they support.
#[derive(Debug, Default)]
pub struct GpuInfo {
    /// Share of time the GPU was busy.
    pub gpu: Option<Percent>,
    /// Share of video memory in use.
    pub memory_usage: Option<Percent>,
    pub memory_used: Option<Bytes>,
    pub memory_total: Option<Bytes>,
    pub temperature: Option<Degree>,
    pub core_clock: Option<Mhz>,
    pub memory_clock: Option<Mhz>,
    /// Current power draw.
    pub power: Option<Watts>,
    /// Fan speed as a share of its maximum; `None` on GPUs without a controllable fan
    /// (many laptops).
    pub fan: Option<Percent>,
    pub encoder: Option<Percent>,
    pub decoder: Option<Percent>,
    pub pcie_gen: Option<u32>,
    pub pcie_width: Option<u32>,
}

/// One running process. Userland threads are not listed, so memory is never counted twice.
#[derive(Debug)]
pub struct Process {
    pub pid: sysinfo::Pid,
    /// Resident memory (RSS).
    pub memory: Bytes,
    /// Path of the executable, or just its name when the path is unreadable.
    pub name: Box<Path>,
    /// Share of the whole machine's CPU capacity: 100 means every core is busy with this process.
    pub cpu_usage: Percent,
    /// Video memory held; `None` when the process uses no GPU.
    pub gpu_memory: Option<Bytes>,
    /// Load of the busiest GPU this process runs on; `None` without a sample this scan.
    pub gpu_usage: Option<Percent>,
    /// The GPUs this process uses.
    pub gpu_number: Option<GpuNumbers>,
    /// Owner of the process; `None` when it could not be read.
    pub user: Option<Box<str>>,
    /// Full command line (arguments joined by spaces), cut off at a fixed length.
    pub command: Box<str>,
    pub parent: Option<sysinfo::Pid>,
    pub state: ProcessState,
    /// How long the process has been running.
    pub run_time: Duration,
    /// Number of threads; `None` when unknown.
    pub threads: Option<usize>,
    /// Bytes per second since the previous scan.
    pub disk_read: Bytes,
    pub disk_write: Bytes,
}

/// Scheduling state of a process.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Sleeping,
    DiskSleep,
    Idle,
    Stopped,
    Zombie,
    #[default]
    Other,
}
impl ProcessState {
    /// Human-readable name for the state.
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Sleeping => "Sleeping",
            Self::DiskSleep => "Waiting for disk",
            Self::Idle => "Idle",
            Self::Stopped => "Stopped",
            Self::Zombie => "Zombie",
            Self::Other => "Other",
        }
    }
}

impl Process {
    /// Combined disk read and write rate in bytes per second.
    pub fn disk_rate(&self) -> u64 {
        self.disk_read.as_u64() + self.disk_write.as_u64()
    }

    /// The executable's file name, or the whole path if it has none.
    pub fn file_name(&self) -> &OsStr {
        self.name.file_name().unwrap_or(self.name.as_os_str())
    }
}

/// A set of GPU indices stored as bits: bit `n` is set when GPU `n` is in the set.
///
/// Indices above 63 all map to bit 63, which no real machine reaches.
#[derive(Clone, Copy, Default)]
pub struct GpuNumbers(u64);
impl GpuNumbers {
    /// The set containing only GPU `i`.
    pub fn from_index(i: usize) -> Self {
        // i expect no one insane eneugh to have 65 gpu's if so gpu 65 notes as gpu 64
        Self(1 << i.min(63))
    }

    /// Builds a set from its raw bits.
    pub fn from_raw_u64(v: u64) -> Self {
        Self(v)
    }

    /// All GPUs that are in either set.
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether the two sets share at least one GPU.
    pub fn has_overlap(&self, other: Self) -> bool {
        for i in other.iter() {
            if self.contains(i) {
                return true;
            }
        }

        false
    }

    /// Whether GPU `index` is in the set.
    pub fn contains(&self, index: usize) -> bool {
        self.0 & (1 << index.min(63)) != 0
    }

    /// The raw bits.
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// The GPU indices in the set, lowest first.
    pub fn iter(self) -> impl Iterator<Item = usize> {
        let mut value = self.0;
        std::iter::from_fn(move || {
            if value == 0 {
                None
            } else {
                let idx = value.trailing_zeros() as usize;
                value &= value - 1;
                Some(idx)
            }
        })
    }
}
impl Debug for GpuNumbers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// `a / b`, or 0.0 when `b` is zero, so an empty core list never yields NaN.
pub fn safe_div(a: f32, b: f32) -> f32 {
    if b == 0.0 { 0.0 } else { a / b }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn process(name: &str, read: u64, write: u64) -> Process {
        Process {
            pid: sysinfo::Pid::from_u32(1),
            memory: Bytes::new(0),
            name: Path::new(name).into(),
            cpu_usage: Percent::new(0.0),
            gpu_memory: None,
            gpu_usage: None,
            gpu_number: None,
            user: None,
            command: "".into(),
            parent: None,
            state: ProcessState::default(),
            run_time: Duration::ZERO,
            threads: None,
            disk_read: Bytes::new(read),
            disk_write: Bytes::new(write),
        }
    }

    #[test]
    fn to_gb_uses_decimal_gigabytes() {
        assert_eq!(Bytes::new(1_500_000_000).to_gb(1), Gb::new(1.5));
        assert_eq!(Bytes::new(3_000_000_000).to_gb(0), Gb::new(3.0));
        assert_eq!(Bytes::new(0).to_gb(2), Gb::new(0.0));
    }

    #[test]
    fn to_gb_rounds_to_the_requested_digits() {
        assert_eq!(Bytes::new(1_234_567_890).to_gb(2), Gb::new(1.23));
        assert_eq!(Bytes::new(1_234_567_890).to_gb(1), Gb::new(1.2));
    }

    #[test]
    fn unit_types_convert_to_f64() {
        assert_eq!(Bytes::new(7).as_f64(), 7.0);
        assert_eq!(Percent::new(12.5).as_f64(), 12.5);
        assert_eq!(Gb::new(0.25).as_f64(), 0.25);
    }

    #[test]
    fn unit_types_default_to_zero() {
        assert_eq!(Bytes::default().as_u64(), 0);
        assert_eq!(Percent::default().as_f32(), 0.0);
        assert_eq!(Mhz::default().as_u64(), 0);
    }

    #[test]
    fn safe_div_never_divides_by_zero() {
        assert_eq!(safe_div(6.0, 3.0), 2.0);
        assert_eq!(safe_div(6.0, 0.0), 0.0);
        assert_eq!(safe_div(0.0, 0.0), 0.0);
    }

    #[test]
    fn gpu_numbers_from_index_sets_one_bit() {
        assert_eq!(GpuNumbers::from_index(0).as_u64(), 1);
        assert_eq!(GpuNumbers::from_index(3).as_u64(), 0b1000);
    }

    #[test]
    fn gpu_numbers_clamp_huge_indices_to_the_last_bit() {
        assert_eq!(GpuNumbers::from_index(70).as_u64(), 1 << 63);
        assert_eq!(GpuNumbers::from_index(63).as_u64(), 1 << 63);
    }

    #[test]
    fn gpu_numbers_union_and_contains() {
        let both = GpuNumbers::from_index(0).union(GpuNumbers::from_index(2));
        assert!(both.contains(0));
        assert!(!both.contains(1));
        assert!(both.contains(2));
        assert_eq!(both.as_u64(), 0b101);
    }

    #[test]
    fn gpu_numbers_overlap() {
        let a = GpuNumbers::from_raw_u64(0b011);
        assert!(a.has_overlap(GpuNumbers::from_raw_u64(0b010)));
        assert!(!a.has_overlap(GpuNumbers::from_raw_u64(0b100)));
        assert!(!a.has_overlap(GpuNumbers::default()));
    }

    #[test]
    fn gpu_numbers_iterate_lowest_first() {
        let numbers = GpuNumbers::from_raw_u64(0b1010_0001);
        assert_eq!(numbers.iter().collect::<Vec<_>>(), vec![0, 5, 7]);
        assert_eq!(GpuNumbers::default().iter().count(), 0);
    }

    #[test]
    fn gpu_numbers_debug_lists_the_indices() {
        assert_eq!(format!("{:?}", GpuNumbers::from_raw_u64(0b101)), "[0, 2]");
    }

    #[test]
    fn process_file_name_is_the_last_path_component() {
        assert_eq!(process("/usr/bin/firefox", 0, 0).file_name(), "firefox");
    }

    #[test]
    fn process_file_name_falls_back_to_the_whole_path() {
        // "/" has no file name component
        assert_eq!(process("/", 0, 0).file_name(), "/");
    }

    #[test]
    fn process_disk_rate_adds_read_and_write() {
        assert_eq!(process("/bin/x", 300, 200).disk_rate(), 500);
    }

    #[test]
    fn process_state_labels_are_distinct() {
        let all = [
            ProcessState::Running,
            ProcessState::Sleeping,
            ProcessState::DiskSleep,
            ProcessState::Idle,
            ProcessState::Stopped,
            ProcessState::Zombie,
            ProcessState::Other,
        ];
        let labels: HashSet<_> = all.iter().map(|state| state.label()).collect();
        assert_eq!(labels.len(), all.len());
        assert_eq!(ProcessState::default(), ProcessState::Other);
    }

    #[test]
    fn enum_defaults_are_the_unknown_variants() {
        assert_eq!(BatteryState::default(), BatteryState::Unknown);
        assert_eq!(DiskKind::default(), DiskKind::Unknown);
        assert_eq!(NetKind::default(), NetKind::Ethernet);
    }

    #[test]
    fn gpu_scan_errors_say_where_they_came_from() {
        let error = ScanError::Gpu(NvmlError::NotFound);
        assert!(error.to_string().starts_with("gpu error:"));
    }
}

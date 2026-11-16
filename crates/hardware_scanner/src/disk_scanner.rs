use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    time::Instant,
};

use crate::{
    model::{Bytes, Degree, DiskInfo, DiskKind, DiskStatic, Percent, Volume},
    sysfs::{read_dir_paths, read_trimmed},
};

const SYS_BLOCK: &str = "/sys/block";
const SYS_CLASS_BLOCK: &str = "/sys/class/block";
const DISKSTATS: &str = "/proc/diskstats";
/// /proc/diskstats and /sys/block/*/size always count 512-byte sectors.
const SECTOR_BYTES: u64 = 512;
/// Drive temperatures outside this range are treated as a bad reading.
const MAX_VALID_C: f32 = 150.0;

/// Cumulative counters of one disk from /proc/diskstats.
#[derive(Clone, Copy)]
struct Counters {
    reads: u64,
    sectors_read: u64,
    read_ms: u64,
    writes: u64,
    sectors_written: u64,
    write_ms: u64,
    io_ms: u64,
}

/// Reads physical disks from /sys/block, /proc/diskstats and the mount table.
pub struct DiskScanner {
    /// Physical disks found at startup; disks plugged in later are not picked up.
    ids: Vec<Box<str>>,
    disks: Option<sysinfo::Disks>,
    previous: HashMap<String, Counters>,
    last_scan: Option<Instant>,
}
impl DiskScanner {
    /// Finds the disks present right now and loads the mount list.
    pub fn new() -> Self {
        Self {
            ids: discover_disks(),
            disks: sysinfo::Disks::new_with_refreshed_list().ok(),
            previous: HashMap::new(),
            last_scan: None,
        }
    }

    /// Model, size and type of every disk found at startup, in the same order as [`scan`](Self::scan).
    pub fn static_infos(&self) -> Vec<DiskStatic> {
        self.ids.iter().map(|id| static_info(id)).collect()
    }

    /// Appends one [`DiskInfo`] per known disk. Rates need two scans, so the first has none.
    pub fn scan(&mut self, out: &mut Vec<DiskInfo>) {
        if let Some(disks) = &mut self.disks {
            disks.refresh(true);
        }

        let now = Instant::now();
        let seconds = self
            .last_scan
            .map(|last| now.duration_since(last).as_secs_f64())
            .filter(|&seconds| seconds > 0.0);

        let stats = read_diskstats(&self.ids);
        let mut volumes = self.volumes_by_disk();

        for id in &self.ids {
            let mut info = DiskInfo {
                id: id.clone(),
                ..DiskInfo::default()
            };

            if let (Some(now), Some(before), Some(seconds)) =
                (stats.get(&**id), self.previous.get(&**id), seconds)
            {
                fill_rates(&mut info, before, now, seconds);
            }

            info.temperature = disk_temperature(id);
            info.volumes = volumes.remove(&**id).unwrap_or_default();
            out.push(info);
        }

        self.previous = stats;
        self.last_scan = Some(now);
    }

    /// Mounted filesystems grouped by the physical disk they live on.
    fn volumes_by_disk(&self) -> HashMap<String, Vec<Volume>> {
        let mut found: Vec<(String, String, Volume)> = Vec::new();
        let Some(disks) = &self.disks else {
            return HashMap::new();
        };

        for disk in disks.iter() {
            let device = disk.name().to_string_lossy().into_owned();
            let Some(parent) = parent_disk(&device) else {
                continue;
            };

            let total = disk.total_space();
            let used = total.saturating_sub(disk.available_space());
            found.push((
                device,
                parent,
                Volume {
                    mount: disk.mount_point().into(),
                    file_system: disk.file_system().to_string_lossy().into_owned().into(),
                    total: Bytes::new(total),
                    used: Bytes::new(used),
                },
            ));
        }

        // A device mounted several times (bind mounts, btrfs subvolumes) shows its shortest mount once.
        found.sort_by_key(|(_, _, volume)| volume.mount.as_os_str().len());
        let mut seen = HashSet::new();
        let mut by_disk: HashMap<String, Vec<Volume>> = HashMap::new();
        for (device, parent, volume) in found {
            if seen.insert(device) {
                by_disk.entry(parent).or_default().push(volume);
            }
        }
        by_disk
    }
}

fn fill_rates(info: &mut DiskInfo, before: &Counters, now: &Counters, seconds: f64) {
    let per_second = |now: u64, before: u64| {
        let bytes = now.saturating_sub(before) * SECTOR_BYTES;
        Bytes::new((bytes as f64 / seconds) as u64)
    };
    info.read_rate = Some(per_second(now.sectors_read, before.sectors_read));
    info.write_rate = Some(per_second(now.sectors_written, before.sectors_written));

    let busy_ms = now.io_ms.saturating_sub(before.io_ms) as f64;
    let active = (busy_ms / (seconds * 1000.0) * 100.0).clamp(0.0, 100.0);
    info.active_time = Some(Percent::new(active as f32));

    let requests = now.reads.saturating_sub(before.reads) + now.writes.saturating_sub(before.writes);
    if requests > 0 {
        let wait_ms = now.read_ms.saturating_sub(before.read_ms)
            + now.write_ms.saturating_sub(before.write_ms);
        info.response_ms = Some(wait_ms as f32 / requests as f32);
    }
}

/// Physical disks: block devices with a backing `device` (this skips loop, ram, zram and
/// device-mapper) that are not optical drives.
fn discover_disks() -> Vec<Box<str>> {
    let mut ids: Vec<Box<str>> = read_dir_paths(SYS_BLOCK)
        .into_iter()
        .filter(|path| path.join("device").exists())
        .filter_map(|path| path.file_name()?.to_str().map(Box::from))
        .filter(|name: &Box<str>| !name.starts_with("sr"))
        .collect();
    ids.sort();
    ids
}

fn static_info(id: &str) -> DiskStatic {
    let dir = Path::new(SYS_BLOCK).join(id);

    let sectors = read_trimmed(&dir.join("size")).and_then(|v| v.parse::<u64>().ok());
    let rotational = read_trimmed(&dir.join("queue/rotational"));
    let kind = if id.starts_with("nvme") {
        DiskKind::Nvme
    } else {
        match rotational.as_deref() {
            Some("1") => DiskKind::Hdd,
            Some("0") => DiskKind::Ssd,
            _ => DiskKind::Unknown,
        }
    };

    DiskStatic {
        id: id.into(),
        model: read_trimmed(&dir.join("device/model")).unwrap_or_default().into(),
        size: Bytes::new(sectors.unwrap_or(0) * SECTOR_BYTES),
        kind,
    }
}

fn read_diskstats(ids: &[Box<str>]) -> HashMap<String, Counters> {
    let Ok(text) = fs::read_to_string(DISKSTATS) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| parse_diskstats_line(line, ids))
        .collect()
}

/// Fields after major/minor/name: reads, merged, sectors read, ms reading, writes, merged,
/// sectors written, ms writing, in flight, ms doing I/O, ...
fn parse_diskstats_line(line: &str, ids: &[Box<str>]) -> Option<(String, Counters)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let name = *fields.get(2)?;
    if !ids.iter().any(|id| &**id == name) {
        return None;
    }

    let number = |i: usize| fields.get(i)?.parse::<u64>().ok();
    Some((
        name.to_string(),
        Counters {
            reads: number(3)?,
            sectors_read: number(5)?,
            read_ms: number(6)?,
            writes: number(7)?,
            sectors_written: number(9)?,
            write_ms: number(10)?,
            io_ms: number(12)?,
        },
    ))
}

/// NVMe drives expose `device/hwmon*/temp1_input`; SATA drives with the `drivetemp` module
/// expose `device/hwmon/hwmon*/temp1_input`.
fn disk_temperature(id: &str) -> Option<Degree> {
    let device = Path::new(SYS_BLOCK).join(id).join("device");
    let mut candidates = read_dir_paths(&device);
    candidates.extend(read_dir_paths(device.join("hwmon")));

    candidates
        .into_iter()
        .filter(|dir| {
            dir.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("hwmon"))
        })
        .find_map(|dir| {
            let celsius = read_trimmed(&dir.join("temp1_input"))?.parse::<f32>().ok()? / 1000.0;
            (0.0..=MAX_VALID_C)
                .contains(&celsius)
                .then(|| Degree::new(celsius))
        })
}

/// Maps a mounted device (`/dev/nvme0n1p2`, `/dev/mapper/root`) to its physical disk name.
fn parent_disk(device: &str) -> Option<String> {
    let real = fs::canonicalize(device).ok()?;
    resolve_block(real.file_name()?.to_str()?, 0)
}

fn resolve_block(name: &str, depth: u8) -> Option<String> {
    if depth > 4 {
        return None;
    }
    let class = Path::new(SYS_CLASS_BLOCK).join(name);

    // device-mapper and md devices sit on top of other devices: follow the first one.
    if let Some(slave) = read_dir_paths(class.join("slaves")).into_iter().next() {
        return resolve_block(slave.file_name()?.to_str()?, depth + 1);
    }

    // A partition's sysfs directory sits inside its disk's directory.
    if class.join("partition").exists() {
        let real = fs::canonicalize(&class).ok()?;
        return real.parent()?.file_name()?.to_str().map(str::to_string);
    }

    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<Box<str>> {
        vec!["nvme0n1".into(), "sda".into()]
    }

    fn counters(sectors_read: u64, sectors_written: u64, io_ms: u64) -> Counters {
        Counters {
            reads: 0,
            sectors_read,
            read_ms: 0,
            writes: 0,
            sectors_written,
            write_ms: 0,
            io_ms,
        }
    }

    #[test]
    fn parses_a_known_disk_line() {
        let line = "259 0 nvme0n1 1000 5 20000 300 2000 7 40000 600 0 450 900 0 0 0 0";
        let (name, c) = parse_diskstats_line(line, &ids()).unwrap();
        assert_eq!(name, "nvme0n1");
        assert_eq!((c.reads, c.sectors_read, c.read_ms), (1000, 20000, 300));
        assert_eq!((c.writes, c.sectors_written, c.write_ms), (2000, 40000, 600));
        assert_eq!(c.io_ms, 450);
    }

    #[test]
    fn ignores_partitions_and_unknown_devices() {
        let partition = "259 1 nvme0n1p1 1 0 1 0 1 0 1 0 0 1 1 0 0 0 0";
        let loopdev = "7 0 loop0 1 0 1 0 1 0 1 0 0 1 1 0 0 0 0";
        assert!(parse_diskstats_line(partition, &ids()).is_none());
        assert!(parse_diskstats_line(loopdev, &ids()).is_none());
    }

    #[test]
    fn rejects_short_or_garbled_lines() {
        assert!(parse_diskstats_line("259 0 nvme0n1 1 2", &ids()).is_none());
        assert!(parse_diskstats_line("", &ids()).is_none());
    }

    #[test]
    fn rates_use_512_byte_sectors_per_second() {
        let mut info = DiskInfo::default();
        // 2048 sectors read in 2 s = 1 MiB over 2 s = 524288 B/s
        fill_rates(&mut info, &counters(0, 0, 0), &counters(2048, 4096, 500), 2.0);
        assert_eq!(info.read_rate, Some(Bytes::new(524_288)));
        assert_eq!(info.write_rate, Some(Bytes::new(1_048_576)));
        // 500 ms busy out of 2000 ms
        assert_eq!(info.active_time, Some(Percent::new(25.0)));
    }

    #[test]
    fn active_time_is_clamped_to_100() {
        let mut info = DiskInfo::default();
        fill_rates(&mut info, &counters(0, 0, 0), &counters(0, 0, 5_000), 1.0);
        assert_eq!(info.active_time, Some(Percent::new(100.0)));
    }

    #[test]
    fn counter_reset_does_not_underflow() {
        let mut info = DiskInfo::default();
        fill_rates(&mut info, &counters(900, 900, 900), &counters(10, 10, 10), 1.0);
        assert_eq!(info.read_rate, Some(Bytes::new(0)));
        assert_eq!(info.active_time, Some(Percent::new(0.0)));
    }

    #[test]
    fn response_time_averages_over_requests() {
        let mut info = DiskInfo::default();
        let before = counters(0, 0, 0);
        let now = Counters {
            reads: 2,
            read_ms: 6,
            writes: 2,
            write_ms: 2,
            ..counters(0, 0, 0)
        };
        fill_rates(&mut info, &before, &now, 1.0);
        // 8 ms of waiting over 4 requests
        assert_eq!(info.response_ms, Some(2.0));
    }

    #[test]
    fn no_requests_means_no_response_time() {
        let mut info = DiskInfo::default();
        fill_rates(&mut info, &counters(0, 0, 0), &counters(0, 0, 0), 1.0);
        assert_eq!(info.response_ms, None);
    }
}

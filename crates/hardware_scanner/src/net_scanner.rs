use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    path::Path,
    time::Instant,
};

use crate::{
    model::{Bytes, NetInfo, NetKind, NetStatic},
    sysfs::read_dir_paths,
};

const SYS_NET: &str = "/sys/class/net";
const PROC_WIRELESS: &str = "/proc/net/wireless";

pub struct NetScanner {
    /// Physical interfaces found at startup; interfaces added later are not picked up.
    ids: Vec<Box<str>>,
    networks: Option<sysinfo::Networks>,
    /// Cumulative (received, transmitted) byte counters from the previous scan.
    previous: HashMap<String, (u64, u64)>,
    last_scan: Option<Instant>,
}
impl NetScanner {
    pub fn new() -> Self {
        let networks = sysinfo::Networks::new_with_refreshed_list().ok();
        Self {
            ids: discover_interfaces(),
            networks,
            previous: HashMap::new(),
            last_scan: None,
        }
    }

    pub fn static_infos(&self) -> Vec<NetStatic> {
        self.ids
            .iter()
            .map(|id| NetStatic {
                id: id.clone(),
                kind: kind_of(id),
                mac: self.mac_address(id),
            })
            .collect()
    }

    /// Appends one [`NetInfo`] per known interface. Rates need two scans, so the first has none.
    pub fn scan(&mut self, out: &mut Vec<NetInfo>) {
        let Some(networks) = &mut self.networks else {
            return;
        };
        networks.refresh(true);

        let now = Instant::now();
        let seconds = self
            .last_scan
            .map(|last| now.duration_since(last).as_secs_f64())
            .filter(|&seconds| seconds > 0.0);
        let signals = read_wireless_signals();

        let mut counters = HashMap::new();
        for id in &self.ids {
            let mut info = NetInfo {
                id: id.clone(),
                ..NetInfo::default()
            };

            if let Some(data) = networks.get(OsStr::new(&**id)) {
                info.connected = matches!(
                    data.operational_state(),
                    sysinfo::InterfaceOperationalState::Up
                );
                info.link_speed = data.receive_link_speed();
                info.addresses = data.ip_networks().iter().map(|ip| ip.addr).collect();
                info.addresses.sort_by_key(|addr| addr.is_ipv6());

                let current = (data.total_received(), data.total_transmitted());
                if let (Some(before), Some(seconds)) = (self.previous.get(&**id), seconds) {
                    info.receive_rate = Some(per_second(current.0, before.0, seconds));
                    info.send_rate = Some(per_second(current.1, before.1, seconds));
                }
                counters.insert(id.to_string(), current);
            }

            info.signal_dbm = signals.get(&**id).copied();
            out.push(info);
        }

        self.previous = counters;
        self.last_scan = Some(now);
    }

    fn mac_address(&self, id: &str) -> Option<Box<str>> {
        let mac = self.networks.as_ref()?.get(OsStr::new(id))?.mac_address();
        (mac != sysinfo::MacAddr::UNSPECIFIED).then(|| mac.to_string().into())
    }
}

/// Bytes per second between two cumulative counters; a counter reset counts as zero.
fn per_second(now: u64, before: u64, seconds: f64) -> Bytes {
    Bytes::new((now.saturating_sub(before) as f64 / seconds) as u64)
}

/// Physical interfaces: everything in /sys/class/net except loopback and virtual devices
/// (docker0, veth*, bridges, tunnels), whose sysfs path runs through `.../virtual/net/...`.
fn discover_interfaces() -> Vec<Box<str>> {
    let mut ids: Vec<Box<str>> = read_dir_paths(SYS_NET)
        .into_iter()
        .filter(|path| !is_virtual(path))
        .filter_map(|path| path.file_name()?.to_str().map(Box::from))
        .filter(|name: &Box<str>| &**name != "lo")
        .collect();
    ids.sort();
    ids
}

fn is_virtual(interface: &Path) -> bool {
    fs::canonicalize(interface).is_ok_and(|real| path_is_virtual(&real))
}

fn path_is_virtual(real_path: &Path) -> bool {
    real_path
        .components()
        .any(|component| component.as_os_str() == "virtual")
}

fn kind_of(id: &str) -> NetKind {
    if Path::new(SYS_NET).join(id).join("wireless").exists() {
        NetKind::Wifi
    } else {
        NetKind::Ethernet
    }
}

fn read_wireless_signals() -> HashMap<String, i32> {
    fs::read_to_string(PROC_WIRELESS)
        .map(|text| parse_wireless(&text))
        .unwrap_or_default()
}

/// Signal level per interface from /proc/net/wireless:
///
/// ```text
/// wlan0: 0000   70.  -40.  -256   0 0 0 0 0   0
/// ```
/// The third column is the level. Only negative values are kept, since a positive one is a
/// driver-specific scale and not dBm.
fn parse_wireless(text: &str) -> HashMap<String, i32> {
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let level = rest.split_whitespace().nth(2)?;
            let dbm = level.trim_end_matches('.').parse::<f32>().ok()?;
            (dbm <= 0.0).then(|| (name.trim().to_string(), dbm as i32))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_is_bytes_per_second() {
        assert_eq!(per_second(3_000, 1_000, 2.0), Bytes::new(1_000));
    }

    #[test]
    fn counter_reset_counts_as_zero() {
        assert_eq!(per_second(10, 9_999, 1.0), Bytes::new(0));
    }

    #[test]
    fn virtual_interfaces_are_recognised_by_path() {
        assert!(path_is_virtual(Path::new("/sys/devices/virtual/net/docker0")));
        assert!(!path_is_virtual(Path::new(
            "/sys/devices/pci0000:00/0000:00:1c.2/0000:05:00.0/net/enp5s0"
        )));
    }

    #[test]
    fn parses_signal_level_in_dbm() {
        let text = "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE\n \
                    face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22\n \
                    wlan0: 0000   70.  -40.  -256        0      0      0      0      0        0\n";
        let signals = parse_wireless(text);
        assert_eq!(signals.get("wlan0"), Some(&-40));
    }

    #[test]
    fn ignores_non_dbm_levels_and_garbage() {
        let text = "header one\nheader two\n wlan0: 0000 70. 55. -256 0 0 0 0 0 0\n not a line\n";
        assert!(parse_wireless(text).is_empty());
    }

    #[test]
    fn empty_wireless_file_has_no_signals() {
        assert!(parse_wireless("").is_empty());
    }
}

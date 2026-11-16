//! Which kernel sensors to read for the CPU and RAM temperature.
//!
//! Rules are tried in order and the first one that yields a sensor wins, so more specific or
//! more accurate drivers come first.

/// Readings below this are treated as a bad sensor, not a real temperature.
pub const MIN_VALID_C: f32 = -40.0;
/// Readings above this are treated as a bad sensor, not a real temperature.
pub const MAX_VALID_C: f32 = 150.0;

#[derive(Debug, Clone, Copy)]
pub struct HwmonRule {
    /// Contents of `hwmonN/name`.
    pub driver: &'static str,
    /// Label prefixes tried in order. The first one that matches is used.
    pub preferred: &'static [&'static str],
    /// Label prefixes used if none of `preferred` matched.
    pub fallback: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct ThermalZoneRule {
    /// Contents of `thermal_zoneN/type`.
    pub zone_type: &'static str,
    /// `false` means this is a weak guess and should be used only as a last resort.
    pub reliable: bool,
}

/// Everything known about finding one component's temperature (CPU or RAM).
pub struct TargetRules {
    /// `/sys/class/hwmon` drivers, tried first and in this order.
    pub hwmon: &'static [HwmonRule],
    /// `/sys/class/thermal` zones, tried when no hwmon rule matched.
    pub thermal_zones: &'static [ThermalZoneRule],
}

/// CPU temperature sources.
pub const CPU: TargetRules = TargetRules {
    hwmon: &[
        // Intel
        HwmonRule {
            driver: "coretemp",
            preferred: &["Package id"],
            fallback: &["Core"],
        },
        // AMD, out-of-tree driver (preferred over k10temp when both exist)
        HwmonRule {
            driver: "zenpower",
            preferred: &["Tdie", "Tctl"],
            fallback: &["Tccd"],
        },
        // AMD, in-kernel driver
        HwmonRule {
            driver: "k10temp",
            preferred: &["Tctl", "Tdie"],
            fallback: &["Tccd"],
        },
        // VIA
        HwmonRule {
            driver: "via_cputemp",
            preferred: &[],
            fallback: &[],
        },
        HwmonRule {
            driver: "via-cputemp",
            preferred: &[],
            fallback: &[],
        },
        // ARM and single-board computers
        HwmonRule {
            driver: "cpu_thermal",
            preferred: &[],
            fallback: &[],
        },
        HwmonRule {
            driver: "cpu-thermal",
            preferred: &[],
            fallback: &[],
        },
        HwmonRule {
            driver: "soc_thermal",
            preferred: &[],
            fallback: &[],
        },
        HwmonRule {
            driver: "soc-thermal",
            preferred: &[],
            fallback: &[],
        },
    ],
    thermal_zones: &[
        ThermalZoneRule {
            zone_type: "x86_pkg_temp",
            reliable: true,
        },
        ThermalZoneRule {
            zone_type: "cpu-thermal",
            reliable: true,
        },
        ThermalZoneRule {
            zone_type: "cpu_thermal",
            reliable: true,
        },
        ThermalZoneRule {
            zone_type: "soc-thermal",
            reliable: true,
        },
        // Generic board sensor, not necessarily the CPU.
        ThermalZoneRule {
            zone_type: "acpitz",
            reliable: false,
        },
    ],
};

/// RAM (DIMM) temperature sources.
pub const RAM: TargetRules = TargetRules {
    hwmon: &[
        // DDR5 SPD hub, one device per DIMM, unlabeled
        HwmonRule {
            driver: "spd5118",
            preferred: &[],
            fallback: &[],
        },
        // DDR3/DDR4 SPD temperature sensors, one device per DIMM
        HwmonRule {
            driver: "jc42",
            preferred: &[],
            fallback: &[],
        },
    ],
    // No thermal-zone fallback exists for RAM.
    thermal_zones: &[],
};

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const TABLES: [(&str, &TargetRules); 2] = [("CPU", &CPU), ("RAM", &RAM)];

    #[test]
    fn the_valid_range_is_a_sane_temperature_window() {
        assert!(MIN_VALID_C < 0.0);
        assert!(MAX_VALID_C > 100.0);
        assert!(MIN_VALID_C < MAX_VALID_C);
    }

    #[test]
    fn hwmon_drivers_are_unique_within_a_table() {
        for (name, rules) in TABLES {
            let mut seen = HashSet::new();
            for rule in rules.hwmon {
                assert!(seen.insert(rule.driver), "{name}: duplicate driver {}", rule.driver);
            }
        }
    }

    #[test]
    fn thermal_zone_types_are_unique_within_a_table() {
        for (name, rules) in TABLES {
            let mut seen = HashSet::new();
            for zone in rules.thermal_zones {
                assert!(seen.insert(zone.zone_type), "{name}: duplicate zone {}", zone.zone_type);
            }
        }
    }

    #[test]
    fn no_rule_has_an_empty_name_or_prefix() {
        for (name, rules) in TABLES {
            for rule in rules.hwmon {
                assert!(!rule.driver.is_empty(), "{name}: empty driver name");
                for prefix in rule.preferred.iter().chain(rule.fallback) {
                    assert!(!prefix.is_empty(), "{name}/{}: empty label prefix", rule.driver);
                }
            }
            for zone in rules.thermal_zones {
                assert!(!zone.zone_type.is_empty(), "{name}: empty zone type");
            }
        }
    }

    #[test]
    fn cpu_has_both_vendors_and_ram_has_no_thermal_fallback() {
        let drivers: Vec<_> = CPU.hwmon.iter().map(|rule| rule.driver).collect();
        assert!(drivers.contains(&"coretemp"), "Intel driver missing");
        assert!(drivers.contains(&"k10temp"), "AMD driver missing");
        assert!(RAM.thermal_zones.is_empty());
    }

    #[test]
    fn unreliable_zones_are_only_used_as_a_last_resort() {
        // The scanner tries reliable zones first, so at least one reliable zone must exist
        // or the unreliable guess would always win.
        assert!(CPU.thermal_zones.iter().any(|zone| zone.reliable));
    }
}

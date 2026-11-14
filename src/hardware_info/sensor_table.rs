pub const MIN_VALID_C: f32 = -40.0;
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

pub struct TargetRules {
    pub hwmon: &'static [HwmonRule],
    pub thermal_zones: &'static [ThermalZoneRule],
}

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

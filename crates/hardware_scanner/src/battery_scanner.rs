use std::{path::Path, time::Duration};

use crate::{
    model::{BatteryInfo, BatteryState, Percent, Watts},
    sysfs::{read_dir_paths, read_trimmed},
};

const POWER_SUPPLY: &str = "/sys/class/power_supply";

/// The first battery found, or `None` on machines without one (desktops).
pub fn read_battery() -> Option<BatteryInfo> {
    let dir = read_dir_paths(POWER_SUPPLY)
        .into_iter()
        .find(|dir| read_trimmed(&dir.join("type")).as_deref() == Some("Battery"))?;

    let percent = read_trimmed(&dir.join("capacity"))?.parse::<f32>().ok()?;
    let state = parse_state(&read_trimmed(&dir.join("status")).unwrap_or_default());

    // Batteries report either energy (µWh) and power (µW), or charge (µAh) and current (µA)
    // plus a voltage (µV). Charge times voltage gives energy, current times voltage gives power.
    let voltage = read_u64(&dir, "voltage_now");
    let from_charge = |name: &str| Some(read_u64(&dir, name)? * voltage? / 1_000_000);
    let power = read_u64(&dir, "power_now").or_else(|| from_charge("current_now"));
    let energy_now = read_u64(&dir, "energy_now").or_else(|| from_charge("charge_now"));
    let energy_full = read_u64(&dir, "energy_full").or_else(|| from_charge("charge_full"));

    Some(BatteryInfo {
        percent: Percent::new(percent),
        state,
        power: power.map(|microwatts| Watts::new(microwatts as f32 / 1_000_000.0)),
        time_remaining: time_remaining(state, energy_now, energy_full, power),
    })
}

fn read_u64(dir: &Path, name: &str) -> Option<u64> {
    read_trimmed(&dir.join(name))?.parse().ok()
}

fn parse_state(status: &str) -> BatteryState {
    match status {
        "Charging" => BatteryState::Charging,
        "Discharging" => BatteryState::Discharging,
        "Full" => BatteryState::Full,
        "Not charging" => BatteryState::NotCharging,
        _ => BatteryState::Unknown,
    }
}

/// Energy in µWh divided by power in µW gives hours. Only meaningful while charging or
/// discharging at a non-zero rate.
fn time_remaining(
    state: BatteryState,
    energy_now: Option<u64>,
    energy_full: Option<u64>,
    power: Option<u64>,
) -> Option<Duration> {
    let power = power.filter(|&power| power > 0)?;
    let energy = match state {
        BatteryState::Discharging => energy_now?,
        BatteryState::Charging => energy_full?.saturating_sub(energy_now?),
        _ => return None,
    };
    Some(Duration::from_secs_f64(energy as f64 / power as f64 * 3600.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discharging_time_is_energy_over_power() {
        // 30 Wh left at 15 W = 2 hours
        let time = time_remaining(
            BatteryState::Discharging,
            Some(30_000_000),
            Some(50_000_000),
            Some(15_000_000),
        );
        assert_eq!(time, Some(Duration::from_secs(7_200)));
    }

    #[test]
    fn charging_time_is_the_missing_energy_over_power() {
        // 20 Wh missing at 20 W = 1 hour
        let time = time_remaining(
            BatteryState::Charging,
            Some(30_000_000),
            Some(50_000_000),
            Some(20_000_000),
        );
        assert_eq!(time, Some(Duration::from_secs(3_600)));
    }

    #[test]
    fn no_estimate_when_full_or_idle_or_unknown() {
        let args = (Some(30_000_000), Some(50_000_000), Some(10_000_000));
        for state in [
            BatteryState::Full,
            BatteryState::NotCharging,
            BatteryState::Unknown,
        ] {
            assert_eq!(time_remaining(state, args.0, args.1, args.2), None);
        }
    }

    #[test]
    fn no_estimate_without_a_usable_power_reading() {
        let discharging = |power| {
            time_remaining(
                BatteryState::Discharging,
                Some(30_000_000),
                Some(50_000_000),
                power,
            )
        };
        assert_eq!(discharging(None), None);
        assert_eq!(discharging(Some(0)), None);
    }

    #[test]
    fn charging_past_full_does_not_underflow() {
        let time = time_remaining(
            BatteryState::Charging,
            Some(60_000_000),
            Some(50_000_000),
            Some(10_000_000),
        );
        assert_eq!(time, Some(Duration::from_secs(0)));
    }

    #[test]
    fn parses_kernel_status_strings() {
        assert_eq!(parse_state("Charging"), BatteryState::Charging);
        assert_eq!(parse_state("Discharging"), BatteryState::Discharging);
        assert_eq!(parse_state("Full"), BatteryState::Full);
        assert_eq!(parse_state("Not charging"), BatteryState::NotCharging);
        assert_eq!(parse_state("???"), BatteryState::Unknown);
    }
}

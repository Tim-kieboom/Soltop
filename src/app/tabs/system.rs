//! The System tab: host, OS, load and battery.

use eframe::egui;
use hardware_scanner::{
    HardwareInfo,
    model::{BatteryInfo, SystemStatic},
};

use super::performance::{section_header, stats_ui, uptime_text};

const BATTERY_COLOR: egui::Color32 = egui::Color32::from_rgb(96, 168, 72);

/// The System tab: facts about the machine and the OS, and battery state when there is one.
pub struct SystemView {
    facts: SystemStatic,
}
impl SystemView {
    pub fn new() -> Self {
        Self {
            facts: SystemStatic::default(),
        }
    }

    /// Called once with the first scan, which carries the static system facts.
    pub fn load_first_info(&mut self, facts: SystemStatic) {
        self.facts = facts;
    }

    pub fn ui(&self, info: &HardwareInfo, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            section_header(ui, "System", &self.facts.hostname);

            ui.add_space(8.0);
            stats_ui(
                ui,
                &[
                    ("Operating system", text_or_unknown(&self.facts.os)),
                    ("Kernel", text_or_unknown(&self.facts.kernel)),
                    ("Architecture", text_or_unknown(&self.facts.architecture)),
                ],
            );

            ui.add_space(4.0);
            let load = info.system().load.map_or("???".into(), |load| {
                format!("{:.2}  {:.2}  {:.2}", load.one, load.five, load.fifteen)
            });
            stats_ui(
                ui,
                &[
                    ("Up time", info.cpu().uptime.map_or("???".into(), uptime_text)),
                    ("Processes", info.processes().len().to_string()),
                    ("Load average (1, 5, 15 min)", load),
                ],
            );

            if let Some(battery) = info.system().battery {
                ui.add_space(16.0);
                battery_ui(ui, battery);
            }
        });
    }
}

/// The battery section: charge bar, state, power and time remaining.
fn battery_ui(ui: &mut egui::Ui, battery: BatteryInfo) {
    let percent = battery.percent.as_f32();
    section_header(ui, "Battery", &format!("{percent:.0}%"));
    ui.add(
        egui::ProgressBar::new(percent / 100.0)
            .fill(BATTERY_COLOR.gamma_multiply(0.7))
            .desired_height(18.0),
    );

    ui.add_space(8.0);
    stats_ui(
        ui,
        &[
            ("State", battery.state.label().to_string()),
            (
                "Power",
                battery
                    .power
                    .map_or("???".into(), |power| format!("{:.1} W", power.as_f32())),
            ),
            (
                "Time remaining",
                battery.time_remaining.map_or("n/a".into(), battery_time),
            ),
        ],
    );
}

/// "2h 05m".
fn battery_time(time: std::time::Duration) -> String {
    let minutes = time.as_secs() / 60;
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

/// The text itself, or `???` when it is empty.
fn text_or_unknown(text: &str) -> String {
    if text.is_empty() {
        "???".to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn battery_time_is_hours_and_zero_padded_minutes() {
        assert_eq!(battery_time(Duration::from_secs(0)), "0h 00m");
        assert_eq!(battery_time(Duration::from_secs(5 * 60)), "0h 05m");
        assert_eq!(battery_time(Duration::from_secs(7_260)), "2h 01m");
        assert_eq!(battery_time(Duration::from_secs(10 * 3_600 + 59 * 60)), "10h 59m");
    }

    #[test]
    fn battery_time_drops_the_leftover_seconds() {
        assert_eq!(battery_time(Duration::from_secs(119)), "0h 01m");
    }

    #[test]
    fn empty_facts_show_as_unknown() {
        assert_eq!(text_or_unknown(""), "???");
        assert_eq!(text_or_unknown("6.12.4"), "6.12.4");
    }

    #[test]
    fn a_new_view_has_no_facts_yet() {
        assert!(SystemView::new().facts.hostname.is_empty());
    }
}

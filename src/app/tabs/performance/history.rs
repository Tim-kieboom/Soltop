use std::collections::VecDeque;

use eframe::egui::{self, Color32};
use hardware_scanner::model::AsF64;

pub struct History<T: AsF64> {
    max: f64,
    limit: usize,
    queue: VecDeque<T>,
}
impl<T: AsF64> History<T> {
    pub fn new(limit: usize, max: f64) -> Self {
        Self {
            max,
            limit,
            queue: VecDeque::with_capacity(limit),
        }
    }

    pub fn set_max(&mut self, max: f64) {
        self.max = max
    }

    pub fn max(&self) -> f64 {
        self.max
    }

    pub fn push(&mut self, value: T) {
        if self.queue.len() >= self.limit {
            _ = self.queue.pop_front();
        }

        self.queue.push_back(value);
    }

    /// The newest `count` samples, oldest first.
    pub fn recent(&self, count: usize) -> Vec<f64> {
        let skip = self.queue.len().saturating_sub(count);
        self.queue.iter().skip(skip).map(|v| v.as_f64()).collect()
    }

    pub fn plot(
        &self,
        ui: &mut egui::Ui,
        id: impl std::hash::Hash + std::fmt::Debug,
        line_name: &str,
        color: Color32,
    ) {
        let points: Vec<[f64; 2]> = self
            .queue
            .iter()
            .enumerate()
            .map(|(i, v)| [i as f64, v.as_f64()])
            .collect();

        egui_plot::Plot::new(egui::Id::unique(id))
            .height(160.0)
            .include_y(0.0)
            .include_y(self.max)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .show_axes([false, false])
            .show_x(false)
            .show_y(false)
            .show(ui, |plot_ui| {
                plot_ui.line(
                    egui_plot::Line::new(line_name, points)
                        .color(color)
                        .width(1.5)
                        .fill(0.0)
                        .fill_alpha(0.25),
                );
            });
    }
}

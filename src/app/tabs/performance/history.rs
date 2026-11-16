//! A fixed-size history of samples, drawn as a graph.

use std::collections::VecDeque;

use eframe::egui::{self, Color32};
use hardware_scanner::model::AsF64;

/// The newest `limit` samples of one value, oldest first, plus the value the graph's top edge
/// stands for. A `max` of 0 means "scale to the data".
pub struct History<T: AsF64> {
    max: f64,
    limit: usize,
    queue: VecDeque<T>,
}
impl<T: AsF64> History<T> {
    /// An empty history holding at most `limit` samples.
    pub fn new(limit: usize, max: f64) -> Self {
        Self {
            max,
            limit,
            queue: VecDeque::with_capacity(limit),
        }
    }

    /// Changes the value the graph's top edge stands for.
    pub fn set_max(&mut self, max: f64) {
        self.max = max
    }

    /// The value the graph's top edge stands for; 0 means "scale to the data".
    pub fn max(&self) -> f64 {
        self.max
    }

    /// Adds a sample, dropping the oldest one when the history is full.
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

    /// Draws the whole history as a filled line graph without axes.
    ///
    /// `id` must be unique per graph on screen, `line_name` labels the line.
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

#[cfg(test)]
mod tests {
    use hardware_scanner::model::Percent;

    use super::*;

    fn filled(limit: usize, values: impl IntoIterator<Item = u32>) -> History<Percent> {
        let mut history = History::new(limit, 100.0);
        for value in values {
            history.push(Percent::new(value as f32));
        }
        history
    }

    #[test]
    fn a_new_history_is_empty() {
        let history: History<Percent> = History::new(5, 100.0);
        assert!(history.recent(10).is_empty());
        assert_eq!(history.max(), 100.0);
    }

    #[test]
    fn push_keeps_only_the_newest_limit_samples() {
        let history = filled(3, 1..=5);
        assert_eq!(history.recent(10), vec![3.0, 4.0, 5.0]);
    }

    #[test]
    fn recent_returns_the_newest_samples_oldest_first() {
        let history = filled(10, 1..=5);
        assert_eq!(history.recent(2), vec![4.0, 5.0]);
    }

    #[test]
    fn recent_asking_for_more_than_exists_returns_everything() {
        let history = filled(10, 1..=2);
        assert_eq!(history.recent(60), vec![1.0, 2.0]);
    }

    #[test]
    fn recent_zero_is_empty() {
        let history = filled(10, 1..=3);
        assert!(history.recent(0).is_empty());
    }

    #[test]
    fn set_max_changes_the_scale() {
        let mut history: History<Percent> = History::new(5, 100.0);
        history.set_max(250.5);
        assert_eq!(history.max(), 250.5);
    }

    #[test]
    fn a_limit_of_one_keeps_just_the_latest_sample() {
        let history = filled(1, [7, 8, 9]);
        assert_eq!(history.recent(5), vec![9.0]);
    }
}

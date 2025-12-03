use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use eframe::egui::{Grid, Ui};
use tracing::error;

use crate::{
    listener::KeyValue,
    metric::{KeyContext, Metric},
};

/// Time threshold to consider typing flow
const TYPING_FLOW_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct FlightTime {
    last_release: Option<SystemTime>,
    stats: HashMap<String, (Duration, u64)>,
}

impl Metric for FlightTime {
    fn process(&mut self, ctx: &KeyContext) {
        match ctx.value {
            KeyValue::Down => {
                if let (Some(last_release), Some(current_char)) = (self.last_release, &ctx.utf8) {
                    let duration_since = match ctx.timestamp.duration_since(last_release) {
                        Ok(duration) => duration,
                        Err(err) => {
                            error!("failed to compute duration since last release: {err}");
                            return;
                        }
                    };
                    if duration_since < TYPING_FLOW_TIMEOUT {
                        let (accumulated_time, count) =
                            self.stats.entry(current_char.clone()).or_default();
                        *accumulated_time += duration_since;
                        *count += 1;
                    }
                }
            }
            KeyValue::Up => {
                self.last_release = Some(ctx.timestamp);
            }
            _ => {}
        }
    }

    fn ui(&self, ui: &mut Ui) {
        ui.heading("Flight Time (Hesitation)");
        ui.small("Avg time to reach this key after releasing the previous one.");
        ui.add_space(5.0);

        Grid::new("flight_time_grid").striped(true).show(ui, |ui| {
            ui.strong("Key");
            ui.strong("Avg Time (ms)");
            ui.end_row();

            let mut data: Vec<_> = self
                .stats
                .iter()
                .map(|(k, (d, c))| (k, d.as_secs_f64() * 1000.0 / *c as f64))
                .collect();

            // Sort by slowest (descending)
            data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            for (key, ms) in data.into_iter().take(5) {
                ui.label(key);
                ui.label(format!("{:.1}", ms));
                ui.end_row();
            }
        });
    }
}

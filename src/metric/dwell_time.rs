use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use eframe::egui::{Grid, Ui};
use evdev::KeyCode;
use tracing::error;

use crate::{
    listener::KeyValue,
    metric::{KeyContext, Metric},
};

#[derive(Default)]
pub struct DwellTime {
    pressed_keys: HashMap<KeyCode, SystemTime>,
    // Map char string to (total_duration, count)
    stats: HashMap<String, (Duration, u64)>,
}

impl Metric for DwellTime {
    fn process(&mut self, ctx: &KeyContext) {
        match ctx.value {
            KeyValue::Down => {
                self.pressed_keys.insert(ctx.key_code, ctx.timestamp);
            }
            KeyValue::Up => {
                let Some(start_time) = self.pressed_keys.remove(&ctx.key_code) else {
                    return;
                };
                if let Some(char_str) = &ctx.utf8 {
                    let duration_since = match ctx.timestamp.duration_since(start_time) {
                        Ok(duration) => duration,
                        Err(err) => {
                            error!("failed to compute duration since last release: {err}");
                            return;
                        }
                    };
                    let (accumulated_time, count) = self.stats.entry(char_str.clone()).or_default();
                    *accumulated_time += duration_since;
                    *count += 1;
                }
            }
            _ => {}
        }
    }

    fn ui(&self, ui: &mut Ui) {
        ui.heading("Dwell Time");
        ui.small("Avg time a key is held down.");
        ui.add_space(5.0);

        Grid::new("dwell_time_grid").striped(true).show(ui, |ui| {
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

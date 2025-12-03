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
pub struct BigramSpeed {
    last_press: Option<(String, SystemTime)>,
    // Map "char1 char2" to (total_duration, count)
    stats: HashMap<(String, String), (Duration, u64)>,
}

impl Metric for BigramSpeed {
    fn process(&mut self, ctx: &KeyContext) {
        if let KeyValue::Down = ctx.value {
            if let Some(current_char) = &ctx.utf8 {
                if let Some((last_char, last_time)) = &self.last_press {
                    let duration_since = match ctx.timestamp.duration_since(*last_time) {
                        Ok(duration) => duration,
                        Err(err) => {
                            error!("failed to compute duration since last release: {err}");
                            return;
                        }
                    };
                    if duration_since < TYPING_FLOW_TIMEOUT {
                        let key = (last_char.clone(), current_char.clone());
                        let (accumulated_time, count) = self.stats.entry(key).or_default();
                        *accumulated_time += duration_since;
                        *count += 1;
                    }
                }
                self.last_press = Some((current_char.clone(), ctx.timestamp));
            }
        }
    }

    fn ui(&self, ui: &mut Ui) {
        ui.heading("Bigram Speed (Flow)");
        ui.small("Speed of specific letter pairs.");
        ui.add_space(5.0);

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.strong("Fastest Pairs");
                self.render_list(ui, false);
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.strong("Slowest Pairs");
                self.render_list(ui, true);
            });
        });
    }
}

impl BigramSpeed {
    fn render_list(&self, ui: &mut Ui, slowest: bool) {
        Grid::new(if slowest { "slow_bg" } else { "fast_bg" })
            .striped(true)
            .show(ui, |ui| {
                let mut data: Vec<_> = self
                    .stats
                    .iter()
                    .filter(|(_, (_, count))| *count > 2) // Need at least 3 samples to be relevant
                    .map(|((c1, c2), (d, c))| {
                        (
                            format!("{} -> {}", c1, c2),
                            d.as_secs_f64() * 1000.0 / *c as f64,
                        )
                    })
                    .collect();

                if slowest {
                    data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                } else {
                    data.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                }

                for (pair, ms) in data.into_iter().take(5) {
                    ui.label(pair);
                    ui.label(format!("{:.0} ms", ms));
                    ui.end_row();
                }
            });
    }
}

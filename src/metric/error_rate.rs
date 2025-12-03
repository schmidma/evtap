use std::collections::{HashMap, VecDeque};

use eframe::egui::{self, Grid, Ui};
use evdev::KeyCode;

use crate::{
    listener::KeyValue,
    metric::{KeyContext, Metric},
};

const HISTORY_SIZE: usize = 10;

#[derive(Default)]
pub struct ErrorRate {
    /// Buffer of recently typed characters (e.g., ['h', 'e', 'l', 'l', 'o'])
    history: VecDeque<String>,
    /// The character that was just deleted by Backspace, waiting for a correction.
    last_deleted: Option<String>,
    /// Counts how often a specific character was deleted.
    mistakes: HashMap<String, u64>,
    /// Counts "I typed X instead of Y" (Mistake -> Correction).
    confusions: HashMap<(String, String), u64>,
}

impl Metric for ErrorRate {
    fn process(&mut self, ctx: &KeyContext) {
        if let KeyValue::Up = ctx.value {
            return;
        }

        if ctx.key_code == KeyCode::KEY_BACKSPACE {
            if let Some(deleted) = self.history.pop_back() {
                self.last_deleted = Some(deleted);
            }
        } else if let Some(utf8) = &ctx.utf8 {
            if let Some(mistake) = self.last_deleted.take() {
                *self.mistakes.entry(mistake.to_string()).or_default() += 1;
                *self
                    .confusions
                    .entry((mistake, utf8.to_string()))
                    .or_default() += 1;
            }

            self.history.push_back(utf8.to_string());
            if self.history.len() > HISTORY_SIZE {
                self.history.pop_front();
            }
        } else {
            // Non-character key (e.g. Arrows, Shift, Ctrl).
        }
    }

    fn ui(&self, ui: &mut Ui) {
        ui.heading("Mistake Analysis");
        ui.add_space(5.0);

        egui::Grid::new("error_stats_grid")
            .striped(true)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.strong("Most Deleted Keys");
                    Grid::new("deleted_keys").striped(true).show(ui, |ui| {
                        let mut sorted_mistakes: Vec<_> = self.mistakes.iter().collect();
                        sorted_mistakes.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

                        for (char, count) in sorted_mistakes.into_iter().take(5) {
                            ui.label(char);
                            ui.label(format!("{}", count));
                            ui.end_row();
                        }
                    });
                });

                ui.vertical(|ui| {
                    ui.strong("Top Confusions (Deleted -> Typed)");
                    Grid::new("confusions").striped(true).show(ui, |ui| {
                        let mut sorted_confusions: Vec<_> = self.confusions.iter().collect();
                        sorted_confusions.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
                        for ((mistake, correction), count) in sorted_confusions.into_iter().take(5)
                        {
                            ui.label(format!("{} -> {}", mistake, correction));
                            ui.label(format!("{}", count));
                            ui.end_row();
                        }
                    });
                });
            });
    }
}

use std::collections::HashMap;

use eframe::egui::{Grid, Ui};
use evdev::KeyCode;

use crate::{
    listener::KeyValue,
    metric::{KeyContext, Metric},
};

#[derive(Default)]
pub struct HeatMap {
    counts: HashMap<KeyCode, u64>,
}

impl Metric for HeatMap {
    fn process(&mut self, ctx: &KeyContext) {
        if let KeyValue::Down = ctx.value {
            *self.counts.entry(ctx.key_code).or_default() += 1;
        }
    }

    fn ui(&self, ui: &mut Ui) {
        ui.heading("Heat Map");
        Grid::new("heatmap_grid").striped(true).show(ui, |ui| {
            let mut counts: Vec<_> = self.counts.iter().collect();
            counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
            for (key_code, count) in counts {
                ui.label(format!("{:?}", key_code));
                ui.label(format!("{}", count));
                ui.end_row();
            }
        });
    }
}

use eframe::egui::Ui;

use crate::{
    listener::KeyValue,
    metric::{KeyContext, Metric},
};

#[derive(Default)]
pub struct TotalPresses {
    pub count: u64,
}

impl Metric for TotalPresses {
    fn process(&mut self, ctx: &KeyContext) {
        if let KeyValue::Down = ctx.value {
            self.count += 1;
        }
    }

    fn ui(&self, ui: &mut Ui) {
        ui.label(format!("Total Key Presses: {}", self.count));
    }
}

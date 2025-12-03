use std::time::SystemTime;

use eframe::egui;
use evdev::KeyCode;

use crate::listener::KeyValue;

pub mod bigram_speed;
pub mod dwell_time;
pub mod error_rate;
pub mod flight_time;
pub mod heatmap;
pub mod total_presses;

pub struct KeyContext {
    pub key_code: KeyCode,
    pub utf8: Option<String>,
    pub timestamp: SystemTime,
    pub value: KeyValue,
}

pub trait Metric {
    fn process(&mut self, ctx: &KeyContext);
    fn ui(&self, ui: &mut egui::Ui);
}

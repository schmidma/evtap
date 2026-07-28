use std::time::Duration;

use crate::input::KeyEvent;

mod bigram_speed;
mod dwell_time;
mod error_rate;
mod flight_time;
mod key_usage;
mod total_presses;

use bigram_speed::BigramSpeed;
use dwell_time::DwellTime;
use error_rate::ErrorRate;
use flight_time::FlightTime;
use key_usage::KeyUsage;
use total_presses::TotalPresses;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReportValue {
    Text(String),
    Count(u64),
    Milliseconds(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReportSection {
    Scalar {
        label: &'static str,
        value: ReportValue,
    },
    Table {
        title: Option<&'static str>,
        columns: &'static [&'static str],
        rows: Vec<Vec<ReportValue>>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricReport {
    pub sections: Vec<ReportSection>,
}

pub trait Metric {
    fn descriptor(&self) -> &'static MetricDescriptor;
    fn process(&mut self, event: &KeyEvent);
    fn report(&self) -> MetricReport;
    fn reset(&mut self);
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DurationStats {
    total: Duration,
    samples: u64,
}

impl DurationStats {
    fn record(&mut self, duration: Duration) {
        self.total += duration;
        self.samples += 1;
    }

    fn average_milliseconds(self) -> f64 {
        self.total.as_secs_f64() * 1_000.0 / self.samples as f64
    }
}

pub fn default_metrics() -> Vec<Box<dyn Metric>> {
    vec![
        Box::new(TotalPresses::default()),
        Box::new(KeyUsage::default()),
        Box::new(ErrorRate::default()),
        Box::new(FlightTime::default()),
        Box::new(DwellTime::default()),
        Box::new(BigramSpeed::default()),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::default_metrics;

    #[test]
    fn default_metric_ids_are_unique() {
        let mut ids = HashSet::new();

        for metric in default_metrics() {
            let id = metric.descriptor().id;
            assert!(ids.insert(id), "duplicate metric id: {id}");
        }
    }
}

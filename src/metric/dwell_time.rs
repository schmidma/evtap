use std::{collections::HashMap, time::SystemTime};

use crate::{
    input::{KeyEvent, KeyEventKind, PhysicalKey},
    metric::{DurationStats, Metric, MetricDescriptor, MetricReport, ReportSection, ReportValue},
};

const DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    id: "dwell-time",
    name: "Dwell Time",
    description: "Average time each character key is held down.",
};

struct PressedKey {
    timestamp: SystemTime,
    text: Option<String>,
}

#[derive(Default)]
pub struct DwellTime {
    pressed_keys: HashMap<PhysicalKey, PressedKey>,
    stats: HashMap<String, DurationStats>,
}

impl Metric for DwellTime {
    fn descriptor(&self) -> &'static MetricDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, event: &KeyEvent) {
        match event.kind() {
            KeyEventKind::Press => {
                self.pressed_keys
                    .entry(event.key().clone())
                    .or_insert_with(|| PressedKey {
                        timestamp: event.timestamp(),
                        text: event.text().map(str::to_owned),
                    });
            }
            KeyEventKind::Release => {
                let Some(pressed) = self.pressed_keys.remove(event.key()) else {
                    return;
                };
                let (Some(text), Ok(duration)) = (
                    pressed.text,
                    event.timestamp().duration_since(pressed.timestamp),
                ) else {
                    return;
                };
                self.stats.entry(text).or_default().record(duration);
            }
            KeyEventKind::Repeat => {}
        }
    }

    fn report(&self) -> MetricReport {
        let mut data: Vec<_> = self.stats.iter().collect();
        data.sort_by(|(left_key, left), (right_key, right)| {
            right
                .average_milliseconds()
                .total_cmp(&left.average_milliseconds())
                .then_with(|| left_key.cmp(right_key))
        });

        MetricReport {
            sections: vec![ReportSection::Table {
                title: None,
                columns: &["Key", "Average", "Samples"],
                rows: data
                    .into_iter()
                    .take(5)
                    .map(|(key, stats)| {
                        vec![
                            ReportValue::Text(key.clone()),
                            ReportValue::Milliseconds(stats.average_milliseconds()),
                            ReportValue::Count(stats.samples),
                        ]
                    })
                    .collect(),
            }],
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use crate::{
        input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey},
        metric::Metric,
    };

    use super::DwellTime;

    fn event(at_ms: u64, kind: KeyEventKind, text: Option<&str>) -> KeyEvent {
        KeyEvent::new(
            PhysicalKey::new(30, "A"),
            text.map(str::to_owned),
            SystemTime::UNIX_EPOCH + Duration::from_millis(at_ms),
            kind,
            KeyRole::Other,
        )
    }

    #[test]
    fn uses_text_captured_when_key_was_pressed() {
        let mut metric = DwellTime::default();

        metric.process(&event(100, KeyEventKind::Press, Some("A")));
        metric.process(&event(220, KeyEventKind::Release, Some("a")));

        assert_eq!(metric.stats.get("A").map(|stats| stats.samples), Some(1));
        assert_eq!(metric.stats.get("a").map(|stats| stats.samples), None);
    }
}

use std::{
    collections::{HashMap, HashSet},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::{
    input::{KeyEvent, KeyEventKind, PhysicalKey},
    metric::{
        DurationStats, Metric, MetricDescriptor, MetricReport, MetricSnapshot, MetricSnapshotError,
        ReportSection, ReportValue, validate_dimension, validate_entry_count,
    },
};

const SNAPSHOT_VERSION: u32 = 1;
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotV1 {
    entries: Vec<DurationEntry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DurationEntry {
    text: String,
    total_ns: u64,
    samples: u64,
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

    fn has_data(&self) -> bool {
        !self.stats.is_empty()
    }

    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError> {
        validate_entry_count(DESCRIPTOR.id, self.stats.len())?;
        let mut entries = Vec::with_capacity(self.stats.len());
        for (text, stats) in &self.stats {
            validate_dimension(DESCRIPTOR.id, text)?;
            let (total_ns, samples) = stats.snapshot_parts(DESCRIPTOR.id)?;
            entries.push(DurationEntry {
                text: text.clone(),
                total_ns,
                samples,
            });
        }
        entries.sort_by(|left, right| left.text.cmp(&right.text));
        MetricSnapshot::encode(DESCRIPTOR.id, SNAPSHOT_VERSION, &SnapshotV1 { entries })
    }

    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError> {
        let state: SnapshotV1 = snapshot.decode(DESCRIPTOR.id, SNAPSHOT_VERSION)?;
        validate_entry_count(DESCRIPTOR.id, state.entries.len())?;

        let mut labels = HashSet::with_capacity(state.entries.len());
        let mut stats = HashMap::with_capacity(state.entries.len());
        for entry in state.entries {
            validate_dimension(DESCRIPTOR.id, &entry.text)?;
            if !labels.insert(entry.text.clone()) {
                return Err(MetricSnapshotError::invalid_payload(
                    DESCRIPTOR.id,
                    "duplicate duration dimension",
                ));
            }
            let duration =
                DurationStats::from_snapshot_parts(DESCRIPTOR.id, entry.total_ns, entry.samples)?;
            stats.insert(entry.text, duration);
        }

        *self = Self {
            pressed_keys: HashMap::new(),
            stats,
        };
        Ok(())
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

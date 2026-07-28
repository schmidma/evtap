use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};

use crate::{
    input::{KeyEvent, KeyEventKind},
    metric::{
        DurationStats, Metric, MetricDescriptor, MetricReport, MetricSnapshot, MetricSnapshotError,
        ReportSection, ReportValue, validate_dimension, validate_entry_count,
    },
};

const TYPING_FLOW_TIMEOUT: Duration = Duration::from_secs(2);
const SNAPSHOT_VERSION: u32 = 1;
const DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    id: "flight-time",
    name: "Flight Time",
    description: "Average time from releasing one key to pressing the next character.",
};

#[derive(Default)]
pub struct FlightTime {
    last_release: Option<SystemTime>,
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

impl Metric for FlightTime {
    fn descriptor(&self) -> &'static MetricDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, event: &KeyEvent) {
        match event.kind() {
            KeyEventKind::Press => {
                let (Some(last_release), Some(text)) = (self.last_release, event.text()) else {
                    return;
                };
                let Ok(duration) = event.timestamp().duration_since(last_release) else {
                    return;
                };
                if duration < TYPING_FLOW_TIMEOUT {
                    self.stats
                        .entry(text.to_owned())
                        .or_default()
                        .record(duration);
                }
            }
            KeyEventKind::Release => {
                self.last_release = Some(event.timestamp());
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
            last_release: None,
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

    use super::FlightTime;

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
    fn measures_release_to_next_press() {
        let mut metric = FlightTime::default();

        metric.process(&event(100, KeyEventKind::Release, Some("a")));
        metric.process(&event(175, KeyEventKind::Press, Some("b")));

        assert_eq!(metric.stats.get("b").map(|stats| stats.samples), Some(1));
        assert_eq!(
            metric.stats.get("b").map(|stats| stats.total),
            Some(Duration::from_millis(75))
        );
    }
}

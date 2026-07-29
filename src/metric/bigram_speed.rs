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
const MINIMUM_SAMPLES: u64 = 3;
const SNAPSHOT_VERSION: u32 = 1;
const DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    id: "bigram-speed",
    name: "Bigram Speed",
    description: "Press-to-press timing for character pairs with at least three samples.",
};

#[derive(Default)]
pub struct BigramSpeed {
    last_press: Option<(String, SystemTime)>,
    stats: HashMap<(String, String), DurationStats>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotV1 {
    pairs: Vec<BigramEntry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BigramEntry {
    first: String,
    second: String,
    total_ns: u64,
    samples: u64,
}

impl BigramSpeed {
    fn rows(&self, slowest: bool) -> Vec<Vec<ReportValue>> {
        let mut data: Vec<_> = self
            .stats
            .iter()
            .filter(|(_, stats)| stats.samples >= MINIMUM_SAMPLES)
            .collect();
        data.sort_by(|(left_pair, left), (right_pair, right)| {
            let order = left
                .average_milliseconds()
                .total_cmp(&right.average_milliseconds());
            let order = if slowest { order.reverse() } else { order };
            order.then_with(|| left_pair.cmp(right_pair))
        });

        data.into_iter()
            .take(5)
            .map(|((first, second), stats)| {
                vec![
                    ReportValue::Text(format!("{first} → {second}")),
                    ReportValue::Milliseconds(stats.average_milliseconds()),
                    ReportValue::Count(stats.samples),
                ]
            })
            .collect()
    }
}

impl Metric for BigramSpeed {
    fn descriptor(&self) -> &'static MetricDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, event: &KeyEvent) {
        if event.kind() != KeyEventKind::Press {
            return;
        }
        let Some(text) = event.text() else {
            return;
        };

        if let Some((previous_text, previous_time)) = &self.last_press
            && let Ok(duration) = event.timestamp().duration_since(*previous_time)
            && duration < TYPING_FLOW_TIMEOUT
        {
            self.stats
                .entry((previous_text.clone(), text.to_owned()))
                .or_default()
                .record(duration);
        }
        self.last_press = Some((text.to_owned(), event.timestamp()));
    }

    fn report(&self) -> MetricReport {
        MetricReport {
            sections: vec![
                ReportSection::Table {
                    title: Some("Fastest Pairs"),
                    columns: &["Pair", "Average", "Samples"],
                    rows: self.rows(false),
                },
                ReportSection::Table {
                    title: Some("Slowest Pairs"),
                    columns: &["Pair", "Average", "Samples"],
                    rows: self.rows(true),
                },
            ],
        }
    }

    fn has_data(&self) -> bool {
        !self.stats.is_empty()
    }

    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError> {
        validate_entry_count(DESCRIPTOR.id, self.stats.len())?;
        let mut pairs = Vec::with_capacity(self.stats.len());
        for ((first, second), stats) in &self.stats {
            validate_dimension(DESCRIPTOR.id, first)?;
            validate_dimension(DESCRIPTOR.id, second)?;
            let (total_ns, samples) = stats.snapshot_parts(DESCRIPTOR.id)?;
            pairs.push(BigramEntry {
                first: first.clone(),
                second: second.clone(),
                total_ns,
                samples,
            });
        }
        pairs.sort_by(|left, right| {
            left.first
                .cmp(&right.first)
                .then_with(|| left.second.cmp(&right.second))
        });
        MetricSnapshot::encode(DESCRIPTOR.id, SNAPSHOT_VERSION, &SnapshotV1 { pairs })
    }

    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError> {
        let state: SnapshotV1 = snapshot.decode(DESCRIPTOR.id, SNAPSHOT_VERSION)?;
        validate_entry_count(DESCRIPTOR.id, state.pairs.len())?;

        let mut dimensions = HashSet::with_capacity(state.pairs.len());
        let mut stats = HashMap::with_capacity(state.pairs.len());
        for entry in state.pairs {
            validate_dimension(DESCRIPTOR.id, &entry.first)?;
            validate_dimension(DESCRIPTOR.id, &entry.second)?;
            let pair = (entry.first, entry.second);
            if !dimensions.insert(pair.clone()) {
                return Err(MetricSnapshotError::invalid_payload(
                    DESCRIPTOR.id,
                    "duplicate bigram dimension",
                ));
            }
            let duration =
                DurationStats::from_snapshot_parts(DESCRIPTOR.id, entry.total_ns, entry.samples)?;
            stats.insert(pair, duration);
        }

        *self = Self {
            last_press: None,
            stats,
        };
        Ok(())
    }

    fn clear_in_flight(&mut self) {
        self.last_press = None;
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

    use super::BigramSpeed;

    fn press(at_ms: u64, text: &str) -> KeyEvent {
        KeyEvent::new(
            PhysicalKey::new(30, text.to_uppercase()),
            Some(text.to_owned()),
            SystemTime::UNIX_EPOCH + Duration::from_millis(at_ms),
            KeyEventKind::Press,
            KeyRole::Other,
        )
    }

    #[test]
    fn measures_press_to_press_duration() {
        let mut metric = BigramSpeed::default();

        metric.process(&press(100, "a"));
        metric.process(&press(180, "b"));

        let key = ("a".to_owned(), "b".to_owned());
        assert_eq!(metric.stats.get(&key).map(|stats| stats.samples), Some(1));
        assert_eq!(
            metric.stats.get(&key).map(|stats| stats.total),
            Some(Duration::from_millis(80))
        );
    }

    #[test]
    fn clearing_in_flight_context_preserves_aggregates_without_bridging() {
        let mut metric = BigramSpeed::default();
        metric.process(&press(100, "a"));
        metric.process(&press(180, "b"));
        let aggregates = metric.stats.clone();

        metric.clear_in_flight();
        metric.process(&press(250, "c"));

        assert_eq!(metric.stats, aggregates);
        assert!(!metric.stats.contains_key(&("b".to_owned(), "c".to_owned())));
    }

    #[test]
    fn snapshot_does_not_bridge_bigram_context_across_restore() {
        let mut metric = BigramSpeed::default();
        metric.process(&press(100, "a"));
        metric.process(&press(180, "b"));

        let mut restored = BigramSpeed::default();
        restored.restore(&metric.snapshot().unwrap()).unwrap();
        assert_eq!(restored.stats, metric.stats);
        assert!(restored.last_press.is_none());

        restored.process(&press(250, "c"));
        assert!(
            !restored
                .stats
                .contains_key(&("b".to_owned(), "c".to_owned()))
        );
        restored.process(&press(300, "d"));
        assert_eq!(
            restored
                .stats
                .get(&("c".to_owned(), "d".to_owned()))
                .map(|stats| stats.samples),
            Some(1)
        );

        restored.reset();
        assert!(!restored.has_data());
    }
}

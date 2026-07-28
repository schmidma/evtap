use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    input::{KeyEvent, KeyEventKind, PhysicalKey},
    metric::{
        Metric, MetricDescriptor, MetricReport, MetricSnapshot, MetricSnapshotError, ReportSection,
        ReportValue, validate_count, validate_dimension, validate_entry_count,
    },
};

const SNAPSHOT_VERSION: u32 = 1;
const DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    id: "key-usage",
    name: "Key Usage",
    description: "Physical keys ranked by press count; automatic repeats are excluded.",
};

#[derive(Default)]
pub struct KeyUsage {
    counts: HashMap<PhysicalKey, u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotV1 {
    keys: Vec<KeyCount>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct KeyCount {
    code: u16,
    label: String,
    count: u64,
}

impl Metric for KeyUsage {
    fn descriptor(&self) -> &'static MetricDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, event: &KeyEvent) {
        if event.kind() == KeyEventKind::Press {
            *self.counts.entry(event.key().clone()).or_default() += 1;
        }
    }

    fn report(&self) -> MetricReport {
        let mut counts: Vec<_> = self.counts.iter().collect();
        counts.sort_by(|(left_key, left_count), (right_key, right_count)| {
            right_count
                .cmp(left_count)
                .then_with(|| left_key.label().cmp(right_key.label()))
        });

        MetricReport {
            sections: vec![ReportSection::Table {
                title: None,
                columns: &["Key", "Presses"],
                rows: counts
                    .into_iter()
                    .map(|(key, count)| {
                        vec![
                            ReportValue::Text(key.label().to_owned()),
                            ReportValue::Count(*count),
                        ]
                    })
                    .collect(),
            }],
        }
    }

    fn has_data(&self) -> bool {
        !self.counts.is_empty()
    }

    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError> {
        validate_entry_count(DESCRIPTOR.id, self.counts.len())?;
        let mut keys = Vec::with_capacity(self.counts.len());
        for (key, count) in &self.counts {
            validate_dimension(DESCRIPTOR.id, key.label())?;
            validate_count(DESCRIPTOR.id, *count)?;
            keys.push(KeyCount {
                code: key.code(),
                label: key.label().to_owned(),
                count: *count,
            });
        }
        keys.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| left.label.cmp(&right.label))
        });
        MetricSnapshot::encode(DESCRIPTOR.id, SNAPSHOT_VERSION, &SnapshotV1 { keys })
    }

    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError> {
        let state: SnapshotV1 = snapshot.decode(DESCRIPTOR.id, SNAPSHOT_VERSION)?;
        validate_entry_count(DESCRIPTOR.id, state.keys.len())?;

        let mut codes = HashSet::with_capacity(state.keys.len());
        let mut counts = HashMap::with_capacity(state.keys.len());
        for key in state.keys {
            validate_dimension(DESCRIPTOR.id, &key.label)?;
            validate_count(DESCRIPTOR.id, key.count)?;
            if !codes.insert(key.code) {
                return Err(MetricSnapshotError::invalid_payload(
                    DESCRIPTOR.id,
                    "duplicate physical key code",
                ));
            }
            counts.insert(PhysicalKey::new(key.code, key.label), key.count);
        }

        self.counts = counts;
        Ok(())
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use crate::{
        input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey},
        metric::{Metric, MetricSnapshot},
    };

    use super::KeyUsage;

    fn event(key: PhysicalKey, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new(key, None, SystemTime::UNIX_EPOCH, kind, KeyRole::Other)
    }

    #[test]
    fn ranks_physical_presses() {
        let mut metric = KeyUsage::default();
        let a = PhysicalKey::new(30, "A");
        let b = PhysicalKey::new(48, "B");

        metric.process(&event(a.clone(), KeyEventKind::Press));
        metric.process(&event(b.clone(), KeyEventKind::Press));
        metric.process(&event(b.clone(), KeyEventKind::Repeat));
        metric.process(&event(b, KeyEventKind::Press));

        assert_eq!(metric.counts.get(&a), Some(&1));
        assert_eq!(metric.counts.len(), 2);
    }

    #[test]
    fn snapshot_is_deterministic_and_rejects_duplicate_codes() {
        let a = PhysicalKey::new(30, "A");
        let b = PhysicalKey::new(48, "B");
        let mut first = KeyUsage::default();
        first.process(&event(b.clone(), KeyEventKind::Press));
        first.process(&event(a.clone(), KeyEventKind::Press));
        let mut second = KeyUsage::default();
        second.process(&event(a.clone(), KeyEventKind::Press));
        second.process(&event(b, KeyEventKind::Press));

        let snapshot = first.snapshot().unwrap();
        assert_eq!(
            snapshot.payload_json(),
            second.snapshot().unwrap().payload_json()
        );

        let mut restored = KeyUsage::default();
        restored.restore(&snapshot).unwrap();
        assert_eq!(restored.counts, first.counts);

        let duplicate = MetricSnapshot::from_json(
            "key-usage",
            1,
            r#"{"keys":[{"code":30,"label":"A","count":1},{"code":30,"label":"OTHER","count":1}]}"#
                .to_owned(),
        )
        .unwrap();
        assert!(restored.restore(&duplicate).is_err());
        assert_eq!(restored.counts, first.counts);
    }
}

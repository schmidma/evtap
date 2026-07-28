use std::collections::HashMap;

use crate::{
    input::{KeyEvent, KeyEventKind, PhysicalKey},
    metric::{Metric, MetricDescriptor, MetricReport, ReportSection, ReportValue},
};

const DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    id: "key-usage",
    name: "Key Usage",
    description: "Physical keys ranked by press count; automatic repeats are excluded.",
};

#[derive(Default)]
pub struct KeyUsage {
    counts: HashMap<PhysicalKey, u64>,
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

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use crate::{
        input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey},
        metric::Metric,
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
}

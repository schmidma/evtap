use serde::{Deserialize, Serialize};

use crate::{
    input::{KeyEvent, KeyEventKind},
    metric::{
        Metric, MetricDescriptor, MetricReport, MetricSnapshot, MetricSnapshotError, ReportSection,
        ReportValue, validate_scalar_count,
    },
};

const SNAPSHOT_VERSION: u32 = 1;
const DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    id: "total-presses",
    name: "Total Key Presses",
    description: "Physical key presses in this session; automatic repeats are excluded.",
};

#[derive(Default)]
pub struct TotalPresses {
    count: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotV1 {
    count: u64,
}

impl Metric for TotalPresses {
    fn descriptor(&self) -> &'static MetricDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, event: &KeyEvent) {
        if event.kind() == KeyEventKind::Press {
            self.count += 1;
        }
    }

    fn report(&self) -> MetricReport {
        MetricReport {
            sections: vec![ReportSection::Scalar {
                label: "Presses",
                value: ReportValue::Count(self.count),
            }],
        }
    }

    fn has_data(&self) -> bool {
        self.count > 0
    }

    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError> {
        validate_scalar_count(DESCRIPTOR.id, self.count)?;
        MetricSnapshot::encode(
            DESCRIPTOR.id,
            SNAPSHOT_VERSION,
            &SnapshotV1 { count: self.count },
        )
    }

    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError> {
        let state: SnapshotV1 = snapshot.decode(DESCRIPTOR.id, SNAPSHOT_VERSION)?;
        validate_scalar_count(DESCRIPTOR.id, state.count)?;
        self.count = state.count;
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

    use super::TotalPresses;

    fn event(kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new(
            PhysicalKey::new(30, "A"),
            Some("a".to_owned()),
            SystemTime::UNIX_EPOCH,
            kind,
            KeyRole::Other,
        )
    }

    #[test]
    fn counts_presses_but_not_repeats_or_releases() {
        let mut metric = TotalPresses::default();

        metric.process(&event(KeyEventKind::Press));
        metric.process(&event(KeyEventKind::Repeat));
        metric.process(&event(KeyEventKind::Release));

        assert_eq!(metric.count, 1);
    }

    #[test]
    fn snapshot_round_trips_and_restore_is_all_or_nothing() {
        let mut metric = TotalPresses::default();
        metric.process(&event(KeyEventKind::Press));
        let snapshot = metric.snapshot().unwrap();

        let mut restored = TotalPresses::default();
        restored.restore(&snapshot).unwrap();
        assert!(restored.has_data());
        assert_eq!(restored.count, 1);

        let invalid = MetricSnapshot::from_json(
            "total-presses",
            1,
            r#"{"count":2,"unknown":true}"#.to_owned(),
        )
        .unwrap();
        assert!(restored.restore(&invalid).is_err());
        assert_eq!(restored.count, 1);

        let overflowing = MetricSnapshot::from_json(
            "total-presses",
            1,
            r#"{"count":9223372036854775808}"#.to_owned(),
        )
        .unwrap();
        assert!(restored.restore(&overflowing).is_err());
        assert_eq!(restored.count, 1);

        restored.reset();
        assert!(!restored.has_data());
        assert_eq!(restored.count, 0);
    }
}

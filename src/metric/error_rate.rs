use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::{
    input::{KeyEvent, KeyEventKind, KeyRole},
    metric::{
        Metric, MetricDescriptor, MetricReport, MetricSnapshot, MetricSnapshotError, ReportSection,
        ReportValue, validate_count, validate_dimension, validate_entry_count,
    },
};

const HISTORY_SIZE: usize = 10;
const SNAPSHOT_VERSION: u32 = 1;
const DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    id: "corrections",
    name: "Correction Signals",
    description: "Backspace-based estimates. A confusion is inferred when text immediately follows a deletion.",
};

#[derive(Default)]
pub struct ErrorRate {
    history: VecDeque<String>,
    pending_deleted: Option<String>,
    mistakes: HashMap<String, u64>,
    confusions: HashMap<(String, String), u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotV1 {
    deletions: Vec<DeletionCount>,
    corrections: Vec<CorrectionCount>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DeletionCount {
    text: String,
    count: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CorrectionCount {
    deleted: String,
    typed: String,
    count: u64,
}

impl Metric for ErrorRate {
    fn descriptor(&self) -> &'static MetricDescriptor {
        &DESCRIPTOR
    }

    fn process(&mut self, event: &KeyEvent) {
        if event.kind() == KeyEventKind::Release {
            return;
        }

        if event.role() == KeyRole::Backspace {
            if let Some(deleted) = self.history.pop_back() {
                *self.mistakes.entry(deleted.clone()).or_default() += 1;
                self.pending_deleted = Some(deleted);
            }
            return;
        }

        let Some(text) = event.text() else {
            return;
        };
        if let Some(deleted) = self.pending_deleted.take() {
            *self
                .confusions
                .entry((deleted, text.to_owned()))
                .or_default() += 1;
        }

        self.history.push_back(text.to_owned());
        if self.history.len() > HISTORY_SIZE {
            self.history.pop_front();
        }
    }

    fn report(&self) -> MetricReport {
        let mut mistakes: Vec<_> = self.mistakes.iter().collect();
        mistakes.sort_by(|(left_key, left), (right_key, right)| {
            right.cmp(left).then_with(|| left_key.cmp(right_key))
        });

        let mut confusions: Vec<_> = self.confusions.iter().collect();
        confusions.sort_by(|(left_pair, left), (right_pair, right)| {
            right.cmp(left).then_with(|| left_pair.cmp(right_pair))
        });

        MetricReport {
            sections: vec![
                ReportSection::Table {
                    title: Some("Most Deleted Text"),
                    columns: &["Text", "Deletions"],
                    rows: mistakes
                        .into_iter()
                        .take(5)
                        .map(|(text, count)| {
                            vec![ReportValue::Text(text.clone()), ReportValue::Count(*count)]
                        })
                        .collect(),
                },
                ReportSection::Table {
                    title: Some("Inferred Corrections"),
                    columns: &["Deleted → Typed", "Occurrences"],
                    rows: confusions
                        .into_iter()
                        .take(5)
                        .map(|((deleted, typed), count)| {
                            vec![
                                ReportValue::Text(format!("{deleted} → {typed}")),
                                ReportValue::Count(*count),
                            ]
                        })
                        .collect(),
                },
            ],
        }
    }

    fn has_data(&self) -> bool {
        !self.mistakes.is_empty() || !self.confusions.is_empty()
    }

    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError> {
        let entries = self
            .mistakes
            .len()
            .checked_add(self.confusions.len())
            .ok_or_else(|| {
                MetricSnapshotError::invalid_payload(DESCRIPTOR.id, "too many dimension entries")
            })?;
        validate_entry_count(DESCRIPTOR.id, entries)?;

        let mut deletions = Vec::with_capacity(self.mistakes.len());
        for (text, count) in &self.mistakes {
            validate_dimension(DESCRIPTOR.id, text)?;
            validate_count(DESCRIPTOR.id, *count)?;
            deletions.push(DeletionCount {
                text: text.clone(),
                count: *count,
            });
        }
        deletions.sort_by(|left, right| left.text.cmp(&right.text));

        let mut corrections = Vec::with_capacity(self.confusions.len());
        for ((deleted, typed), count) in &self.confusions {
            validate_dimension(DESCRIPTOR.id, deleted)?;
            validate_dimension(DESCRIPTOR.id, typed)?;
            validate_count(DESCRIPTOR.id, *count)?;
            corrections.push(CorrectionCount {
                deleted: deleted.clone(),
                typed: typed.clone(),
                count: *count,
            });
        }
        corrections.sort_by(|left, right| {
            left.deleted
                .cmp(&right.deleted)
                .then_with(|| left.typed.cmp(&right.typed))
        });

        MetricSnapshot::encode(
            DESCRIPTOR.id,
            SNAPSHOT_VERSION,
            &SnapshotV1 {
                deletions,
                corrections,
            },
        )
    }

    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError> {
        let state: SnapshotV1 = snapshot.decode(DESCRIPTOR.id, SNAPSHOT_VERSION)?;
        let entries = state
            .deletions
            .len()
            .checked_add(state.corrections.len())
            .ok_or_else(|| {
                MetricSnapshotError::invalid_payload(DESCRIPTOR.id, "too many dimension entries")
            })?;
        validate_entry_count(DESCRIPTOR.id, entries)?;

        let mut deletion_labels = HashSet::with_capacity(state.deletions.len());
        let mut mistakes = HashMap::with_capacity(state.deletions.len());
        for deletion in state.deletions {
            validate_dimension(DESCRIPTOR.id, &deletion.text)?;
            validate_count(DESCRIPTOR.id, deletion.count)?;
            if !deletion_labels.insert(deletion.text.clone()) {
                return Err(MetricSnapshotError::invalid_payload(
                    DESCRIPTOR.id,
                    "duplicate deletion dimension",
                ));
            }
            mistakes.insert(deletion.text, deletion.count);
        }

        let mut correction_pairs = HashSet::with_capacity(state.corrections.len());
        let mut confusions = HashMap::with_capacity(state.corrections.len());
        for correction in state.corrections {
            validate_dimension(DESCRIPTOR.id, &correction.deleted)?;
            validate_dimension(DESCRIPTOR.id, &correction.typed)?;
            validate_count(DESCRIPTOR.id, correction.count)?;
            let pair = (correction.deleted, correction.typed);
            if !correction_pairs.insert(pair.clone()) {
                return Err(MetricSnapshotError::invalid_payload(
                    DESCRIPTOR.id,
                    "duplicate correction dimension",
                ));
            }
            confusions.insert(pair, correction.count);
        }

        *self = Self {
            history: VecDeque::new(),
            pending_deleted: None,
            mistakes,
            confusions,
        };
        Ok(())
    }

    fn clear_in_flight(&mut self) {
        self.history.clear();
        self.pending_deleted = None;
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

    use super::ErrorRate;

    fn text(value: &str) -> KeyEvent {
        KeyEvent::new(
            PhysicalKey::new(30, value.to_uppercase()),
            Some(value.to_owned()),
            SystemTime::UNIX_EPOCH,
            KeyEventKind::Press,
            KeyRole::Other,
        )
    }

    fn backspace() -> KeyEvent {
        KeyEvent::new(
            PhysicalKey::new(14, "BACKSPACE"),
            None,
            SystemTime::UNIX_EPOCH,
            KeyEventKind::Press,
            KeyRole::Backspace,
        )
    }

    #[test]
    fn records_deletion_and_following_correction() {
        let mut metric = ErrorRate::default();

        metric.process(&text("o"));
        metric.process(&backspace());
        metric.process(&text("p"));

        assert_eq!(metric.mistakes.get("o"), Some(&1));
        assert_eq!(
            metric.confusions.get(&("o".to_owned(), "p".to_owned())),
            Some(&1)
        );
    }

    #[test]
    fn clearing_in_flight_context_drops_recent_text_and_pending_correction() {
        let mut metric = ErrorRate::default();
        metric.process(&text("o"));
        metric.process(&backspace());
        let mistakes = metric.mistakes.clone();

        metric.clear_in_flight();
        metric.process(&text("p"));

        assert_eq!(metric.mistakes, mistakes);
        assert!(metric.confusions.is_empty());
        assert_eq!(metric.history.back().map(String::as_str), Some("p"));
    }

    #[test]
    fn snapshot_restores_aggregates_without_transient_text_context() {
        let mut metric = ErrorRate::default();
        metric.process(&text("o"));
        metric.process(&backspace());
        assert_eq!(metric.pending_deleted.as_deref(), Some("o"));

        let snapshot = metric.snapshot().unwrap();
        let mut restored = ErrorRate::default();
        restored.restore(&snapshot).unwrap();

        assert_eq!(restored.mistakes.get("o"), Some(&1));
        assert!(restored.history.is_empty());
        assert!(restored.pending_deleted.is_none());
        restored.process(&text("p"));
        assert!(restored.confusions.is_empty());

        restored.process(&backspace());
        assert_eq!(restored.mistakes.get("p"), Some(&1));
        restored.reset();
        assert!(!restored.has_data());
        assert!(restored.history.is_empty());
    }

    #[test]
    fn snapshot_round_trips_correction_counts() {
        let mut metric = ErrorRate::default();
        metric.process(&text("o"));
        metric.process(&backspace());
        metric.process(&text("p"));

        let mut restored = ErrorRate::default();
        restored.restore(&metric.snapshot().unwrap()).unwrap();

        assert_eq!(restored.mistakes, metric.mistakes);
        assert_eq!(restored.confusions, metric.confusions);
    }
}

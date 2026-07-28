use std::collections::{HashMap, VecDeque};

use crate::{
    input::{KeyEvent, KeyEventKind, KeyRole},
    metric::{Metric, MetricDescriptor, MetricReport, ReportSection, ReportValue},
};

const HISTORY_SIZE: usize = 10;
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
}

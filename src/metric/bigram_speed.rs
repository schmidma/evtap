use std::{collections::HashMap, time::Duration, time::SystemTime};

use crate::{
    input::{KeyEvent, KeyEventKind},
    metric::{DurationStats, Metric, MetricDescriptor, MetricReport, ReportSection, ReportValue},
};

const TYPING_FLOW_TIMEOUT: Duration = Duration::from_secs(2);
const MINIMUM_SAMPLES: u64 = 3;
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
}

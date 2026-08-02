use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime},
};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::{
    app::view::components::{
        TextTokenContext, describe_text_token, disclosure_list, format_duration_ms,
        format_exact_count, inline_empty_state, ranked_bar_with_label, section_title,
        summary_value, text_token,
    },
    input::{KeyEvent, KeyEventKind},
    metric::{
        DurationStats, Metric, MetricSnapshot, MetricSnapshotError, validate_dimension,
        validate_entry_count,
    },
};

const METRIC_ID: &str = "bigram-speed";
const TYPING_FLOW_TIMEOUT: Duration = Duration::from_secs(2);
const MINIMUM_SAMPLES: u64 = 3;
const SNAPSHOT_VERSION: u32 = 1;
const INITIAL_PANEL_ROWS: usize = 5;
const SIDE_BY_SIDE_WIDTH: f32 = 640.0;

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
    fn sorted_pairs(&self, slowest: bool) -> Vec<((&str, &str), DurationStats)> {
        let mut data: Vec<_> = self
            .stats
            .iter()
            .filter(|(_, stats)| stats.samples >= MINIMUM_SAMPLES)
            .map(|((first, second), stats)| ((first.as_str(), second.as_str()), *stats))
            .collect();
        data.sort_by(|(left_pair, left), (right_pair, right)| {
            let order = left.compare_average(*right);
            let order = if slowest { order.reverse() } else { order };
            order.then_with(|| left_pair.cmp(right_pair))
        });
        data
    }

    fn maximum_qualifying_average(&self) -> f64 {
        self.stats
            .values()
            .filter(|stats| stats.samples >= MINIMUM_SAMPLES)
            .copied()
            .max_by(|left, right| left.compare_average(*right))
            .map(DurationStats::average_milliseconds)
            .unwrap_or(0.0)
    }
}

fn expansion_state_id(panel: &'static str) -> egui::Id {
    egui::Id::new((METRIC_ID, "analysis-expansion-state", panel))
}

fn sample_count_label(samples: u64) -> String {
    let noun = if samples == 1 { "sample" } else { "samples" };
    format!("{} {noun}", format_exact_count(samples))
}

fn bigram_empty_state(ui: &mut egui::Ui) {
    inline_empty_state(
        ui,
        egui_phosphor::regular::CLOCK,
        "No bigram samples yet",
        "Press-to-press timing for consecutive produced text will appear here when captured.",
    );
}

fn insufficient_samples_state(ui: &mut egui::Ui) {
    inline_empty_state(
        ui,
        egui_phosphor::regular::CLOCK,
        "Not enough samples yet",
        "Each pair needs 3 samples before it is shown.",
    );
}

fn pair_tokens(ui: &mut egui::Ui, first: &str, second: &str) {
    text_token(ui, first, TextTokenContext::ProducedText);
    ui.label("→");
    text_token(ui, second, TextTokenContext::ProducedText);
}

fn pair_summary(ui: &mut egui::Ui, heading: &str, first: &str, second: &str, stats: DurationStats) {
    ui.label(egui::RichText::new(heading).strong());
    ui.horizontal(|ui| pair_tokens(ui, first, second));
    let average = format_duration_ms(stats.average_milliseconds());
    summary_value(
        ui,
        "Average press-to-press",
        &average,
        &average,
        &sample_count_label(stats.samples),
    );
}

fn pair_bar(ui: &mut egui::Ui, first: &str, second: &str, stats: DurationStats, maximum: f64) {
    let average_value = stats.average_milliseconds();
    let average = format_duration_ms(average_value);
    let samples = sample_count_label(stats.samples);
    let visible_value = format!("{average} · {samples}");
    let first_description = describe_text_token(first, TextTokenContext::ProducedText);
    let second_description = describe_text_token(second, TextTokenContext::ProducedText);
    let accessible_label = format!(
        "Bigram from {} to {}: {average} average press-to-press across {samples}",
        first_description.accessible_label, second_description.accessible_label
    );
    ranked_bar_with_label(
        ui,
        &accessible_label,
        average_value,
        maximum,
        &visible_value,
        |ui| pair_tokens(ui, first, second),
    );
}

fn pair_panel(
    ui: &mut egui::Ui,
    heading: &str,
    panel_id: &'static str,
    pairs: &[((&str, &str), DurationStats)],
    maximum: f64,
) {
    ui.push_id((METRIC_ID, panel_id), |ui| {
        ui.set_width(ui.available_width());
        section_title(ui, heading);
        let total_rows = pairs.len();
        disclosure_list(
            ui,
            expansion_state_id(panel_id),
            total_rows,
            INITIAL_PANEL_ROWS,
            |ui, shown_rows| {
                for ((first, second), stats) in pairs.iter().copied().take(shown_rows) {
                    pair_bar(ui, first, second, stats, maximum);
                }
            },
        );
    });
}

impl Metric for BigramSpeed {
    const ID: &'static str = METRIC_ID;

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

    fn summary_ui(&self, ui: &mut egui::Ui) {
        if self.stats.is_empty() {
            bigram_empty_state(ui);
            return;
        }
        let fastest = self.sorted_pairs(false);
        if fastest.is_empty() {
            insufficient_samples_state(ui);
            return;
        }
        let slowest = self.sorted_pairs(true);
        let ((fastest_first, fastest_second), fastest_stats) = fastest[0];
        let ((slowest_first, slowest_second), slowest_stats) = slowest[0];
        pair_summary(ui, "Fastest", fastest_first, fastest_second, fastest_stats);
        ui.separator();
        pair_summary(ui, "Slowest", slowest_first, slowest_second, slowest_stats);
    }

    fn analysis_ui(&self, ui: &mut egui::Ui) {
        if self.stats.is_empty() {
            bigram_empty_state(ui);
            return;
        }
        let fastest = self.sorted_pairs(false);
        if fastest.is_empty() {
            insufficient_samples_state(ui);
            return;
        }
        let slowest = self.sorted_pairs(true);
        let maximum = self.maximum_qualifying_average();
        if ui.available_width() >= SIDE_BY_SIDE_WIDTH {
            ui.columns(2, |columns| {
                pair_panel(
                    &mut columns[0],
                    "Fastest pairs",
                    "fastest",
                    &fastest,
                    maximum,
                );
                pair_panel(
                    &mut columns[1],
                    "Slowest pairs",
                    "slowest",
                    &slowest,
                    maximum,
                );
            });
        } else {
            pair_panel(ui, "Fastest pairs", "fastest", &fastest, maximum);
            ui.add_space(8.0);
            pair_panel(ui, "Slowest pairs", "slowest", &slowest, maximum);
        }
    }

    fn has_data(&self) -> bool {
        !self.stats.is_empty()
    }

    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError> {
        validate_entry_count(Self::ID, self.stats.len())?;
        let mut pairs = Vec::with_capacity(self.stats.len());
        for ((first, second), stats) in &self.stats {
            validate_dimension(Self::ID, first)?;
            validate_dimension(Self::ID, second)?;
            let (total_ns, samples) = stats.snapshot_parts(Self::ID)?;
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
        MetricSnapshot::encode(Self::ID, SNAPSHOT_VERSION, &SnapshotV1 { pairs })
    }

    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError> {
        let state: SnapshotV1 = snapshot.decode(Self::ID, SNAPSHOT_VERSION)?;
        validate_entry_count(Self::ID, state.pairs.len())?;

        let mut dimensions = HashSet::with_capacity(state.pairs.len());
        let mut stats = HashMap::with_capacity(state.pairs.len());
        for entry in state.pairs {
            validate_dimension(Self::ID, &entry.first)?;
            validate_dimension(Self::ID, &entry.second)?;
            let pair = (entry.first, entry.second);
            if !dimensions.insert(pair.clone()) {
                return Err(MetricSnapshotError::invalid_payload(
                    Self::ID,
                    "duplicate bigram dimension",
                ));
            }
            let duration =
                DurationStats::from_snapshot_parts(Self::ID, entry.total_ns, entry.samples)?;
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

    use eframe::egui;
    use egui_kittest::{Harness, kittest::Queryable};

    use crate::{
        input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey},
        metric::{DurationStats, Metric},
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

    fn duration_stats(total_ms: u64, samples: u64) -> DurationStats {
        DurationStats {
            total: Duration::from_millis(total_ms),
            samples,
        }
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
    fn threshold_and_average_ordering_are_exact_and_deterministic() {
        let mut metric = BigramSpeed::default();
        metric.stats.insert(
            ("below".to_owned(), "threshold".to_owned()),
            duration_stats(20, 2),
        );
        metric
            .stats
            .insert(("b".to_owned(), "pair".to_owned()), duration_stats(300, 3));
        metric
            .stats
            .insert(("a".to_owned(), "pair".to_owned()), duration_stats(300, 3));
        metric.stats.insert(
            ("slow".to_owned(), "pair".to_owned()),
            duration_stats(600, 3),
        );

        let fastest: Vec<_> = metric
            .sorted_pairs(false)
            .into_iter()
            .map(|(pair, _)| pair)
            .collect();
        assert_eq!(fastest, [("a", "pair"), ("b", "pair"), ("slow", "pair")]);
        let slowest: Vec<_> = metric
            .sorted_pairs(true)
            .into_iter()
            .map(|(pair, _)| pair)
            .collect();
        assert_eq!(slowest, [("slow", "pair"), ("a", "pair"), ("b", "pair")]);

        metric.stats.clear();
        metric.stats.insert(
            ("alphabetically".to_owned(), "first".to_owned()),
            DurationStats {
                total: Duration::from_nanos(i64::MAX as u64 - 1),
                samples: 3,
            },
        );
        metric.stats.insert(
            ("truly".to_owned(), "slower".to_owned()),
            DurationStats {
                total: Duration::from_nanos(i64::MAX as u64),
                samples: 3,
            },
        );
        let slowest: Vec<_> = metric
            .sorted_pairs(true)
            .into_iter()
            .map(|(pair, _)| pair)
            .collect();
        assert_eq!(slowest, [("truly", "slower"), ("alphabetically", "first")]);
    }

    #[test]
    fn summary_distinguishes_empty_and_insufficient_samples() {
        let empty_metric = BigramSpeed::default();
        let empty = Harness::new_ui(move |ui| empty_metric.summary_ui(ui));
        assert!(empty.query_by_label("No bigram samples yet").is_some());

        let mut insufficient_metric = BigramSpeed::default();
        insufficient_metric
            .stats
            .insert(("a".to_owned(), "b".to_owned()), duration_stats(100, 2));
        let insufficient = Harness::new_ui(move |ui| insufficient_metric.summary_ui(ui));
        assert!(
            insufficient
                .query_by_label("Not enough samples yet")
                .is_some()
        );
        assert!(
            insufficient
                .query_by_label("Each pair needs 3 samples before it is shown.")
                .is_some()
        );
    }

    #[test]
    fn analysis_pairs_are_accessible_and_panels_expand_independently() {
        let mut metric = BigramSpeed::default();
        metric
            .stats
            .insert(("\n".to_owned(), " ".to_owned()), duration_stats(300, 3));
        for index in 0..5 {
            metric.stats.insert(
                (format!("first-{index}"), format!("second-{index}")),
                duration_stats(300 + index * 3, 3),
            );
        }
        let mut harness = Harness::new_ui(move |ui| metric.analysis_ui(ui));

        assert_eq!(
            harness
                .query_all_by_role_and_label(
                    egui::accesskit::Role::ProgressIndicator,
                    "Bigram from Produced text: Newline to Produced text: Space: 100.0 ms average press-to-press across 3 samples",
                )
                .count(),
            2
        );
        assert_eq!(harness.query_all_by_label("Showing 5 of 6").count(), 2);
        harness
            .query_all_by_label("Show 1 more")
            .next()
            .expect("one panel should expose its expansion action")
            .click();
        harness.run_steps(2);
        assert_eq!(harness.query_all_by_label("Showing 6 of 6").count(), 1);
        assert_eq!(harness.query_all_by_label("Showing 5 of 6").count(), 1);
        assert_eq!(harness.query_all_by_label("Show fewer").count(), 1);
        assert_eq!(harness.query_all_by_label("Show 1 more").count(), 1);
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

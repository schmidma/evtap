use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime},
};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::{
    app::view::components::{
        TextTokenContext, describe_text_token, disclosure_list, format_duration_ms,
        format_exact_count_u128, inline_empty_state, metric_summary_value, ranked_bar_with_label,
        text_token,
    },
    input::{KeyEvent, KeyEventKind},
    metric::{
        DurationStats, Metric, MetricSnapshot, MetricSnapshotError, validate_dimension,
        validate_entry_count,
    },
};

const METRIC_ID: &str = "flight-time";
const TYPING_FLOW_TIMEOUT: Duration = Duration::from_secs(2);
const SNAPSHOT_VERSION: u32 = 1;
const INITIAL_ANALYSIS_ROWS: usize = 8;

#[derive(Default)]
pub struct FlightTime {
    last_release: Option<SystemTime>,
    stats: HashMap<String, DurationStats>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SortChoice {
    #[default]
    Slowest,
    Fastest,
    Label,
    SampleCount,
}

impl SortChoice {
    fn label(self) -> &'static str {
        match self {
            Self::Slowest => "Slowest",
            Self::Fastest => "Fastest",
            Self::Label => "Label",
            Self::SampleCount => "Sample count",
        }
    }
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

impl FlightTime {
    fn overall_average(&self) -> Option<(f64, u128)> {
        let samples = self
            .stats
            .values()
            .map(|stats| u128::from(stats.samples))
            .sum::<u128>();
        if samples == 0 {
            return None;
        }
        let total_nanoseconds = self
            .stats
            .values()
            .map(|stats| stats.total.as_nanos())
            .sum::<u128>();
        Some((
            total_nanoseconds as f64 / samples as f64 / 1_000_000.0,
            samples,
        ))
    }

    fn sorted_stats(&self, sort: SortChoice) -> Vec<(&str, DurationStats)> {
        let mut stats: Vec<_> = self
            .stats
            .iter()
            .map(|(text, stats)| (text.as_str(), *stats))
            .collect();
        stats.sort_by(|(left_text, left), (right_text, right)| match sort {
            SortChoice::Slowest => right
                .compare_average(*left)
                .then_with(|| left_text.cmp(right_text)),
            SortChoice::Fastest => left
                .compare_average(*right)
                .then_with(|| left_text.cmp(right_text)),
            SortChoice::Label => left_text.cmp(right_text),
            SortChoice::SampleCount => right
                .samples
                .cmp(&left.samples)
                .then_with(|| left_text.cmp(right_text)),
        });
        stats
    }

    fn maximum_average(&self) -> f64 {
        self.stats
            .values()
            .copied()
            .max_by(|left, right| left.compare_average(*right))
            .map(DurationStats::average_milliseconds)
            .unwrap_or(0.0)
    }
}

fn sort_state_id() -> egui::Id {
    egui::Id::new((METRIC_ID, "analysis-sort-state"))
}

fn expansion_state_id() -> egui::Id {
    egui::Id::new((METRIC_ID, "analysis-expansion-state"))
}

fn sample_count_label(samples: u128) -> String {
    let count = format_exact_count_u128(samples);
    let noun = if samples == 1 { "sample" } else { "samples" };
    format!("{count} {noun}")
}

fn flight_empty_state(ui: &mut egui::Ui) {
    inline_empty_state(
        ui,
        egui_phosphor::regular::CLOCK,
        "No flight samples yet",
        "Timing from release to the next text-producing press will appear here when captured.",
    );
}

fn flight_bar(ui: &mut egui::Ui, text: &str, stats: DurationStats, maximum: f64) {
    let average = stats.average_milliseconds();
    let visible_average = format_duration_ms(average);
    let samples = sample_count_label(u128::from(stats.samples));
    let visible_value = format!("{visible_average} · {samples}");
    let token = describe_text_token(text, TextTokenContext::ProducedText);
    let accessible_label = format!(
        "Flight time to destination {}: {visible_average} average across {samples}, measured from release to the next text-producing press",
        token.accessible_label
    );
    ranked_bar_with_label(
        ui,
        &accessible_label,
        average,
        maximum,
        &visible_value,
        |ui| {
            text_token(ui, text, TextTokenContext::ProducedText);
        },
    );
}

impl Metric for FlightTime {
    const ID: &'static str = METRIC_ID;

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

    fn summary_ui(&self, ui: &mut egui::Ui) {
        let Some((average, samples)) = self.overall_average() else {
            flight_empty_state(ui);
            return;
        };
        let visible_average = format_duration_ms(average);
        metric_summary_value(
            ui,
            egui_phosphor::regular::PAPER_PLANE_TILT,
            "Average flight time",
            &visible_average,
            &visible_average,
            &format!("Across {}.", sample_count_label(samples)),
        );
    }

    fn analysis_ui(&self, ui: &mut egui::Ui) {
        if self.stats.is_empty() {
            flight_empty_state(ui);
            return;
        }

        let sort_id = sort_state_id();
        let mut sort = ui
            .ctx()
            .data(|data| data.get_temp::<SortChoice>(sort_id))
            .unwrap_or_default();
        let previous_sort = sort;
        ui.horizontal(|ui| {
            ui.label("Sort");
            egui::ComboBox::from_id_salt((METRIC_ID, "analysis-sort-control"))
                .selected_text(sort.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut sort,
                        SortChoice::Slowest,
                        SortChoice::Slowest.label(),
                    );
                    ui.selectable_value(
                        &mut sort,
                        SortChoice::Fastest,
                        SortChoice::Fastest.label(),
                    );
                    ui.selectable_value(&mut sort, SortChoice::Label, SortChoice::Label.label());
                    ui.selectable_value(
                        &mut sort,
                        SortChoice::SampleCount,
                        SortChoice::SampleCount.label(),
                    );
                });
        });
        if sort != previous_sort {
            ui.ctx().data_mut(|data| data.insert_temp(sort_id, sort));
        }

        let stats = self.sorted_stats(sort);
        let total_rows = stats.len();
        let maximum = self.maximum_average();
        disclosure_list(
            ui,
            expansion_state_id(),
            total_rows,
            INITIAL_ANALYSIS_ROWS,
            |ui, shown_rows| {
                for (text, stats) in stats.into_iter().take(shown_rows) {
                    flight_bar(ui, text, stats, maximum);
                }
            },
        );
    }

    fn has_data(&self) -> bool {
        !self.stats.is_empty()
    }

    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError> {
        validate_entry_count(Self::ID, self.stats.len())?;
        let mut entries = Vec::with_capacity(self.stats.len());
        for (text, stats) in &self.stats {
            validate_dimension(Self::ID, text)?;
            let (total_ns, samples) = stats.snapshot_parts(Self::ID)?;
            entries.push(DurationEntry {
                text: text.clone(),
                total_ns,
                samples,
            });
        }
        entries.sort_by(|left, right| left.text.cmp(&right.text));
        MetricSnapshot::encode(Self::ID, SNAPSHOT_VERSION, &SnapshotV1 { entries })
    }

    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError> {
        let state: SnapshotV1 = snapshot.decode(Self::ID, SNAPSHOT_VERSION)?;
        validate_entry_count(Self::ID, state.entries.len())?;

        let mut labels = HashSet::with_capacity(state.entries.len());
        let mut stats = HashMap::with_capacity(state.entries.len());
        for entry in state.entries {
            validate_dimension(Self::ID, &entry.text)?;
            if !labels.insert(entry.text.clone()) {
                return Err(MetricSnapshotError::invalid_payload(
                    Self::ID,
                    "duplicate duration dimension",
                ));
            }
            let duration =
                DurationStats::from_snapshot_parts(Self::ID, entry.total_ns, entry.samples)?;
            stats.insert(entry.text, duration);
        }

        *self = Self {
            last_release: None,
            stats,
        };
        Ok(())
    }

    fn clear_in_flight(&mut self) {
        self.last_release = None;
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
        app::view::components::{TextTokenContext, describe_text_token},
        input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey},
        metric::{DurationStats, Metric},
    };

    use super::{FlightTime, SortChoice};

    fn event(at_ms: u64, kind: KeyEventKind, text: Option<&str>) -> KeyEvent {
        KeyEvent::new(
            PhysicalKey::new(30, "A"),
            text.map(str::to_owned),
            SystemTime::UNIX_EPOCH + Duration::from_millis(at_ms),
            kind,
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

    #[test]
    fn overall_average_is_weighted_by_samples() {
        let mut metric = FlightTime::default();
        metric.stats.insert("a".to_owned(), duration_stats(100, 2));
        metric.stats.insert("b".to_owned(), duration_stats(100, 1));
        metric.stats.insert("c".to_owned(), duration_stats(150, 3));

        let (average, samples) = metric
            .overall_average()
            .expect("populated flight stats should have an average");
        assert_eq!(samples, 6);
        assert!((average - 350.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn sorting_is_deterministic_for_averages_labels_and_sample_counts() {
        let mut metric = FlightTime::default();
        metric
            .stats
            .insert("alpha".to_owned(), duration_stats(100, 2));
        metric
            .stats
            .insert("beta".to_owned(), duration_stats(100, 1));
        metric
            .stats
            .insert("delta".to_owned(), duration_stats(150, 3));

        let order = |sort| {
            metric
                .sorted_stats(sort)
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
        };
        assert_eq!(order(SortChoice::Slowest), ["beta", "alpha", "delta"]);
        assert_eq!(order(SortChoice::Fastest), ["alpha", "delta", "beta"]);
        assert_eq!(order(SortChoice::Label), ["alpha", "beta", "delta"]);
        assert_eq!(order(SortChoice::SampleCount), ["delta", "alpha", "beta"]);

        metric.stats.clear();
        metric.stats.insert(
            "alphabetically-first".to_owned(),
            DurationStats {
                total: Duration::from_nanos(i64::MAX as u64 - 1),
                samples: 1,
            },
        );
        metric.stats.insert(
            "truly-slower".to_owned(),
            DurationStats {
                total: Duration::from_nanos(i64::MAX as u64),
                samples: 1,
            },
        );
        let order = |sort| {
            metric
                .sorted_stats(sort)
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            order(SortChoice::Slowest),
            ["truly-slower", "alphabetically-first"]
        );
    }

    #[test]
    fn analysis_rows_explain_destination_and_expansion_uses_temporary_state() {
        let mut metric = FlightTime::default();
        for index in 0..9 {
            metric
                .stats
                .insert(format!("text-{index}"), duration_stats(75, 1));
        }
        let mut harness = Harness::new_ui(move |ui| {
            metric.analysis_ui(ui);
        });

        let token = describe_text_token("text-0", TextTokenContext::ProducedText);
        let accessible_label = format!(
            "Flight time to destination {}: 75.0 ms average across 1 sample, measured from release to the next text-producing press",
            token.accessible_label
        );
        assert!(
            harness
                .query_by_role_and_label(
                    egui::accesskit::Role::ProgressIndicator,
                    &accessible_label,
                )
                .is_some()
        );
        assert!(harness.query_by_label("Showing 8 of 9").is_some());
        harness.get_by_label("Show 1 more").click();
        harness.step();
        harness.step();
        assert!(harness.query_by_label("Showing 9 of 9").is_some());
        assert!(harness.query_by_label("Show fewer").is_some());
    }

    #[test]
    fn clearing_in_flight_context_preserves_aggregates_without_bridging() {
        let mut metric = FlightTime::default();
        metric.process(&event(100, KeyEventKind::Release, Some("a")));

        metric.clear_in_flight();
        metric.process(&event(175, KeyEventKind::Press, Some("b")));

        assert!(metric.stats.is_empty());
    }

    #[test]
    fn snapshot_does_not_bridge_release_context_across_restore() {
        let mut metric = FlightTime::default();
        metric.process(&event(100, KeyEventKind::Release, Some("a")));
        metric.process(&event(175, KeyEventKind::Press, Some("b")));
        metric.process(&event(200, KeyEventKind::Release, Some("b")));

        let mut restored = FlightTime::default();
        let snapshot = metric.snapshot().expect("flight snapshot should encode");
        restored
            .restore(&snapshot)
            .expect("flight snapshot should restore");
        assert_eq!(restored.stats, metric.stats);
        assert!(restored.last_release.is_none());

        restored.process(&event(250, KeyEventKind::Press, Some("c")));
        assert!(!restored.stats.contains_key("c"));
        restored.process(&event(300, KeyEventKind::Release, Some("c")));
        restored.process(&event(350, KeyEventKind::Press, Some("d")));
        assert_eq!(restored.stats.get("d").map(|stats| stats.samples), Some(1));

        restored.reset();
        assert!(!restored.has_data());
    }
}

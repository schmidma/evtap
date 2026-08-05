use std::{
    collections::{HashMap, HashSet},
    time::SystemTime,
};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::{
    app::view::components::{
        TextTokenContext, describe_text_token, format_duration_ms, inline_empty_state,
        metric_summary_value, ranked_bar_with_label, text_token,
    },
    input::{KeyEvent, KeyEventKind, PhysicalKey},
    metric::{
        DurationStats, Metric, MetricSnapshot, MetricSnapshotError,
        duration_analysis::{overall_average, render_duration_analysis, sample_count_label},
        validate_dimension, validate_entry_count,
    },
};

const METRIC_ID: &str = "dwell-time";
const SNAPSHOT_VERSION: u32 = 1;

struct PressedKey {
    timestamp: SystemTime,
    text: Option<String>,
}

#[derive(Default)]
pub struct DwellTime {
    pressed_keys: HashMap<PhysicalKey, PressedKey>,
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

fn dwell_empty_state(ui: &mut egui::Ui) {
    inline_empty_state(
        ui,
        egui_phosphor::regular::CLOCK,
        "No dwell samples yet",
        "Character hold durations will appear here when they are captured.",
    );
}

fn dwell_bar(ui: &mut egui::Ui, text: &str, stats: DurationStats, maximum: f64) {
    let average = stats.average_milliseconds();
    let visible_average = format_duration_ms(average);
    let samples = sample_count_label(u128::from(stats.samples));
    let visible_value = format!("{visible_average} · {samples}");
    let token = describe_text_token(text, TextTokenContext::ProducedText);
    let accessible_label = format!(
        "Dwell time for {}: {visible_average} average across {samples}",
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

impl Metric for DwellTime {
    const ID: &'static str = METRIC_ID;

    fn process(&mut self, event: &KeyEvent) {
        match event.kind() {
            KeyEventKind::Press => {
                self.pressed_keys
                    .entry(event.key().clone())
                    .or_insert_with(|| PressedKey {
                        timestamp: event.timestamp(),
                        text: event.text().map(str::to_owned),
                    });
            }
            KeyEventKind::Release => {
                let Some(pressed) = self.pressed_keys.remove(event.key()) else {
                    return;
                };
                let (Some(text), Ok(duration)) = (
                    pressed.text,
                    event.timestamp().duration_since(pressed.timestamp),
                ) else {
                    return;
                };
                self.stats.entry(text).or_default().record(duration);
            }
            KeyEventKind::Repeat => {}
        }
    }

    fn summary_ui(&self, ui: &mut egui::Ui) {
        let Some((average, samples)) = overall_average(&self.stats) else {
            dwell_empty_state(ui);
            return;
        };
        let visible_average = format_duration_ms(average);
        metric_summary_value(
            ui,
            egui_phosphor::regular::TIMER,
            "Average dwell time",
            &visible_average,
            &visible_average,
            &format!("Across {}.", sample_count_label(samples)),
        );
    }

    fn analysis_ui(&self, ui: &mut egui::Ui) {
        if self.stats.is_empty() {
            dwell_empty_state(ui);
            return;
        }

        render_duration_analysis(ui, METRIC_ID, &self.stats, dwell_bar);
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
            pressed_keys: HashMap::new(),
            stats,
        };
        Ok(())
    }

    fn clear_in_flight(&mut self) {
        self.pressed_keys.clear();
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

    use super::DwellTime;

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
    fn uses_text_captured_when_key_was_pressed() {
        let mut metric = DwellTime::default();

        metric.process(&event(100, KeyEventKind::Press, Some("A")));
        metric.process(&event(220, KeyEventKind::Release, Some("a")));

        assert_eq!(metric.stats.get("A").map(|stats| stats.samples), Some(1));
        assert_eq!(metric.stats.get("a").map(|stats| stats.samples), None);
    }

    #[test]
    fn analysis_rows_are_accessible_and_expansion_uses_temporary_state() {
        let mut metric = DwellTime::default();
        for index in 0..9 {
            metric
                .stats
                .insert(format!("text-{index}"), duration_stats(100, 1));
        }
        let mut harness = Harness::new_ui(move |ui| {
            metric.analysis_ui(ui);
        });

        let token = describe_text_token("text-0", TextTokenContext::ProducedText);
        let accessible_label = format!(
            "Dwell time for {}: 100.0 ms average across 1 sample",
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
    fn clearing_in_flight_context_drops_unfinished_presses() {
        let mut metric = DwellTime::default();
        metric.process(&event(100, KeyEventKind::Press, Some("a")));

        metric.clear_in_flight();
        metric.process(&event(180, KeyEventKind::Release, Some("a")));

        assert!(metric.pressed_keys.is_empty());
        assert!(!metric.stats.contains_key("a"));
    }

    #[test]
    fn snapshot_does_not_restore_pressed_keys() {
        let mut metric = DwellTime::default();
        metric.process(&event(100, KeyEventKind::Press, Some("a")));
        metric.process(&event(180, KeyEventKind::Release, Some("a")));
        metric.process(&event(200, KeyEventKind::Press, Some("b")));
        assert_eq!(metric.pressed_keys.len(), 1);

        let mut restored = DwellTime::default();
        let snapshot = metric.snapshot().expect("dwell snapshot should encode");
        restored
            .restore(&snapshot)
            .expect("dwell snapshot should restore");
        assert_eq!(restored.stats, metric.stats);
        assert!(restored.pressed_keys.is_empty());

        restored.process(&event(260, KeyEventKind::Release, Some("b")));
        assert!(!restored.stats.contains_key("b"));
        restored.reset();
        assert!(!restored.has_data());
    }
}

use std::collections::{HashMap, HashSet, VecDeque};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::{
    app::view::components::{
        TextTokenContext, describe_text_token, disclosure_list, format_exact_count,
        format_exact_count_u128, inline_empty_state, ranked_bar_with_label, section_title,
        summary_value, text_token,
    },
    input::{KeyEvent, KeyEventKind, KeyRole},
    metric::{
        Metric, MetricSnapshot, MetricSnapshotError, validate_count, validate_dimension,
        validate_entry_count,
    },
};

const METRIC_ID: &str = "corrections";
const HISTORY_SIZE: usize = 10;
const SNAPSHOT_VERSION: u32 = 1;
const INITIAL_PANEL_ROWS: usize = 6;
const SIDE_BY_SIDE_WIDTH: f32 = 640.0;

#[derive(Default)]
pub struct CorrectionSignals {
    history: VecDeque<String>,
    pending_deleted: Option<String>,
    mistakes: HashMap<String, u64>,
    confusions: HashMap<(String, String), u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SortChoice {
    #[default]
    OccurrenceCount,
    Label,
}

impl SortChoice {
    fn label(self) -> &'static str {
        match self {
            Self::OccurrenceCount => "Occurrence count",
            Self::Label => "Label",
        }
    }
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

impl CorrectionSignals {
    fn sorted_mistakes(&self, sort: SortChoice) -> Vec<(&str, u64)> {
        let mut mistakes: Vec<_> = self
            .mistakes
            .iter()
            .map(|(text, count)| (text.as_str(), *count))
            .collect();
        mistakes.sort_by(
            |(left_text, left_count), (right_text, right_count)| match sort {
                SortChoice::OccurrenceCount => right_count
                    .cmp(left_count)
                    .then_with(|| left_text.cmp(right_text)),
                SortChoice::Label => left_text.cmp(right_text),
            },
        );
        mistakes
    }

    fn sorted_confusions(&self, sort: SortChoice) -> Vec<((&str, &str), u64)> {
        let mut confusions: Vec<_> = self
            .confusions
            .iter()
            .map(|((deleted, typed), count)| ((deleted.as_str(), typed.as_str()), *count))
            .collect();
        confusions.sort_by(
            |(left_pair, left_count), (right_pair, right_count)| match sort {
                SortChoice::OccurrenceCount => right_count
                    .cmp(left_count)
                    .then_with(|| left_pair.cmp(right_pair)),
                SortChoice::Label => left_pair.cmp(right_pair),
            },
        );
        confusions
    }

    fn total_deletions(&self) -> u128 {
        self.mistakes.values().map(|count| u128::from(*count)).sum()
    }
}

fn sort_state_id() -> egui::Id {
    egui::Id::new((METRIC_ID, "analysis-sort-state"))
}

fn expansion_state_id(panel: &'static str) -> egui::Id {
    egui::Id::new((METRIC_ID, "analysis-expansion-state", panel))
}

fn count_label(count: u64) -> String {
    let noun = if count == 1 {
        "occurrence"
    } else {
        "occurrences"
    };
    format!("{} {noun}", format_exact_count(count))
}

fn correction_empty_state(ui: &mut egui::Ui) {
    inline_empty_state(
        ui,
        egui_phosphor::regular::BACKSPACE,
        "No corrections observed",
        "Backspace-based deletion and correction signals will appear here when captured.",
    );
}

fn deleted_text_empty_state(ui: &mut egui::Ui) {
    inline_empty_state(
        ui,
        egui_phosphor::regular::BACKSPACE,
        "No deleted text yet",
        "No produced text has been observed immediately before a backspace.",
    );
}

fn inferred_corrections_empty_state(ui: &mut egui::Ui) {
    inline_empty_state(
        ui,
        egui_phosphor::regular::ARROW_RIGHT,
        "No inferred corrections yet",
        "Produced text immediately following a deletion will appear here.",
    );
}

fn correction_tokens(ui: &mut egui::Ui, deleted: &str, typed: &str) {
    text_token(ui, deleted, TextTokenContext::ProducedText);
    ui.label("→");
    text_token(ui, typed, TextTokenContext::ProducedText);
}

fn deleted_text_bar(ui: &mut egui::Ui, text: &str, count: u64, maximum: u64) {
    let visible_count = format_exact_count(count);
    let description = describe_text_token(text, TextTokenContext::ProducedText);
    let noun = if count == 1 { "deletion" } else { "deletions" };
    let accessible_label = format!(
        "Deleted {}: {visible_count} {noun}",
        description.accessible_label
    );
    ranked_bar_with_label(
        ui,
        &accessible_label,
        count as f64,
        maximum as f64,
        &visible_count,
        |ui| {
            text_token(ui, text, TextTokenContext::ProducedText);
        },
    );
}

fn correction_bar(ui: &mut egui::Ui, deleted: &str, typed: &str, count: u64, maximum: u64) {
    let visible_count = format_exact_count(count);
    let deleted_description = describe_text_token(deleted, TextTokenContext::ProducedText);
    let typed_description = describe_text_token(typed, TextTokenContext::ProducedText);
    let occurrences = count_label(count);
    let accessible_label = format!(
        "Inferred correction from {} to {}: {occurrences}",
        deleted_description.accessible_label, typed_description.accessible_label
    );
    ranked_bar_with_label(
        ui,
        &accessible_label,
        count as f64,
        maximum as f64,
        &visible_count,
        |ui| correction_tokens(ui, deleted, typed),
    );
}

fn deleted_text_panel(ui: &mut egui::Ui, rows: &[(&str, u64)]) {
    ui.push_id((METRIC_ID, "deleted-text"), |ui| {
        ui.set_width(ui.available_width());
        section_title(ui, "Most-deleted text");
        if rows.is_empty() {
            deleted_text_empty_state(ui);
            return;
        }
        let total_rows = rows.len();
        let maximum = rows.iter().map(|(_, count)| *count).max().unwrap_or(0);
        disclosure_list(
            ui,
            expansion_state_id("deleted-text"),
            total_rows,
            INITIAL_PANEL_ROWS,
            |ui, shown_rows| {
                for (text, count) in rows.iter().copied().take(shown_rows) {
                    deleted_text_bar(ui, text, count, maximum);
                }
            },
        );
    });
}

fn corrections_panel(ui: &mut egui::Ui, rows: &[((&str, &str), u64)]) {
    ui.push_id((METRIC_ID, "inferred-corrections"), |ui| {
        ui.set_width(ui.available_width());
        section_title(ui, "Inferred corrections");
        if rows.is_empty() {
            inferred_corrections_empty_state(ui);
            return;
        }
        let total_rows = rows.len();
        let maximum = rows.iter().map(|(_, count)| *count).max().unwrap_or(0);
        disclosure_list(
            ui,
            expansion_state_id("inferred-corrections"),
            total_rows,
            INITIAL_PANEL_ROWS,
            |ui, shown_rows| {
                for ((deleted, typed), count) in rows.iter().copied().take(shown_rows) {
                    correction_bar(ui, deleted, typed, count, maximum);
                }
            },
        );
    });
}

impl Metric for CorrectionSignals {
    const ID: &'static str = METRIC_ID;

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

    fn summary_ui(&self, ui: &mut egui::Ui) {
        if self.mistakes.is_empty() && self.confusions.is_empty() {
            correction_empty_state(ui);
            return;
        }

        let total = format_exact_count_u128(self.total_deletions());
        summary_value(
            ui,
            "Observed deletions",
            &total,
            &total,
            "Produced text observed immediately before backspace presses.",
        );
        ui.separator();

        let corrections = self.sorted_confusions(SortChoice::OccurrenceCount);
        let Some(((deleted, typed), count)) = corrections.first().copied() else {
            inferred_corrections_empty_state(ui);
            return;
        };
        ui.label(egui::RichText::new("Most frequent inferred correction").strong());
        ui.horizontal(|ui| correction_tokens(ui, deleted, typed));
        let occurrences = count_label(count);
        summary_value(
            ui,
            "Occurrences",
            &format_exact_count(count),
            &format_exact_count(count),
            &occurrences,
        );
    }

    fn analysis_ui(&self, ui: &mut egui::Ui) {
        if self.mistakes.is_empty() && self.confusions.is_empty() {
            correction_empty_state(ui);
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
                        SortChoice::OccurrenceCount,
                        SortChoice::OccurrenceCount.label(),
                    );
                    ui.selectable_value(&mut sort, SortChoice::Label, SortChoice::Label.label());
                });
        });
        if sort != previous_sort {
            ui.ctx().data_mut(|data| data.insert_temp(sort_id, sort));
        }

        let mistakes = self.sorted_mistakes(sort);
        let corrections = self.sorted_confusions(sort);
        if ui.available_width() >= SIDE_BY_SIDE_WIDTH {
            ui.columns(2, |columns| {
                deleted_text_panel(&mut columns[0], &mistakes);
                corrections_panel(&mut columns[1], &corrections);
            });
        } else {
            deleted_text_panel(ui, &mistakes);
            ui.add_space(8.0);
            corrections_panel(ui, &corrections);
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
                MetricSnapshotError::invalid_payload(Self::ID, "too many dimension entries")
            })?;
        validate_entry_count(Self::ID, entries)?;

        let mut deletions = Vec::with_capacity(self.mistakes.len());
        for (text, count) in &self.mistakes {
            validate_dimension(Self::ID, text)?;
            validate_count(Self::ID, *count)?;
            deletions.push(DeletionCount {
                text: text.clone(),
                count: *count,
            });
        }
        deletions.sort_by(|left, right| left.text.cmp(&right.text));

        let mut corrections = Vec::with_capacity(self.confusions.len());
        for ((deleted, typed), count) in &self.confusions {
            validate_dimension(Self::ID, deleted)?;
            validate_dimension(Self::ID, typed)?;
            validate_count(Self::ID, *count)?;
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
            Self::ID,
            SNAPSHOT_VERSION,
            &SnapshotV1 {
                deletions,
                corrections,
            },
        )
    }

    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError> {
        let state: SnapshotV1 = snapshot.decode(Self::ID, SNAPSHOT_VERSION)?;
        let entries = state
            .deletions
            .len()
            .checked_add(state.corrections.len())
            .ok_or_else(|| {
                MetricSnapshotError::invalid_payload(Self::ID, "too many dimension entries")
            })?;
        validate_entry_count(Self::ID, entries)?;

        let mut deletion_labels = HashSet::with_capacity(state.deletions.len());
        let mut mistakes = HashMap::with_capacity(state.deletions.len());
        for deletion in state.deletions {
            validate_dimension(Self::ID, &deletion.text)?;
            validate_count(Self::ID, deletion.count)?;
            if !deletion_labels.insert(deletion.text.clone()) {
                return Err(MetricSnapshotError::invalid_payload(
                    Self::ID,
                    "duplicate deletion dimension",
                ));
            }
            mistakes.insert(deletion.text, deletion.count);
        }

        let mut correction_pairs = HashSet::with_capacity(state.corrections.len());
        let mut confusions = HashMap::with_capacity(state.corrections.len());
        for correction in state.corrections {
            validate_dimension(Self::ID, &correction.deleted)?;
            validate_dimension(Self::ID, &correction.typed)?;
            validate_count(Self::ID, correction.count)?;
            let pair = (correction.deleted, correction.typed);
            if !correction_pairs.insert(pair.clone()) {
                return Err(MetricSnapshotError::invalid_payload(
                    Self::ID,
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

    use eframe::egui;
    use egui_kittest::{Harness, kittest::Queryable};

    use crate::{
        input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey},
        metric::Metric,
    };

    use super::{CorrectionSignals, METRIC_ID, SortChoice};

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
        let mut metric = CorrectionSignals::default();

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
    fn sorting_is_deterministic_for_counts_and_labels() {
        let mut metric = CorrectionSignals::default();
        metric.mistakes.insert("b".to_owned(), 2);
        metric.mistakes.insert("a".to_owned(), 2);
        metric.mistakes.insert("c".to_owned(), 3);
        metric
            .confusions
            .insert(("b".to_owned(), "x".to_owned()), 4);
        metric
            .confusions
            .insert(("a".to_owned(), "z".to_owned()), 4);
        metric
            .confusions
            .insert(("a".to_owned(), "y".to_owned()), 5);

        let mistake_order = |sort| {
            metric
                .sorted_mistakes(sort)
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
        };
        assert_eq!(mistake_order(SortChoice::OccurrenceCount), ["c", "a", "b"]);
        assert_eq!(mistake_order(SortChoice::Label), ["a", "b", "c"]);

        let correction_order = |sort| {
            metric
                .sorted_confusions(sort)
                .into_iter()
                .map(|(pair, _)| pair)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            correction_order(SortChoice::OccurrenceCount),
            [("a", "y"), ("a", "z"), ("b", "x")]
        );
        assert_eq!(
            correction_order(SortChoice::Label),
            [("a", "y"), ("a", "z"), ("b", "x")]
        );
    }

    #[test]
    fn summary_uses_overflow_safe_deletion_total_and_most_frequent_pair() {
        let mut metric = CorrectionSignals::default();
        for text in ["a", "b", "c"] {
            metric.mistakes.insert(text.to_owned(), i64::MAX as u64);
        }
        metric
            .confusions
            .insert(("x".to_owned(), "y".to_owned()), 9);
        assert_eq!(metric.total_deletions(), 27_670_116_110_564_327_421);

        let harness = Harness::new_ui(move |ui| metric.summary_ui(ui));
        assert!(
            harness
                .query_by_label("Observed deletions: 27,670,116,110,564,327,421")
                .is_some()
        );
        assert!(
            harness
                .query_by_label("Most frequent inferred correction")
                .is_some()
        );
        assert!(harness.query_by_label("Occurrences: 9").is_some());
    }

    #[test]
    fn analysis_rows_are_accessible_and_panels_expand_independently() {
        let mut metric = CorrectionSignals::default();
        metric.mistakes.insert("\n".to_owned(), 100);
        metric
            .confusions
            .insert(("\n".to_owned(), " ".to_owned()), 100);
        for index in 0..6 {
            metric.mistakes.insert(format!("deleted-{index}"), 1);
            metric
                .confusions
                .insert((format!("deleted-{index}"), format!("typed-{index}")), 1);
        }
        let mut harness = Harness::new_ui(move |ui| metric.analysis_ui(ui));

        assert!(
            harness
                .query_by_role_and_label(
                    egui::accesskit::Role::ProgressIndicator,
                    "Deleted Produced text: Newline: 100 deletions",
                )
                .is_some()
        );
        assert!(
            harness
                .query_by_role_and_label(
                    egui::accesskit::Role::ProgressIndicator,
                    "Inferred correction from Produced text: Newline to Produced text: Space: 100 occurrences",
                )
                .is_some()
        );
        assert_eq!(harness.query_all_by_label("Showing 6 of 7").count(), 2);
        harness
            .query_all_by_label("Show 1 more")
            .next()
            .expect("one panel should expose its expansion action")
            .click();
        harness.run_steps(2);
        assert_eq!(harness.query_all_by_label("Showing 7 of 7").count(), 1);
        assert_eq!(harness.query_all_by_label("Showing 6 of 7").count(), 1);
        assert_eq!(harness.query_all_by_label("Show fewer").count(), 1);
        assert_eq!(harness.query_all_by_label("Show 1 more").count(), 1);
    }

    #[test]
    fn clearing_in_flight_context_drops_recent_text_and_pending_correction() {
        let mut metric = CorrectionSignals::default();
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
        let mut metric = CorrectionSignals::default();
        metric.process(&text("o"));
        metric.process(&backspace());
        assert_eq!(metric.pending_deleted.as_deref(), Some("o"));

        let snapshot = metric.snapshot().unwrap();
        let mut restored = CorrectionSignals::default();
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
        let mut metric = CorrectionSignals::default();
        metric.process(&text("o"));
        metric.process(&backspace());
        metric.process(&text("p"));

        let snapshot = metric.snapshot().unwrap();
        assert_eq!(snapshot.metric_id(), METRIC_ID);
        assert_eq!(snapshot.schema_version(), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(snapshot.payload_json()).unwrap(),
            serde_json::json!({
                "deletions": [{ "text": "o", "count": 1 }],
                "corrections": [{ "deleted": "o", "typed": "p", "count": 1 }],
            })
        );

        let mut restored = CorrectionSignals::default();
        restored.restore(&snapshot).unwrap();

        assert_eq!(restored.mistakes, metric.mistakes);
        assert_eq!(restored.confusions, metric.confusions);
    }
}

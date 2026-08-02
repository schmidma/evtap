use std::collections::{HashMap, HashSet};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::{
    app::view::components::{
        card_header, disclosure_list, format_exact_count, inline_empty_state, physical_key_token,
        ranked_bar_with_label,
    },
    input::{KeyEvent, KeyEventKind, PhysicalKey},
    metric::{
        Metric, MetricSnapshot, MetricSnapshotError, validate_count, validate_dimension,
        validate_entry_count,
    },
};

const METRIC_ID: &str = "key-usage";
const SNAPSHOT_VERSION: u32 = 1;
const INITIAL_ANALYSIS_ROWS: usize = 8;

#[derive(Default)]
pub struct KeyUsage {
    counts: HashMap<PhysicalKey, u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SortChoice {
    #[default]
    PressCount,
    KeyLabel,
}

impl SortChoice {
    fn label(self) -> &'static str {
        match self {
            Self::PressCount => "Press count",
            Self::KeyLabel => "Key label",
        }
    }
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

impl KeyUsage {
    fn sorted_counts(&self, sort: SortChoice) -> Vec<(&PhysicalKey, u64)> {
        let mut counts: Vec<_> = self
            .counts
            .iter()
            .map(|(key, count)| (key, *count))
            .collect();
        counts.sort_by(
            |(left_key, left_count), (right_key, right_count)| match sort {
                SortChoice::PressCount => right_count
                    .cmp(left_count)
                    .then_with(|| left_key.label().cmp(right_key.label()))
                    .then_with(|| left_key.code().cmp(&right_key.code())),
                SortChoice::KeyLabel => left_key
                    .label()
                    .cmp(right_key.label())
                    .then_with(|| left_key.code().cmp(&right_key.code())),
            },
        );
        counts
    }

    fn total_presses(&self) -> u128 {
        self.counts.values().map(|count| u128::from(*count)).sum()
    }

    fn maximum_count(&self) -> u64 {
        self.counts.values().copied().max().unwrap_or(0)
    }

    pub(crate) fn most_used_ui(&self, ui: &mut egui::Ui) {
        let Some((key, count)) = self.sorted_counts(SortChoice::PressCount).first().copied() else {
            inline_empty_state(
                ui,
                egui_phosphor::regular::KEYBOARD,
                "No samples yet",
                "The most-used physical key will appear after capture begins.",
            );
            return;
        };
        let total = self.total_presses();
        card_header(
            ui,
            egui_phosphor::regular::STAR_FOUR,
            "Most-used physical key",
        );
        ui.add_space(4.0);
        physical_key_token(ui, key.label(), key.code());
        let exact = format_exact_count(count);
        crate::app::view::components::summary_value(
            ui,
            "Presses",
            &exact,
            &exact,
            &format!("{:.1}% of physical presses", percentage(count, total)),
        );
    }
}

fn sort_state_id() -> egui::Id {
    egui::Id::new((METRIC_ID, "analysis-sort-state"))
}

fn expansion_state_id() -> egui::Id {
    egui::Id::new((METRIC_ID, "analysis-expansion-state"))
}

fn percentage(count: u64, total: u128) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 * 100.0 / total as f64
    }
}

fn count_and_percentage(count: u64, total: u128) -> String {
    format!(
        "{} · {:.1}%",
        format_exact_count(count),
        percentage(count, total)
    )
}

fn key_usage_empty_state(ui: &mut egui::Ui) {
    inline_empty_state(
        ui,
        egui_phosphor::regular::KEYBOARD,
        "No key usage yet",
        "Physical key presses will appear here when they are captured.",
    );
}

fn key_usage_bar(ui: &mut egui::Ui, key: &PhysicalKey, count: u64, total: u128, maximum: u64) {
    let exact_count = format_exact_count(count);
    let percent = percentage(count, total);
    let visible_value = count_and_percentage(count, total);
    let accessible_label = format!(
        "Physical key {}, Linux key code {}: {exact_count} presses, {percent:.1}% of total physical presses",
        key.label(),
        key.code(),
    );
    ranked_bar_with_label(
        ui,
        &accessible_label,
        count as f64,
        maximum as f64,
        &visible_value,
        |ui| {
            physical_key_token(ui, key.label(), key.code());
        },
    );
}

impl Metric for KeyUsage {
    const ID: &'static str = METRIC_ID;

    fn process(&mut self, event: &KeyEvent) {
        if event.kind() == KeyEventKind::Press {
            *self.counts.entry(event.key().clone()).or_default() += 1;
        }
    }

    fn summary_ui(&self, ui: &mut egui::Ui) {
        if self.counts.is_empty() {
            key_usage_empty_state(ui);
            return;
        }

        let counts = self.sorted_counts(SortChoice::PressCount);
        let total = self.total_presses();
        let maximum = self.maximum_count();
        for (key, count) in counts.into_iter().take(3) {
            key_usage_bar(ui, key, count, total, maximum);
        }
    }

    fn analysis_ui(&self, ui: &mut egui::Ui) {
        if self.counts.is_empty() {
            key_usage_empty_state(ui);
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
                        SortChoice::PressCount,
                        SortChoice::PressCount.label(),
                    );
                    ui.selectable_value(
                        &mut sort,
                        SortChoice::KeyLabel,
                        SortChoice::KeyLabel.label(),
                    );
                });
        });
        if sort != previous_sort {
            ui.ctx().data_mut(|data| data.insert_temp(sort_id, sort));
        }

        let counts = self.sorted_counts(sort);
        let total_rows = counts.len();
        let total = self.total_presses();
        let maximum = self.maximum_count();
        disclosure_list(
            ui,
            expansion_state_id(),
            total_rows,
            INITIAL_ANALYSIS_ROWS,
            |ui, shown_rows| {
                for (key, count) in counts.into_iter().take(shown_rows) {
                    key_usage_bar(ui, key, count, total, maximum);
                }
            },
        );
    }

    fn has_data(&self) -> bool {
        !self.counts.is_empty()
    }

    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError> {
        validate_entry_count(Self::ID, self.counts.len())?;
        let mut keys = Vec::with_capacity(self.counts.len());
        for (key, count) in &self.counts {
            validate_dimension(Self::ID, key.label())?;
            validate_count(Self::ID, *count)?;
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
        MetricSnapshot::encode(Self::ID, SNAPSHOT_VERSION, &SnapshotV1 { keys })
    }

    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError> {
        let state: SnapshotV1 = snapshot.decode(Self::ID, SNAPSHOT_VERSION)?;
        validate_entry_count(Self::ID, state.keys.len())?;

        let mut codes = HashSet::with_capacity(state.keys.len());
        let mut counts = HashMap::with_capacity(state.keys.len());
        for key in state.keys {
            validate_dimension(Self::ID, &key.label)?;
            validate_count(Self::ID, key.count)?;
            if !codes.insert(key.code) {
                return Err(MetricSnapshotError::invalid_payload(
                    Self::ID,
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

    use eframe::egui;
    use egui_kittest::{Harness, kittest::Queryable};

    use crate::{
        input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey},
        metric::{Metric, MetricSnapshot},
    };

    use super::{KeyUsage, METRIC_ID, SortChoice, count_and_percentage, percentage};

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
    fn sorting_is_deterministic_for_counts_labels_and_codes() {
        let mut metric = KeyUsage::default();
        metric.counts.insert(PhysicalKey::new(31, "A"), 10);
        metric.counts.insert(PhysicalKey::new(44, "Z"), 10);
        metric.counts.insert(PhysicalKey::new(30, "A"), 10);
        metric.counts.insert(PhysicalKey::new(48, "B"), 12);

        let press_order: Vec<_> = metric
            .sorted_counts(SortChoice::PressCount)
            .into_iter()
            .map(|(key, _)| (key.label(), key.code()))
            .collect();
        assert_eq!(press_order, [("B", 48), ("A", 30), ("A", 31), ("Z", 44)]);

        let label_order: Vec<_> = metric
            .sorted_counts(SortChoice::KeyLabel)
            .into_iter()
            .map(|(key, _)| (key.label(), key.code()))
            .collect();
        assert_eq!(label_order, [("A", 30), ("A", 31), ("B", 48), ("Z", 44)]);
    }

    #[test]
    fn percentages_and_grouped_counts_are_stable() {
        assert_eq!(percentage(1, 3), 100.0 / 3.0);
        assert_eq!(percentage(0, 0), 0.0);
        assert_eq!(count_and_percentage(1_234, 4_936), "1,234 · 25.0%");
    }

    #[test]
    fn analysis_rows_are_accessible_and_expansion_uses_temporary_state() {
        let mut accessible_metric = KeyUsage::default();
        accessible_metric
            .counts
            .insert(PhysicalKey::new(30, "A"), 1_234);
        let accessible_harness = Harness::new_ui(move |ui| {
            accessible_metric.analysis_ui(ui);
        });
        assert!(
            accessible_harness
                .query_by_role_and_label(
                    egui::accesskit::Role::ProgressIndicator,
                    "Physical key A, Linux key code 30: 1,234 presses, 100.0% of total physical presses",
                )
                .is_some()
        );

        let mut expanded_metric = KeyUsage::default();
        for code in 1..=9 {
            expanded_metric
                .counts
                .insert(PhysicalKey::new(code, format!("Key {code}")), 1);
        }
        let mut expansion_harness = Harness::new_ui(move |ui| {
            expanded_metric.analysis_ui(ui);
        });
        assert!(expansion_harness.query_by_label("Showing 8 of 9").is_some());
        expansion_harness.get_by_label("Show 1 more").click();
        expansion_harness.step();
        expansion_harness.step();
        assert!(expansion_harness.query_by_label("Showing 9 of 9").is_some());
        assert!(expansion_harness.query_by_label("Show fewer").is_some());
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
            METRIC_ID,
            1,
            r#"{"keys":[{"code":30,"label":"A","count":1},{"code":30,"label":"OTHER","count":1}]}"#
                .to_owned(),
        )
        .unwrap();
        assert!(restored.restore(&duplicate).is_err());
        assert_eq!(restored.counts, first.counts);
    }
}

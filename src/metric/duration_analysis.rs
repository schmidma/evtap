use std::collections::HashMap;

use eframe::egui;

use crate::app::view::components::{disclosure_list, format_exact_count_u128};

use super::DurationStats;

const INITIAL_ANALYSIS_ROWS: usize = 8;

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

pub(super) fn overall_average(stats: &HashMap<String, DurationStats>) -> Option<(f64, u128)> {
    let samples = stats
        .values()
        .map(|stats| u128::from(stats.samples))
        .sum::<u128>();
    if samples == 0 {
        return None;
    }
    let total_nanoseconds = stats
        .values()
        .map(|stats| stats.total.as_nanos())
        .sum::<u128>();
    Some((
        total_nanoseconds as f64 / samples as f64 / 1_000_000.0,
        samples,
    ))
}

pub(super) fn sample_count_label(samples: u128) -> String {
    let count = format_exact_count_u128(samples);
    let noun = if samples == 1 { "sample" } else { "samples" };
    format!("{count} {noun}")
}

pub(super) fn render_duration_analysis(
    ui: &mut egui::Ui,
    metric_id: &'static str,
    stats: &HashMap<String, DurationStats>,
    mut render_row: impl FnMut(&mut egui::Ui, &str, DurationStats, f64),
) {
    let sort_id = egui::Id::new((metric_id, "analysis-sort-state"));
    let mut sort = ui
        .ctx()
        .data(|data| data.get_temp::<SortChoice>(sort_id))
        .unwrap_or_default();
    let previous_sort = sort;
    ui.horizontal(|ui| {
        ui.label("Sort");
        egui::ComboBox::from_id_salt((metric_id, "analysis-sort-control"))
            .selected_text(sort.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut sort, SortChoice::Slowest, SortChoice::Slowest.label());
                ui.selectable_value(&mut sort, SortChoice::Fastest, SortChoice::Fastest.label());
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

    let rows = sorted_stats(stats, sort);
    let total_rows = rows.len();
    let maximum = maximum_average(stats);
    disclosure_list(
        ui,
        egui::Id::new((metric_id, "analysis-expansion-state")),
        total_rows,
        INITIAL_ANALYSIS_ROWS,
        |ui, shown_rows| {
            for (text, stats) in rows.into_iter().take(shown_rows) {
                render_row(ui, text, stats, maximum);
            }
        },
    );
}

fn sorted_stats(
    stats: &HashMap<String, DurationStats>,
    sort: SortChoice,
) -> Vec<(&str, DurationStats)> {
    let mut rows: Vec<_> = stats
        .iter()
        .map(|(text, stats)| (text.as_str(), *stats))
        .collect();
    rows.sort_by(|(left_text, left), (right_text, right)| match sort {
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
    rows
}

fn maximum_average(stats: &HashMap<String, DurationStats>) -> f64 {
    stats
        .values()
        .copied()
        .max_by(|left, right| left.compare_average(*right))
        .map(DurationStats::average_milliseconds)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use super::{SortChoice, overall_average, sorted_stats};
    use crate::metric::DurationStats;

    fn duration_stats(total_ms: u64, samples: u64) -> DurationStats {
        DurationStats {
            total: Duration::from_millis(total_ms),
            samples,
        }
    }

    #[test]
    fn overall_average_is_weighted_by_samples() {
        let stats = HashMap::from([
            ("a".to_owned(), duration_stats(100, 2)),
            ("b".to_owned(), duration_stats(100, 1)),
            ("c".to_owned(), duration_stats(150, 3)),
        ]);

        let (average, samples) =
            overall_average(&stats).expect("populated duration stats should have an average");
        assert_eq!(samples, 6);
        assert!((average - 350.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn sorting_is_deterministic_for_averages_labels_and_sample_counts() {
        let mut stats = HashMap::from([
            ("alpha".to_owned(), duration_stats(100, 2)),
            ("beta".to_owned(), duration_stats(100, 1)),
            ("delta".to_owned(), duration_stats(150, 3)),
        ]);
        let order = |sort| {
            sorted_stats(&stats, sort)
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
        };
        assert_eq!(order(SortChoice::Slowest), ["beta", "alpha", "delta"]);
        assert_eq!(order(SortChoice::Fastest), ["alpha", "delta", "beta"]);
        assert_eq!(order(SortChoice::Label), ["alpha", "beta", "delta"]);
        assert_eq!(order(SortChoice::SampleCount), ["delta", "alpha", "beta"]);

        stats.clear();
        stats.insert(
            "alphabetically-first".to_owned(),
            DurationStats {
                total: Duration::from_nanos(i64::MAX as u64 - 1),
                samples: 1,
            },
        );
        stats.insert(
            "truly-slower".to_owned(),
            DurationStats {
                total: Duration::from_nanos(i64::MAX as u64),
                samples: 1,
            },
        );
        assert_eq!(
            sorted_stats(&stats, SortChoice::Slowest)
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>(),
            ["truly-slower", "alphabetically-first"]
        );
    }
}

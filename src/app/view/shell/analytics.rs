use eframe::egui;

use super::super::{
    components::{self, BannerSeverity},
    theme,
};
use crate::{
    app::{App, AppView, TimingView},
    metric::Metric,
};

const SUMMARY_TILE_MIN_HEIGHT: f32 = 168.0;
fn metric_analysis_card(
    ui: &mut egui::Ui,
    icon: &str,
    heading: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    components::card(ui, |ui| {
        ui.set_width(ui.available_width());
        components::card_header(ui, icon, heading);
        components::vertical_gap(ui, theme::SPACE_LG);
        add_contents(ui);
    });
}

impl App {
    fn page_heading(&self, ui: &mut egui::Ui, title: &str, description: &str) {
        ui.heading(title);
        ui.label(description);
        components::vertical_gap(ui, theme::SPACE_XL);
    }

    pub(in crate::app::view::shell) fn render_overview_page(&mut self, ui: &mut egui::Ui) {
        self.page_heading(
            ui,
            "Overview",
            "Aggregate keyboard mechanics for the active session.",
        );
        if !self.working_session.metrics.has_data() {
            components::contextual_banner(
                ui,
                BannerSeverity::Info,
                "Start with a short typing session",
                "Choose a keyboard, select Start, and type normally. evtap builds aggregate analytics without keeping raw input events.",
            );
            components::vertical_gap(ui, theme::SPACE_LG);
        }

        let render_summary = |ui: &mut egui::Ui, index: usize| -> Option<AppView> {
            match index {
                0 => components::open_card(
                    ui,
                    "overview-total",
                    "Open Key Usage",
                    SUMMARY_TILE_MIN_HEIGHT,
                    None,
                    |ui| {
                        self.working_session.metrics.total_presses().summary_ui(ui);
                    },
                )
                .then_some(AppView::KeyUsage),
                1 => components::open_card(
                    ui,
                    "overview-most-used",
                    "Open Key Usage",
                    SUMMARY_TILE_MIN_HEIGHT,
                    None,
                    |ui| self.working_session.metrics.key_usage().most_used_ui(ui),
                )
                .then_some(AppView::KeyUsage),
                2 => components::open_card(
                    ui,
                    "overview-dwell",
                    "Open Dwell timing",
                    SUMMARY_TILE_MIN_HEIGHT,
                    None,
                    |ui| {
                        self.working_session.metrics.dwell_time().summary_ui(ui);
                    },
                )
                .then_some(AppView::Timing(TimingView::Dwell)),
                3 => components::open_card(
                    ui,
                    "overview-flight",
                    "Open Flight timing",
                    SUMMARY_TILE_MIN_HEIGHT,
                    None,
                    |ui| self.working_session.metrics.flight_time().summary_ui(ui),
                )
                .then_some(AppView::Timing(TimingView::Flight)),
                _ => None,
            }
        };
        let mut requested_view = None;
        if ui.available_width() >= 760.0 {
            ui.columns(4, |columns| {
                for (index, column) in columns.iter_mut().enumerate() {
                    if let Some(view) = render_summary(column, index) {
                        requested_view = Some(view);
                    }
                }
            });
        } else {
            for first in [0, 2] {
                ui.columns(2, |columns| {
                    for (offset, column) in columns.iter_mut().enumerate() {
                        if let Some(view) = render_summary(column, first + offset) {
                            requested_view = Some(view);
                        }
                    }
                });
            }
        }
        if let Some(view) = requested_view {
            if let AppView::Timing(timing_view) = view {
                self.timing_view = timing_view;
            }
            self.view = view;
        }

        components::vertical_gap(ui, theme::SPACE_MD);
        let key_usage_card = |ui: &mut egui::Ui| {
            components::open_card(
                ui,
                "overview-key-ranking",
                "Open Key Usage",
                0.0,
                Some("Open Key Usage"),
                |ui| {
                    components::card_header(
                        ui,
                        egui_phosphor::regular::CHART_BAR_HORIZONTAL,
                        "Key Usage",
                    );
                    ui.weak("Most-used physical keys in this session.");
                    components::vertical_gap(ui, theme::SPACE_LG);
                    self.working_session.metrics.key_usage().summary_ui(ui);
                },
            )
        };
        let corrections_card = |ui: &mut egui::Ui| {
            components::open_card(
                ui,
                "overview-corrections",
                "Open Corrections",
                0.0,
                Some("Open Corrections"),
                |ui| {
                    components::card_header(
                        ui,
                        egui_phosphor::regular::ARROW_U_DOWN_LEFT,
                        "Correction Signals",
                    );
                    ui.weak("Observed deletions and inferred replacements.");
                    components::vertical_gap(ui, theme::SPACE_LG);
                    self.working_session.metrics.corrections().summary_ui(ui);
                },
            )
        };
        let (open_key_usage, open_corrections) = if ui.available_width() >= 700.0 {
            let mut open_key_usage = false;
            let mut open_corrections = false;
            let available = ui.available_width();
            let gap = ui.spacing().item_spacing.x;
            let primary_width = (available - gap) * 0.62;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(primary_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        open_key_usage = key_usage_card(ui);
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(available - primary_width - gap, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        open_corrections = corrections_card(ui);
                    },
                );
            });
            (open_key_usage, open_corrections)
        } else {
            let open_key_usage = key_usage_card(ui);
            components::vertical_gap(ui, theme::SPACE_LG);
            let open_corrections = corrections_card(ui);
            (open_key_usage, open_corrections)
        };
        if open_key_usage {
            self.view = AppView::KeyUsage;
        } else if open_corrections {
            self.view = AppView::Corrections;
        }
    }

    pub(in crate::app::view::shell) fn render_key_usage_page(&self, ui: &mut egui::Ui) {
        self.page_heading(
            ui,
            "Key Usage",
            "Physical key counts identified by Linux key code, with each key's share of all physical presses in this session.",
        );
        metric_analysis_card(
            ui,
            egui_phosphor::regular::CHART_BAR_HORIZONTAL,
            "Physical key ranking",
            |ui| self.working_session.metrics.key_usage().analysis_ui(ui),
        );
    }

    pub(in crate::app::view::shell) fn render_timing_page(
        &mut self,
        ui: &mut egui::Ui,
        timing_view: TimingView,
    ) {
        let description = match timing_view {
            TimingView::Dwell => {
                "How long produced-text keys are held, weighted by completed press-to-release samples."
            }
            TimingView::Flight => {
                "Release-to-next-press timing grouped by the destination produced text."
            }
            TimingView::Bigrams => {
                "Press-to-press timing for consecutive produced text; pairs appear after at least three samples."
            }
        };
        self.page_heading(ui, "Timing", description);
        ui.horizontal(|ui| {
            for (label, target) in [
                ("Dwell", TimingView::Dwell),
                ("Flight", TimingView::Flight),
                ("Bigrams", TimingView::Bigrams),
            ] {
                if ui.selectable_label(timing_view == target, label).clicked() {
                    self.timing_view = target;
                    self.view = AppView::Timing(target);
                }
            }
        });
        components::vertical_gap(ui, theme::SPACE_LG);
        match timing_view {
            TimingView::Dwell => {
                metric_analysis_card(ui, egui_phosphor::regular::TIMER, "Dwell time", |ui| {
                    self.working_session.metrics.dwell_time().analysis_ui(ui)
                })
            }
            TimingView::Flight => metric_analysis_card(
                ui,
                egui_phosphor::regular::PAPER_PLANE_TILT,
                "Flight time",
                |ui| self.working_session.metrics.flight_time().analysis_ui(ui),
            ),
            TimingView::Bigrams => {
                metric_analysis_card(ui, egui_phosphor::regular::CLOCK, "Bigram speed", |ui| {
                    self.working_session.metrics.bigram_speed().analysis_ui(ui)
                })
            }
        }
    }

    pub(in crate::app::view::shell) fn render_corrections_page(&self, ui: &mut egui::Ui) {
        self.page_heading(
            ui,
            "Corrections",
            "Backspace-based estimates: deletions are observed before backspace and replacements inferred from the next produced text—not an accuracy score.",
        );
        metric_analysis_card(
            ui,
            egui_phosphor::regular::ARROW_U_DOWN_LEFT,
            "Correction signals",
            |ui| self.working_session.metrics.corrections().analysis_ui(ui),
        );
    }
}

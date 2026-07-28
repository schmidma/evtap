use eframe::egui::{Grid, Ui};

use crate::metric::{Metric, ReportSection, ReportValue};

pub fn render_metric(ui: &mut Ui, metric: &dyn Metric) {
    let descriptor = metric.descriptor();
    ui.heading(descriptor.name);
    if !descriptor.description.is_empty() {
        ui.small(descriptor.description);
    }
    ui.add_space(5.0);

    let report = metric.report();
    for (section_index, section) in report.sections.into_iter().enumerate() {
        match section {
            ReportSection::Scalar { label, value } => {
                ui.label(format!("{label}: {}", format_value(&value)));
            }
            ReportSection::Table {
                title,
                columns,
                rows,
            } => {
                if let Some(title) = title {
                    ui.strong(title);
                }
                if rows.is_empty() {
                    ui.weak("No samples yet");
                    continue;
                }
                Grid::new((descriptor.id, section_index))
                    .striped(true)
                    .show(ui, |ui| {
                        for column in columns {
                            ui.strong(*column);
                        }
                        ui.end_row();
                        for row in rows {
                            for value in row {
                                ui.label(format_value(&value));
                            }
                            ui.end_row();
                        }
                    });
            }
        }
        ui.add_space(5.0);
    }
}

fn format_value(value: &ReportValue) -> String {
    match value {
        ReportValue::Text(value) => value.clone(),
        ReportValue::Count(value) => value.to_string(),
        ReportValue::Milliseconds(value) => format!("{value:.1} ms"),
    }
}

#[cfg(test)]
mod tests {
    use super::format_value;
    use crate::metric::ReportValue;

    #[test]
    fn formats_report_values() {
        assert_eq!(format_value(&ReportValue::Count(42)), "42");
        assert_eq!(format_value(&ReportValue::Milliseconds(12.34)), "12.3 ms");
        assert_eq!(format_value(&ReportValue::Text("key".to_owned())), "key");
    }
}

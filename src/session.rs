use std::collections::HashSet;

use crate::metric::{MetricSnapshot, MetricSnapshotError, SessionMetrics};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionId(i64);

impl SessionId {
    pub fn new(value: i64) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyboardContext {
    pub display_name: Option<String>,
    pub model: String,
    pub layout: String,
    pub variant: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSnapshot {
    pub id: Option<SessionId>,
    pub name: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_opened_at_ms: i64,
    pub captured_duration_ns: i64,
    pub application_version: String,
    pub keyboard: KeyboardContext,
    pub metrics: Vec<MetricSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMetadata {
    pub id: SessionId,
    pub name: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_opened_at_ms: i64,
    pub captured_duration_ns: i64,
    pub application_version: String,
    pub keyboard: KeyboardContext,
}

impl SessionMetadata {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("Untitled session")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredMetricSnapshot {
    pub metric_id: String,
    pub schema_version: i64,
    pub payload_json: String,
}

impl StoredMetricSnapshot {
    pub fn to_metric_snapshot(&self) -> Result<MetricSnapshot, StoredMetricError> {
        let schema_version = u32::try_from(self.schema_version)
            .ok()
            .filter(|version| *version > 0)
            .ok_or(StoredMetricError::InvalidSchemaVersion {
                actual: self.schema_version,
            })?;
        MetricSnapshot::from_json(
            self.metric_id.clone(),
            schema_version,
            self.payload_json.clone(),
        )
        .map_err(StoredMetricError::InvalidSnapshot)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoredMetricError {
    #[error("stored metric schema version {actual} is invalid")]
    InvalidSchemaVersion { actual: i64 },
    #[error("stored metric snapshot metadata is invalid")]
    InvalidSnapshot(#[source] MetricSnapshotError),
}

#[derive(Debug)]
pub struct StoredSession {
    pub metadata: SessionMetadata,
    pub metrics: Vec<StoredMetricSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetricRecoveryIssue {
    Unknown { metric_id: String },
    Duplicate { metric_id: String },
    Invalid { metric_id: String, details: String },
}

impl SessionMetrics {
    pub fn restore(
        snapshots: &[StoredMetricSnapshot],
    ) -> (SessionMetrics, Vec<MetricRecoveryIssue>) {
        let mut metrics = SessionMetrics::default();
        let mut issues = Vec::new();
        let mut encountered = HashSet::with_capacity(snapshots.len());

        for stored in snapshots {
            if !encountered.insert(stored.metric_id.as_str()) {
                issues.push(MetricRecoveryIssue::Duplicate {
                    metric_id: stored.metric_id.clone(),
                });
                continue;
            }
            if !SessionMetrics::contains_id(&stored.metric_id) {
                issues.push(MetricRecoveryIssue::Unknown {
                    metric_id: stored.metric_id.clone(),
                });
                continue;
            }
            let result = stored
                .to_metric_snapshot()
                .map_err(|error| error.to_string())
                .and_then(|snapshot| {
                    metrics
                        .restore_snapshot(&snapshot)
                        .map_err(|error| error.to_string())
                });
            if let Err(details) = result {
                issues.push(MetricRecoveryIssue::Invalid {
                    metric_id: stored.metric_id.clone(),
                    details,
                });
            }
        }

        (metrics, issues)
    }
}

#[cfg(test)]
mod tests {
    use crate::metric::{MetricSnapshot, SessionMetrics};

    use super::{
        MetricRecoveryIssue, SessionId, SessionMetadata, StoredMetricError, StoredMetricSnapshot,
    };

    #[test]
    fn stored_metric_validates_schema_before_conversion() {
        let stored = StoredMetricSnapshot {
            metric_id: "test".to_owned(),
            schema_version: -1,
            payload_json: "{}".to_owned(),
        };
        assert!(matches!(
            stored.to_metric_snapshot(),
            Err(StoredMetricError::InvalidSchemaVersion { actual: -1 })
        ));

        let stored = StoredMetricSnapshot {
            metric_id: "test".to_owned(),
            schema_version: 1,
            payload_json: "{}".to_owned(),
        };
        assert_eq!(
            stored.to_metric_snapshot().unwrap(),
            MetricSnapshot::from_json("test", 1, "{}".to_owned()).unwrap()
        );
    }

    #[test]
    fn recovery_isolates_partial_unknown_duplicate_and_invalid_metrics() {
        let snapshots = vec![
            StoredMetricSnapshot {
                metric_id: "total-presses".to_owned(),
                schema_version: 1,
                payload_json: r#"{"count":17}"#.to_owned(),
            },
            StoredMetricSnapshot {
                metric_id: "key-usage".to_owned(),
                schema_version: 1,
                payload_json: "not json".to_owned(),
            },
            StoredMetricSnapshot {
                metric_id: "future-metric".to_owned(),
                schema_version: 1,
                payload_json: "{}".to_owned(),
            },
            StoredMetricSnapshot {
                metric_id: "total-presses".to_owned(),
                schema_version: 1,
                payload_json: r#"{"count":99}"#.to_owned(),
            },
        ];

        let (recovered, issues) = SessionMetrics::restore(&snapshots);
        let recovered_snapshots = recovered.snapshots().unwrap();
        let total = recovered_snapshots
            .iter()
            .find(|snapshot| snapshot.metric_id() == "total-presses")
            .unwrap();
        let key_usage = recovered_snapshots
            .iter()
            .find(|snapshot| snapshot.metric_id() == "key-usage")
            .unwrap();

        assert_eq!(total.payload_json(), r#"{"count":17}"#);
        assert_eq!(key_usage.payload_json(), r#"{"keys":[]}"#);
        let defaults = SessionMetrics::default().snapshots().unwrap();
        for recovered in recovered_snapshots
            .iter()
            .filter(|snapshot| snapshot.metric_id() != "total-presses")
        {
            let default = defaults
                .iter()
                .find(|snapshot| snapshot.metric_id() == recovered.metric_id())
                .unwrap();
            assert_eq!(recovered, default);
        }
        assert_eq!(issues.len(), 3);
        assert!(matches!(
            &issues[0],
            MetricRecoveryIssue::Invalid { metric_id, .. } if metric_id == "key-usage"
        ));
        assert!(matches!(
            &issues[1],
            MetricRecoveryIssue::Unknown { metric_id } if metric_id == "future-metric"
        ));
        assert!(matches!(
            &issues[2],
            MetricRecoveryIssue::Duplicate { metric_id } if metric_id == "total-presses"
        ));
    }

    #[test]
    fn unnamed_session_has_a_stable_display_fallback() {
        let metadata = SessionMetadata {
            id: SessionId::new(1).unwrap(),
            name: None,
            created_at_ms: 1,
            updated_at_ms: 1,
            last_opened_at_ms: 1,
            captured_duration_ns: 0,
            application_version: "test".to_owned(),
            keyboard: Default::default(),
        };

        assert_eq!(metadata.display_name(), "Untitled session");
    }
}

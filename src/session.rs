#![allow(
    dead_code,
    reason = "session types are consumed by the storage worker milestone"
)]

use crate::metric::{MetricSnapshot, MetricSnapshotError};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Active,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyboardContext {
    pub display_name: Option<String>,
    pub model: String,
    pub layout: String,
    pub variant: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSnapshot {
    pub id: Option<SessionId>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub captured_duration_ns: i64,
    pub application_version: String,
    pub keyboard: KeyboardContext,
    pub metrics: Vec<MetricSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMetadata {
    pub id: SessionId,
    pub status: SessionStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub captured_duration_ns: i64,
    pub application_version: String,
    pub keyboard: KeyboardContext,
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
pub struct SessionSummary {
    pub metadata: SessionMetadata,
    pub total_presses: Option<u64>,
}

#[cfg(test)]
mod tests {
    use crate::metric::MetricSnapshot;

    use super::{StoredMetricError, StoredMetricSnapshot};

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
}

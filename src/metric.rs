#![allow(
    dead_code,
    reason = "snapshot APIs are consumed by the next persistence milestone"
)]

use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::input::KeyEvent;

const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIMENSION_ENTRIES: usize = 100_000;
const MAX_DIMENSION_BYTES: usize = 256;
const MAX_DURATION_NANOSECONDS: u64 = i64::MAX as u64;

mod bigram_speed;
mod dwell_time;
mod error_rate;
mod flight_time;
mod key_usage;
mod total_presses;

use bigram_speed::BigramSpeed;
use dwell_time::DwellTime;
use error_rate::ErrorRate;
use flight_time::FlightTime;
use key_usage::KeyUsage;
use total_presses::TotalPresses;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricSnapshot {
    metric_id: String,
    schema_version: u32,
    payload_json: String,
}

impl MetricSnapshot {
    pub fn from_json(
        metric_id: impl Into<String>,
        schema_version: u32,
        payload_json: String,
    ) -> Result<Self, MetricSnapshotError> {
        let metric_id = metric_id.into();
        if metric_id.is_empty() {
            return Err(MetricSnapshotError::EmptyMetricId);
        }
        if schema_version == 0 {
            return Err(MetricSnapshotError::InvalidSchemaVersion);
        }
        if payload_json.len() > MAX_SNAPSHOT_BYTES {
            return Err(MetricSnapshotError::PayloadTooLarge {
                metric_id,
                size: payload_json.len(),
                maximum: MAX_SNAPSHOT_BYTES,
            });
        }

        Ok(Self {
            metric_id,
            schema_version,
            payload_json,
        })
    }

    pub fn encode<T: Serialize>(
        metric_id: &str,
        schema_version: u32,
        payload: &T,
    ) -> Result<Self, MetricSnapshotError> {
        let payload_json =
            serde_json::to_string(payload).map_err(|source| MetricSnapshotError::Encode {
                metric_id: metric_id.to_owned(),
                source,
            })?;
        Self::from_json(metric_id, schema_version, payload_json)
    }

    pub fn decode<T: DeserializeOwned>(
        &self,
        expected_metric_id: &str,
        supported_schema_version: u32,
    ) -> Result<T, MetricSnapshotError> {
        if self.metric_id() != expected_metric_id {
            return Err(MetricSnapshotError::MetricMismatch {
                expected: expected_metric_id.to_owned(),
                actual: self.metric_id().to_owned(),
            });
        }
        if self.schema_version() != supported_schema_version {
            return Err(MetricSnapshotError::UnsupportedSchemaVersion {
                metric_id: self.metric_id().to_owned(),
                supported: supported_schema_version,
                actual: self.schema_version(),
            });
        }

        serde_json::from_str(self.payload_json()).map_err(|source| MetricSnapshotError::Decode {
            metric_id: self.metric_id().to_owned(),
            schema_version: self.schema_version(),
            source,
        })
    }

    pub fn metric_id(&self) -> &str {
        &self.metric_id
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn payload_json(&self) -> &str {
        &self.payload_json
    }
}

#[derive(Debug, Error)]
pub enum MetricSnapshotError {
    #[error("metric snapshot ID cannot be empty")]
    EmptyMetricId,
    #[error("metric snapshot schema version must be greater than zero")]
    InvalidSchemaVersion,
    #[error("metric snapshot for {metric_id} is {size} bytes; maximum is {maximum} bytes")]
    PayloadTooLarge {
        metric_id: String,
        size: usize,
        maximum: usize,
    },
    #[error("failed to encode metric snapshot for {metric_id}")]
    Encode {
        metric_id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("snapshot belongs to metric {actual}, not {expected}")]
    MetricMismatch { expected: String, actual: String },
    #[error(
        "metric {metric_id} snapshot schema version {actual} is unsupported; expected {supported}"
    )]
    UnsupportedSchemaVersion {
        metric_id: String,
        supported: u32,
        actual: u32,
    },
    #[error("failed to decode metric {metric_id} snapshot schema version {schema_version}")]
    Decode {
        metric_id: String,
        schema_version: u32,
        #[source]
        source: serde_json::Error,
    },
    #[error("metric {metric_id} snapshot is invalid: {reason}")]
    InvalidPayload {
        metric_id: String,
        reason: &'static str,
    },
}

impl MetricSnapshotError {
    pub fn invalid_payload(metric_id: &str, reason: &'static str) -> Self {
        Self::InvalidPayload {
            metric_id: metric_id.to_owned(),
            reason,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReportValue {
    Text(String),
    Count(u64),
    Milliseconds(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReportSection {
    Scalar {
        label: &'static str,
        value: ReportValue,
    },
    Table {
        title: Option<&'static str>,
        columns: &'static [&'static str],
        rows: Vec<Vec<ReportValue>>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricReport {
    pub sections: Vec<ReportSection>,
}

pub trait Metric {
    fn descriptor(&self) -> &'static MetricDescriptor;
    fn process(&mut self, event: &KeyEvent);
    fn report(&self) -> MetricReport;
    fn has_data(&self) -> bool;
    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError>;
    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError>;
    fn reset(&mut self);
}

fn validate_entry_count(metric_id: &str, entries: usize) -> Result<(), MetricSnapshotError> {
    if entries > MAX_DIMENSION_ENTRIES {
        return Err(MetricSnapshotError::invalid_payload(
            metric_id,
            "too many dimension entries",
        ));
    }
    Ok(())
}

fn validate_dimension(metric_id: &str, value: &str) -> Result<(), MetricSnapshotError> {
    if value.len() > MAX_DIMENSION_BYTES {
        return Err(MetricSnapshotError::invalid_payload(
            metric_id,
            "dimension string is too long",
        ));
    }
    Ok(())
}

fn validate_count(metric_id: &str, count: u64) -> Result<(), MetricSnapshotError> {
    if count == 0 {
        return Err(MetricSnapshotError::invalid_payload(
            metric_id,
            "aggregate count must be greater than zero",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DurationStats {
    total: Duration,
    samples: u64,
}

impl DurationStats {
    fn record(&mut self, duration: Duration) {
        self.total += duration;
        self.samples += 1;
    }

    fn average_milliseconds(self) -> f64 {
        self.total.as_secs_f64() * 1_000.0 / self.samples as f64
    }

    fn snapshot_parts(self, metric_id: &str) -> Result<(u64, u64), MetricSnapshotError> {
        validate_count(metric_id, self.samples)?;
        let total_nanoseconds = u64::try_from(self.total.as_nanos()).map_err(|_| {
            MetricSnapshotError::invalid_payload(metric_id, "duration total is too large")
        })?;
        if total_nanoseconds > MAX_DURATION_NANOSECONDS {
            return Err(MetricSnapshotError::invalid_payload(
                metric_id,
                "duration total is too large",
            ));
        }
        Ok((total_nanoseconds, self.samples))
    }

    fn from_snapshot_parts(
        metric_id: &str,
        total_nanoseconds: u64,
        samples: u64,
    ) -> Result<Self, MetricSnapshotError> {
        validate_count(metric_id, samples)?;
        if total_nanoseconds > MAX_DURATION_NANOSECONDS {
            return Err(MetricSnapshotError::invalid_payload(
                metric_id,
                "duration total is too large",
            ));
        }
        Ok(Self {
            total: Duration::from_nanos(total_nanoseconds),
            samples,
        })
    }
}

pub fn default_metrics() -> Vec<Box<dyn Metric>> {
    vec![
        Box::new(TotalPresses::default()),
        Box::new(KeyUsage::default()),
        Box::new(ErrorRate::default()),
        Box::new(FlightTime::default()),
        Box::new(DwellTime::default()),
        Box::new(BigramSpeed::default()),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde::{Deserialize, Serialize};

    use super::{MetricSnapshot, MetricSnapshotError, default_metrics};

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct TestPayload {
        count: u64,
    }

    #[test]
    fn metric_snapshot_round_trips_json_payload() {
        let snapshot = MetricSnapshot::encode("test", 1, &TestPayload { count: 42 }).unwrap();

        assert_eq!(snapshot.metric_id(), "test");
        assert_eq!(snapshot.schema_version(), 1);
        assert_eq!(snapshot.payload_json(), r#"{"count":42}"#);
        let restored: TestPayload = snapshot.decode("test", 1).unwrap();
        assert_eq!(restored, TestPayload { count: 42 });
    }

    #[test]
    fn metric_snapshot_rejects_wrong_identity_and_version() {
        let snapshot = MetricSnapshot::encode("test", 2, &TestPayload { count: 42 }).unwrap();

        assert!(matches!(
            snapshot.decode::<TestPayload>("other", 2),
            Err(MetricSnapshotError::MetricMismatch { .. })
        ));
        assert!(matches!(
            snapshot.decode::<TestPayload>("test", 1),
            Err(MetricSnapshotError::UnsupportedSchemaVersion { .. })
        ));
    }

    #[test]
    fn metric_snapshot_validates_metadata_and_size() {
        assert!(matches!(
            MetricSnapshot::from_json("", 1, "{}".to_owned()),
            Err(MetricSnapshotError::EmptyMetricId)
        ));
        assert!(matches!(
            MetricSnapshot::from_json("test", 0, "{}".to_owned()),
            Err(MetricSnapshotError::InvalidSchemaVersion)
        ));
        assert!(matches!(
            MetricSnapshot::from_json("test", 1, "x".repeat(super::MAX_SNAPSHOT_BYTES + 1)),
            Err(MetricSnapshotError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn default_metric_ids_are_unique() {
        let mut ids = HashSet::new();

        for metric in default_metrics() {
            let id = metric.descriptor().id;
            assert!(ids.insert(id), "duplicate metric id: {id}");
        }
    }

    #[test]
    fn default_metrics_round_trip_empty_snapshots() {
        for mut metric in default_metrics() {
            assert!(!metric.has_data());
            let snapshot = metric.snapshot().unwrap();
            assert_eq!(snapshot.metric_id(), metric.descriptor().id);
            assert_eq!(snapshot.schema_version(), 1);
            metric.restore(&snapshot).unwrap();
            assert!(!metric.has_data());
        }
    }
}

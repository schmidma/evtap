use std::time::Duration;

use eframe::egui;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::input::KeyEvent;

const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIMENSION_ENTRIES: usize = 100_000;
const MAX_DIMENSION_BYTES: usize = 256;
const MAX_AGGREGATE_COUNT: u64 = i64::MAX as u64;
const MAX_DURATION_NANOSECONDS: u64 = i64::MAX as u64;

mod bigram_speed;
mod correction_signals;
mod dwell_time;
mod flight_time;
mod key_usage;
mod total_presses;

pub(crate) use bigram_speed::BigramSpeed;
pub(crate) use correction_signals::CorrectionSignals;
pub(crate) use dwell_time::DwellTime;
pub(crate) use flight_time::FlightTime;
pub(crate) use key_usage::KeyUsage;
pub(crate) use total_presses::TotalPresses;

const SESSION_METRIC_IDS: [&str; 6] = [
    <TotalPresses as Metric>::ID,
    <KeyUsage as Metric>::ID,
    <DwellTime as Metric>::ID,
    <FlightTime as Metric>::ID,
    <BigramSpeed as Metric>::ID,
    <CorrectionSignals as Metric>::ID,
];

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

pub trait Metric {
    const ID: &'static str;

    fn process(&mut self, event: &KeyEvent);
    fn summary_ui(&self, ui: &mut egui::Ui);
    fn analysis_ui(&self, ui: &mut egui::Ui);
    fn has_data(&self) -> bool;
    fn snapshot(&self) -> Result<MetricSnapshot, MetricSnapshotError>;
    fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), MetricSnapshotError>;
    fn clear_in_flight(&mut self) {}
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

fn validate_scalar_count(metric_id: &str, count: u64) -> Result<(), MetricSnapshotError> {
    if count > MAX_AGGREGATE_COUNT {
        return Err(MetricSnapshotError::invalid_payload(
            metric_id,
            "aggregate count is too large",
        ));
    }
    Ok(())
}

fn validate_count(metric_id: &str, count: u64) -> Result<(), MetricSnapshotError> {
    validate_scalar_count(metric_id, count)?;
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

    fn compare_average(self, other: Self) -> std::cmp::Ordering {
        let left = self.total.as_nanos() * u128::from(other.samples);
        let right = other.total.as_nanos() * u128::from(self.samples);
        left.cmp(&right)
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

#[derive(Default)]
pub struct SessionMetrics {
    total_presses: TotalPresses,
    key_usage: KeyUsage,
    dwell_time: DwellTime,
    flight_time: FlightTime,
    bigram_speed: BigramSpeed,
    corrections: CorrectionSignals,
}

impl SessionMetrics {
    pub fn process(&mut self, event: &KeyEvent) {
        self.total_presses.process(event);
        self.key_usage.process(event);
        self.dwell_time.process(event);
        self.flight_time.process(event);
        self.bigram_speed.process(event);
        self.corrections.process(event);
    }

    pub fn clear_in_flight(&mut self) {
        self.total_presses.clear_in_flight();
        self.key_usage.clear_in_flight();
        self.dwell_time.clear_in_flight();
        self.flight_time.clear_in_flight();
        self.bigram_speed.clear_in_flight();
        self.corrections.clear_in_flight();
    }

    pub fn reset(&mut self) {
        self.total_presses.reset();
        self.key_usage.reset();
        self.dwell_time.reset();
        self.flight_time.reset();
        self.bigram_speed.reset();
        self.corrections.reset();
    }

    pub fn has_data(&self) -> bool {
        self.total_presses.has_data()
            || self.key_usage.has_data()
            || self.dwell_time.has_data()
            || self.flight_time.has_data()
            || self.bigram_speed.has_data()
            || self.corrections.has_data()
    }

    pub fn snapshots(&self) -> Result<Vec<MetricSnapshot>, MetricSnapshotError> {
        Ok(vec![
            self.total_presses.snapshot()?,
            self.key_usage.snapshot()?,
            self.dwell_time.snapshot()?,
            self.flight_time.snapshot()?,
            self.bigram_speed.snapshot()?,
            self.corrections.snapshot()?,
        ])
    }

    pub(crate) fn contains_id(metric_id: &str) -> bool {
        SESSION_METRIC_IDS.contains(&metric_id)
    }

    pub(crate) fn restore_snapshot(
        &mut self,
        snapshot: &MetricSnapshot,
    ) -> Result<(), MetricSnapshotError> {
        match snapshot.metric_id() {
            metric_id if metric_id == <TotalPresses as Metric>::ID => {
                self.total_presses.restore(snapshot)
            }
            metric_id if metric_id == <KeyUsage as Metric>::ID => self.key_usage.restore(snapshot),
            metric_id if metric_id == <DwellTime as Metric>::ID => {
                self.dwell_time.restore(snapshot)
            }
            metric_id if metric_id == <FlightTime as Metric>::ID => {
                self.flight_time.restore(snapshot)
            }
            metric_id if metric_id == <BigramSpeed as Metric>::ID => {
                self.bigram_speed.restore(snapshot)
            }
            metric_id if metric_id == <CorrectionSignals as Metric>::ID => {
                self.corrections.restore(snapshot)
            }
            metric_id => Err(MetricSnapshotError::MetricMismatch {
                expected: "a built-in session metric".to_owned(),
                actual: metric_id.to_owned(),
            }),
        }
    }

    pub(crate) fn total_presses(&self) -> &TotalPresses {
        &self.total_presses
    }

    pub(crate) fn key_usage(&self) -> &KeyUsage {
        &self.key_usage
    }

    pub(crate) fn dwell_time(&self) -> &DwellTime {
        &self.dwell_time
    }

    pub(crate) fn flight_time(&self) -> &FlightTime {
        &self.flight_time
    }

    pub(crate) fn bigram_speed(&self) -> &BigramSpeed {
        &self.bigram_speed
    }

    pub(crate) fn corrections(&self) -> &CorrectionSignals {
        &self.corrections
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        time::{Duration, SystemTime},
    };

    use serde::{Deserialize, Serialize};

    use crate::input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey};

    use super::{
        DurationStats, MAX_AGGREGATE_COUNT, MAX_DIMENSION_BYTES, MAX_DIMENSION_ENTRIES,
        MAX_DURATION_NANOSECONDS, MetricSnapshot, MetricSnapshotError, SESSION_METRIC_IDS,
        SessionMetrics, validate_count, validate_dimension, validate_entry_count,
        validate_scalar_count,
    };

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
    fn validators_reject_unsafe_aggregate_values() {
        assert!(validate_scalar_count("test", 0).is_ok());
        assert!(validate_scalar_count("test", MAX_AGGREGATE_COUNT + 1).is_err());
        assert!(validate_count("test", 1).is_ok());
        assert!(validate_count("test", 0).is_err());
        assert!(validate_dimension("test", &"x".repeat(MAX_DIMENSION_BYTES)).is_ok());
        assert!(validate_dimension("test", &"x".repeat(MAX_DIMENSION_BYTES + 1)).is_err());
        assert!(validate_entry_count("test", MAX_DIMENSION_ENTRIES).is_ok());
        assert!(validate_entry_count("test", MAX_DIMENSION_ENTRIES + 1).is_err());
        assert!(DurationStats::from_snapshot_parts("test", 1, 1).is_ok());
        assert!(
            DurationStats::from_snapshot_parts("test", MAX_DURATION_NANOSECONDS + 1, 1).is_err()
        );
    }

    fn event(
        code: u16,
        label: &str,
        text: Option<&str>,
        at_ms: u64,
        kind: KeyEventKind,
        role: KeyRole,
    ) -> KeyEvent {
        KeyEvent::new(
            PhysicalKey::new(code, label),
            text.map(str::to_owned),
            SystemTime::UNIX_EPOCH + Duration::from_millis(at_ms),
            kind,
            role,
        )
    }

    fn populated_metrics() -> SessionMetrics {
        let mut metrics = SessionMetrics::default();
        metrics.process(&event(
            30,
            "A",
            Some("a"),
            0,
            KeyEventKind::Press,
            KeyRole::Other,
        ));
        metrics.process(&event(
            30,
            "A",
            Some("a"),
            10,
            KeyEventKind::Release,
            KeyRole::Other,
        ));
        metrics.process(&event(
            48,
            "B",
            Some("b"),
            20,
            KeyEventKind::Press,
            KeyRole::Other,
        ));
        metrics.process(&event(
            48,
            "B",
            Some("b"),
            30,
            KeyEventKind::Release,
            KeyRole::Other,
        ));
        metrics.process(&event(
            14,
            "BACKSPACE",
            None,
            40,
            KeyEventKind::Press,
            KeyRole::Backspace,
        ));
        metrics.process(&event(
            46,
            "C",
            Some("c"),
            50,
            KeyEventKind::Press,
            KeyRole::Other,
        ));
        metrics
    }

    fn restore_snapshots(snapshots: &[MetricSnapshot]) -> SessionMetrics {
        let mut restored = SessionMetrics::default();
        for snapshot in snapshots {
            restored.restore_snapshot(snapshot).unwrap();
        }
        restored
    }

    #[test]
    fn failed_restore_does_not_change_session_metrics() {
        let mut metrics = populated_metrics();

        for metric_id in SESSION_METRIC_IDS {
            let before = metrics.snapshots().unwrap();
            let malformed = MetricSnapshot::from_json(metric_id, 1, "{".to_owned()).unwrap();

            assert!(metrics.restore_snapshot(&malformed).is_err());
            assert_eq!(metrics.snapshots().unwrap(), before);
        }
    }

    #[test]
    fn session_metric_ids_have_a_stable_unique_order() {
        let snapshots = SessionMetrics::default().snapshots().unwrap();
        let ids: Vec<_> = snapshots.iter().map(MetricSnapshot::metric_id).collect();
        let unique: HashSet<_> = ids.iter().copied().collect();

        assert_eq!(ids, SESSION_METRIC_IDS);
        assert_eq!(unique.len(), SESSION_METRIC_IDS.len());
        assert!(
            snapshots
                .iter()
                .all(|snapshot| snapshot.schema_version() == 1)
        );
    }

    #[test]
    fn session_metrics_round_trip_all_aggregates() {
        let metrics = populated_metrics();
        let snapshots = metrics.snapshots().unwrap();
        let defaults = SessionMetrics::default().snapshots().unwrap();
        assert!(
            snapshots
                .iter()
                .zip(defaults)
                .all(|(snapshot, default)| snapshot.payload_json() != default.payload_json())
        );

        let restored = restore_snapshots(&snapshots);

        assert!(restored.has_data());
        assert_eq!(restored.snapshots().unwrap(), snapshots);
    }

    #[test]
    fn session_metrics_reset_every_metric() {
        let mut metrics = populated_metrics();
        assert!(metrics.has_data());

        metrics.reset();

        assert!(!metrics.has_data());
        assert_eq!(
            metrics.snapshots().unwrap(),
            SessionMetrics::default().snapshots().unwrap()
        );
    }

    #[test]
    fn clearing_session_in_flight_state_prevents_cross_boundary_analysis() {
        let mut metrics = SessionMetrics::default();
        metrics.process(&event(
            30,
            "A",
            Some("a"),
            0,
            KeyEventKind::Press,
            KeyRole::Other,
        ));
        let snapshots = metrics.snapshots().unwrap();
        let mut restored = restore_snapshots(&snapshots);

        metrics.clear_in_flight();
        let next = event(48, "B", Some("b"), 10, KeyEventKind::Press, KeyRole::Other);
        metrics.process(&next);
        restored.process(&next);

        assert_eq!(metrics.snapshots().unwrap(), restored.snapshots().unwrap());
    }
}

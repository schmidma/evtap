use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use crate::{
    input::{KeyEvent, PhysicalKey},
    metric::SessionMetrics,
    session::{KeyboardContext, MetricRecoveryIssue, SessionId, SessionSnapshot, StoredSession},
};

pub(super) struct WorkingSession {
    pub(super) id: Option<SessionId>,
    pub(super) name: Option<String>,
    pub(super) created_at_ms: i64,
    pub(super) last_opened_at_ms: i64,
    captured_duration: Duration,
    capture_started_at: Option<Instant>,
    pub(super) keyboard: KeyboardContext,
    pub(super) restored: bool,
    pub(super) metrics: SessionMetrics,
    physical_keys: HashMap<u16, PhysicalKey>,
}

impl WorkingSession {
    pub(super) fn untitled(now_ms: i64, keyboard: KeyboardContext) -> Self {
        Self {
            id: None,
            name: None,
            created_at_ms: now_ms,
            last_opened_at_ms: now_ms,
            captured_duration: Duration::ZERO,
            capture_started_at: None,
            keyboard,
            restored: false,
            metrics: SessionMetrics::default(),
            physical_keys: HashMap::new(),
        }
    }

    pub(super) fn restore(stored: StoredSession) -> (Self, Vec<MetricRecoveryIssue>) {
        let (metrics, recovery_issues) = SessionMetrics::restore(&stored.metrics);
        let metadata = stored.metadata;
        let captured_duration =
            Duration::from_nanos(u64::try_from(metadata.captured_duration_ns).unwrap_or_default());
        (
            Self {
                id: Some(metadata.id),
                name: metadata.name,
                created_at_ms: metadata.created_at_ms,
                last_opened_at_ms: metadata.last_opened_at_ms,
                captured_duration,
                capture_started_at: None,
                keyboard: metadata.keyboard,
                restored: true,
                metrics,
                physical_keys: HashMap::new(),
            },
            recovery_issues,
        )
    }

    pub(super) fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("Untitled session")
    }

    pub(super) fn duration(&self) -> Duration {
        self.capture_started_at
            .map_or(self.captured_duration, |started| {
                self.captured_duration.saturating_add(started.elapsed())
            })
    }

    pub(super) fn start_capture(&mut self) {
        self.capture_started_at = Some(Instant::now());
        self.restored = false;
    }

    pub(super) fn finish_capture_segment(&mut self) -> bool {
        let Some(started) = self.capture_started_at.take() else {
            return false;
        };
        self.captured_duration = self.captured_duration.saturating_add(started.elapsed());
        true
    }

    pub(super) fn physical_key(
        &mut self,
        code: u16,
        label: impl FnOnce() -> String,
    ) -> PhysicalKey {
        self.physical_keys
            .entry(code)
            .or_insert_with(|| PhysicalKey::new(code, label()))
            .clone()
    }

    pub(super) fn process(&mut self, event: &KeyEvent) {
        self.metrics.process(event);
    }

    pub(super) fn clear_in_flight(&mut self) {
        self.metrics.clear_in_flight();
        self.physical_keys.clear();
    }

    pub(super) fn has_content(&self) -> bool {
        self.name.is_some()
            || !self.captured_duration.is_zero()
            || self.capture_started_at.is_some()
            || self.metrics.has_data()
    }

    pub(super) fn reset_statistics(&mut self) {
        self.metrics.reset();
        self.physical_keys.clear();
        self.captured_duration = Duration::ZERO;
        self.capture_started_at = None;
    }

    pub(super) fn snapshot(&self, now_ms: i64) -> Result<SessionSnapshot, String> {
        let captured_duration_ns = i64::try_from(self.duration().as_nanos())
            .map_err(|_| "Capture duration exceeds the storage range".to_owned())?;
        let metrics = self
            .metrics
            .snapshots()
            .map_err(|error| error.to_string())?;
        Ok(SessionSnapshot {
            id: self.id,
            name: self.name.clone(),
            created_at_ms: self.created_at_ms,
            updated_at_ms: now_ms,
            last_opened_at_ms: self.last_opened_at_ms,
            captured_duration_ns,
            application_version: env!("CARGO_PKG_VERSION").to_owned(),
            keyboard: self.keyboard.clone(),
            metrics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WorkingSession;
    use crate::session::KeyboardContext;

    #[test]
    fn saving_updates_modified_time_without_changing_open_time() {
        let session = WorkingSession::untitled(10, KeyboardContext::default());

        let snapshot = session.snapshot(20).unwrap();

        assert_eq!(snapshot.updated_at_ms, 20);
        assert_eq!(snapshot.last_opened_at_ms, 10);
    }
}

use super::*;

impl App {
    pub(super) fn drain_storage_events(&mut self) {
        loop {
            let event = match self.storage.as_ref().map(StorageWorker::try_recv) {
                Some(Ok(Some(event))) => event,
                Some(Ok(None)) | None => break,
                Some(Err(error)) => {
                    self.set_storage_failure(
                        StorageOperation::Maintenance,
                        None,
                        format!("Storage worker stopped: {error}"),
                    );
                    self.storage_tracker.set_failed();
                    break;
                }
            };
            self.handle_storage_event(event);
        }
    }

    pub(super) fn handle_storage_event(&mut self, event: StorageEvent) {
        match event {
            StorageEvent::Opened { sessions, selected } => {
                let first_open = !self.initial_storage_open_handled;
                self.initial_storage_open_handled = true;
                self.sessions = sessions;
                self.clear_storage_failure(StorageOperation::Open);
                if first_open {
                    if let Some(selected) = selected {
                        self.restore_session(selected);
                        self.storage_tracker.reset_saved();
                    } else {
                        self.storage_tracker.reset_unsaved();
                        if self.settings.last_session_id().is_some() {
                            self.settings.set_last_session_id(None);
                            self.save_settings();
                        }
                    }
                } else {
                    if self.working_session.id.is_some() {
                        self.storage_tracker.reset_saved();
                    } else {
                        self.storage_tracker.reset_unsaved();
                    }
                    if self.working_session.id.is_none() && self.session_has_content() {
                        self.note_session_dirty();
                    }
                }
            }
            StorageEvent::Saved {
                generation,
                session_id,
            } => {
                self.working_session.id = Some(session_id);
                if let Err(error) = self.storage_tracker.acknowledge(generation) {
                    self.set_storage_failure(
                        StorageOperation::Save,
                        None,
                        format!("Unexpected save acknowledgement: {error}"),
                    );
                    self.storage_tracker.set_failed();
                    self.pending_boundary_after_save = None;
                    return;
                }
                self.settings.set_last_session_id(Some(session_id));
                self.save_settings();
                self.clear_storage_failure(StorageOperation::Save);
                self.clear_storage_failure(StorageOperation::ShutdownSave);
                self.refresh_session_lists();
                if self.storage_tracker.is_dirty() {
                    if let Some(target) = self.pending_boundary_after_save {
                        self.begin_save(Some(target));
                    }
                } else if let Some(target) = self.pending_boundary_after_save.take() {
                    self.execute_boundary(target);
                }
            }
            StorageEvent::SessionsListed {
                request_id,
                sessions,
            } => {
                let matched_order = if self.session_list_request_id == Some(request_id) {
                    self.sessions = sessions;
                    self.session_list_request_id = None;
                    Some(SessionListOrder::LastOpened)
                } else if self.manage_list_request_id == Some(request_id) {
                    self.managed_sessions = sessions;
                    self.manage_list_request_id = None;
                    self.manage_list_loading = false;
                    Some(SessionListOrder::LastUpdated)
                } else {
                    None
                };
                if let Some(order) = matched_order {
                    self.clear_list_storage_failure(order);
                }
            }
            StorageEvent::SessionListFailed {
                request_id,
                order,
                failure,
            } => self.handle_session_list_failed(request_id, order, failure),
            StorageEvent::SessionLoaded {
                request_id,
                session,
            } => {
                if request_id != self.load_request_id {
                    return;
                }
                self.loading_session = false;
                self.clear_storage_failure(StorageOperation::Load);
                match session {
                    Some(session) => {
                        let id = session.metadata.id;
                        self.restore_session(session);
                        self.storage_tracker.reset_saved();
                        self.settings.set_last_session_id(Some(id));
                        self.save_settings();
                        self.refresh_session_lists();
                    }
                    None => {
                        self.new_working_session();
                        self.session_notice = Some(
                            "Started an untitled session. The missing saved session and all other saved data were left unchanged."
                                .to_owned(),
                        );
                    }
                }
            }
            StorageEvent::SessionRenamed {
                request_id,
                session,
            } => self.handle_session_renamed(request_id, session),
            StorageEvent::SessionRenameFailed {
                request_id,
                session_id,
                failure,
            } => self.handle_session_rename_failed(request_id, session_id, failure),
            StorageEvent::SessionDeleted {
                session_id,
                deleted,
            } => {
                self.deleting_session = false;
                self.clear_storage_failure(StorageOperation::Delete);
                if deleted && self.working_session.id == Some(session_id) {
                    self.new_working_session();
                }
                self.refresh_session_lists();
            }
            StorageEvent::AllDeleted => {
                self.deleting_all = false;
                self.clear_storage_failure(StorageOperation::DeleteAll);
                self.sessions.clear();
                self.managed_sessions.clear();
                if self.working_session.id.is_some() {
                    self.new_working_session();
                }
            }
            StorageEvent::Failed(failure) => self.handle_storage_failure(failure, None),
            StorageEvent::ShutdownComplete { .. } => {}
        }
    }

    pub(super) fn set_storage_failure(
        &mut self,
        operation: StorageOperation,
        list_order: Option<SessionListOrder>,
        details: impl Into<String>,
    ) {
        debug_assert!(operation == StorageOperation::List || list_order.is_none());
        self.storage_failure = Some(StorageFailureNotice {
            operation,
            list_order,
            message: storage_failure_message(operation).to_owned(),
            details: details.into(),
        });
    }

    pub(super) fn handle_storage_failure(
        &mut self,
        failure: crate::storage::StorageFailure,
        list_order: Option<SessionListOrder>,
    ) {
        let operation = failure.operation;
        self.set_storage_failure(
            operation,
            list_order,
            format!(
                "{} at {}: {}",
                storage_operation_label(operation),
                failure.database_path.display(),
                failure.details
            ),
        );
        if let Some(generation) = failure.generation {
            let _ = self.storage_tracker.fail(generation);
            self.pending_boundary_after_save = None;
        } else if operation == StorageOperation::Open {
            self.initial_storage_open_handled = true;
            self.storage_tracker.set_failed();
        }
        if operation == StorageOperation::Load {
            self.loading_session = false;
        }
        if operation == StorageOperation::Delete {
            self.deleting_session = false;
        }
        if operation == StorageOperation::DeleteAll {
            self.deleting_all = false;
        }
    }

    pub(super) fn clear_storage_failure(&mut self, operation: StorageOperation) {
        if self
            .storage_failure
            .as_ref()
            .is_some_and(|failure| failure.operation == operation)
        {
            self.storage_failure = None;
        }
    }

    fn clear_list_storage_failure(&mut self, order: SessionListOrder) {
        if self.storage_failure.as_ref().is_some_and(|failure| {
            failure.operation == StorageOperation::List && failure.list_order == Some(order)
        }) {
            self.storage_failure = None;
        }
    }

    pub(super) fn request_save(&mut self, after: Option<BoundaryTarget>) {
        self.request_save_from(after, None);
    }

    pub(super) fn request_save_from(
        &mut self,
        after: Option<BoundaryTarget>,
        opener: Option<egui::Id>,
    ) {
        if !self.settings.storage_disclosure_acknowledged() {
            self.open_disclosure_prompt(DisclosureIntent::Save(after), opener);
            return;
        }
        if let Some(opener) = opener {
            self.focus_after_prompt = Some(opener);
        }
        self.begin_save(after);
    }

    pub(super) fn begin_save(&mut self, after: Option<BoundaryTarget>) {
        if self.storage_tracker.in_flight().is_some() {
            if after.is_some() {
                self.pending_boundary_after_save = after;
            }
            return;
        }
        if self.working_session.id.is_none()
            && matches!(
                self.storage_tracker.status(),
                StorageStatus::Failed | StorageStatus::Saved
            )
        {
            let _ = self.storage_tracker.mark_dirty();
        }
        let generation = match self.storage_tracker.begin_save() {
            Ok(Some(generation)) => generation,
            Ok(None) => {
                if let Some(target) = after {
                    self.execute_boundary(target);
                }
                return;
            }
            Err(error) => {
                self.set_storage_failure(
                    StorageOperation::Save,
                    None,
                    format!("Could not begin save: {error}"),
                );
                return;
            }
        };
        let snapshot = match self.session_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let _ = self.storage_tracker.fail(generation);
                self.set_storage_failure(StorageOperation::Save, None, error);
                return;
            }
        };
        self.pending_boundary_after_save = after;
        let command = StorageCommand::Save(SaveRequest {
            generation,
            snapshot,
        });
        match self.storage.as_ref().map(|worker| worker.send(command)) {
            Some(Ok(())) => self.checkpoint_schedule.save_started(),
            Some(Err(error)) => {
                let _ = self.storage_tracker.fail(generation);
                self.set_storage_failure(
                    StorageOperation::Save,
                    None,
                    format!("Could not request save: {error}"),
                );
                self.pending_boundary_after_save = None;
            }
            None => {
                let _ = self.storage_tracker.fail(generation);
                self.set_storage_failure(
                    StorageOperation::Save,
                    None,
                    "Storage worker is unavailable",
                );
                self.pending_boundary_after_save = None;
            }
        }
    }

    pub(super) fn session_snapshot(&self) -> Result<SessionSnapshot, String> {
        self.working_session.snapshot(unix_now_ms()?)
    }

    fn handle_session_list_failed(
        &mut self,
        request_id: u64,
        order: SessionListOrder,
        failure: crate::storage::StorageFailure,
    ) {
        let matched = match order {
            SessionListOrder::LastOpened if self.session_list_request_id == Some(request_id) => {
                self.session_list_request_id = None;
                true
            }
            SessionListOrder::LastUpdated if self.manage_list_request_id == Some(request_id) => {
                self.manage_list_request_id = None;
                self.manage_list_loading = false;
                true
            }
            _ => false,
        };
        if matched {
            self.handle_storage_failure(failure, Some(order));
        } else {
            tracing::debug!(request_id, ?order, "ignored stale session list failure");
        }
    }

    fn next_list_request_id(&mut self) -> u64 {
        self.next_list_request_id = self.next_list_request_id.wrapping_add(1);
        self.next_list_request_id
    }

    pub(super) fn request_session_list(&mut self) {
        let request_id = self.next_list_request_id();
        self.session_list_request_id = Some(request_id);
        let send_result = self.storage.as_ref().map_or_else(
            || Err("Storage worker is unavailable".to_owned()),
            |worker| {
                worker
                    .send(StorageCommand::ListSessions {
                        request_id,
                        order: SessionListOrder::LastOpened,
                    })
                    .map_err(|error| format!("Could not request saved sessions: {error}"))
            },
        );
        if let Err(error) = send_result {
            self.session_list_request_id = None;
            self.set_storage_failure(
                StorageOperation::List,
                Some(SessionListOrder::LastOpened),
                error,
            );
        }
    }

    pub(super) fn request_manage_session_list(&mut self) {
        let request_id = self.next_list_request_id();
        self.manage_list_request_id = Some(request_id);
        self.manage_list_loading = true;
        let send_result = self.storage.as_ref().map_or_else(
            || Err("Storage worker is unavailable".to_owned()),
            |worker| {
                worker
                    .send(StorageCommand::ListSessions {
                        request_id,
                        order: SessionListOrder::LastUpdated,
                    })
                    .map_err(|error| format!("Could not request saved sessions: {error}"))
            },
        );
        if let Err(error) = send_result {
            self.manage_list_request_id = None;
            self.manage_list_loading = false;
            self.set_storage_failure(
                StorageOperation::List,
                Some(SessionListOrder::LastUpdated),
                error,
            );
        }
    }

    pub(super) fn refresh_session_lists(&mut self) {
        self.request_session_list();
        if matches!(self.view, AppView::Sessions) {
            self.request_manage_session_list();
        }
    }
}

pub(super) fn storage_failure_message(operation: StorageOperation) -> &'static str {
    match operation {
        StorageOperation::Open => {
            "evtap could not open local storage. In-memory capture remains available."
        }
        StorageOperation::Save | StorageOperation::ShutdownSave => {
            "The active session could not be saved. Unsaved changes remain in memory."
        }
        StorageOperation::List => "Saved session metadata could not be refreshed.",
        StorageOperation::Load => {
            "The selected session could not be loaded. The current session remains active."
        }
        StorageOperation::Rename => "The session name could not be saved.",
        StorageOperation::Delete => "The selected saved session could not be deleted.",
        StorageOperation::DeleteAll => "Saved sessions could not be deleted.",
        StorageOperation::Maintenance => "Local storage maintenance could not be completed.",
    }
}

fn storage_operation_label(operation: StorageOperation) -> &'static str {
    match operation {
        StorageOperation::Open => "Could not open session storage",
        StorageOperation::Save => "Could not save session",
        StorageOperation::List => "Could not list saved sessions",
        StorageOperation::Load => "Could not load session",
        StorageOperation::Rename => "Could not rename session",
        StorageOperation::Delete => "Could not delete session",
        StorageOperation::DeleteAll => "Could not delete saved sessions",
        StorageOperation::Maintenance => "Could not reclaim deleted storage",
        StorageOperation::ShutdownSave => "Could not save before shutdown",
    }
}

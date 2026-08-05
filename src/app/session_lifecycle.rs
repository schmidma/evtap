use super::persistence::storage_failure_message;
use super::*;

impl App {
    pub(super) fn restore_session(&mut self, stored: StoredSession) {
        let keyboard = stored.metadata.keyboard.clone();
        let (working_session, recovery_issues) = WorkingSession::restore(stored);
        self.session_notice = None;
        self.recovery_messages = recovery_issues
            .iter()
            .map(metric_recovery_message)
            .collect();
        self.model.clone_from(&keyboard.model);
        self.layout.clone_from(&keyboard.layout);
        self.variant.clone_from(&keyboard.variant);
        self.available_variants = xkb_helper::get_variants(&self.layout);
        self.reinit_xkb();
        self.working_session = working_session;
        self.listener = None;
        self.listener_state = ListenerState::Idle;
        self.clear_in_flight();
        self.select_remembered_device();
    }

    pub(super) fn new_working_session(&mut self) {
        let now_ms = unix_now_ms().unwrap_or_default();
        self.working_session = WorkingSession::untitled(
            now_ms,
            KeyboardContext {
                display_name: None,
                model: self.model.clone(),
                layout: self.layout.clone(),
                variant: self.variant.clone(),
            },
        );
        self.listener = None;
        self.listener_state = ListenerState::Idle;
        self.storage_tracker.reset_unsaved();
        self.checkpoint_schedule.clear();
        self.recovery_messages.clear();
        self.session_notice = None;
        self.settings.set_last_session_id(None);
        self.save_settings();
    }

    pub(super) fn note_session_dirty(&mut self) {
        if let Err(error) = self.storage_tracker.mark_dirty() {
            self.set_storage_failure(
                StorageOperation::Save,
                None,
                format!("Could not track unsaved changes: {error}"),
            );
            return;
        }
        if self.settings.autosave_enabled() {
            self.checkpoint_schedule.note_dirty(Instant::now());
        }
    }

    pub(super) fn working_dirty(&self) -> bool {
        if self.working_session.id.is_some() {
            self.storage_tracker.is_dirty()
        } else {
            self.session_has_content()
        }
    }

    pub(super) fn session_has_content(&self) -> bool {
        self.working_session.has_content()
    }

    pub(super) fn open_prompt(&mut self, kind: ActivePromptKind, opener: Option<egui::Id>) {
        if self.active_prompt.is_some() {
            tracing::debug!("ignored an attempt to open a second application prompt");
            return;
        }
        self.active_prompt = Some(ActivePrompt {
            kind,
            opener,
            needs_initial_focus: true,
        });
    }

    pub(super) fn finish_prompt(&mut self) -> Option<ActivePromptKind> {
        let prompt = self.active_prompt.take()?;
        if self.pending_boundary_after_stop.is_none()
            && let Some(opener) = prompt.opener
        {
            self.focus_after_prompt = Some(opener);
        }
        Some(prompt.kind)
    }

    pub(super) fn resume_deferred_boundary(&mut self) {
        if self.active_prompt.is_none()
            && let Some(target) = self.pending_boundary_after_stop.take()
        {
            self.continue_boundary(target);
        }
    }

    pub(super) fn take_prompt_for_transition(
        &mut self,
    ) -> Option<(ActivePromptKind, Option<egui::Id>)> {
        let prompt = self.active_prompt.take()?;
        Some((prompt.kind, prompt.opener))
    }

    pub(super) fn active_prompt_tag(&self) -> Option<ActivePromptTag> {
        self.active_prompt.as_ref().map(ActivePrompt::tag)
    }

    pub(super) fn take_prompt_initial_focus(&mut self) -> bool {
        self.active_prompt
            .as_mut()
            .is_some_and(|prompt| std::mem::take(&mut prompt.needs_initial_focus))
    }

    pub(super) fn rename_prompt(&self) -> Option<&RenamePrompt> {
        match &self.active_prompt.as_ref()?.kind {
            ActivePromptKind::Rename(prompt) => Some(prompt),
            _ => None,
        }
    }

    pub(super) fn rename_prompt_mut(&mut self) -> Option<(&mut RenamePrompt, &mut bool)> {
        let prompt = self.active_prompt.as_mut()?;
        match &mut prompt.kind {
            ActivePromptKind::Rename(rename) => Some((rename, &mut prompt.needs_initial_focus)),
            _ => None,
        }
    }

    pub(super) fn open_disclosure_prompt(
        &mut self,
        intent: DisclosureIntent,
        opener: Option<egui::Id>,
    ) {
        self.open_prompt(ActivePromptKind::Disclosure(intent), opener);
    }

    pub(super) fn open_manage_sessions(&mut self) {
        self.view = AppView::Sessions;
        self.request_manage_session_list();
    }

    pub(super) fn request_boundary(&mut self, target: BoundaryTarget) {
        self.request_boundary_from(target, None);
    }

    pub(super) fn request_boundary_from(
        &mut self,
        target: BoundaryTarget,
        opener: Option<egui::Id>,
    ) {
        self.pending_boundary_opener = opener;
        if self.listener.is_some() {
            self.pending_boundary_after_stop = Some(target);
            self.stop_listener();
        } else {
            self.clear_in_flight();
            self.continue_boundary(target);
        }
    }

    pub(super) fn continue_boundary(&mut self, target: BoundaryTarget) {
        match boundary_policy(self.working_dirty(), self.settings.autosave_enabled()) {
            BoundaryPolicy::Proceed => {
                self.pending_boundary_opener = None;
                self.execute_boundary(target);
            }
            BoundaryPolicy::Save => {
                self.pending_boundary_opener = None;
                self.request_save(Some(target));
            }
            BoundaryPolicy::Prompt => {
                let opener = self.pending_boundary_opener.take();
                self.open_prompt(ActivePromptKind::Boundary(target), opener);
            }
        }
    }

    pub(super) fn execute_boundary(&mut self, target: BoundaryTarget) {
        self.pending_boundary_after_save = None;
        self.pending_boundary_opener = None;
        match target {
            BoundaryTarget::New => self.new_working_session(),
            BoundaryTarget::Load(session_id) => {
                self.load_request_id = self.load_request_id.wrapping_add(1);
                self.loading_session = true;
                let command = StorageCommand::LoadSession {
                    request_id: self.load_request_id,
                    session_id,
                    opened_at_ms: unix_now_ms().unwrap_or_default(),
                };
                let send_result = self.storage.as_ref().map_or_else(
                    || Err("Storage worker is unavailable".to_owned()),
                    |worker| {
                        worker
                            .send(command)
                            .map_err(|error| format!("Could not request session load: {error}"))
                    },
                );
                if let Err(error) = send_result {
                    self.loading_session = false;
                    self.storage_tracker.set_failed();
                    self.set_storage_failure(StorageOperation::Load, None, error);
                }
            }
            BoundaryTarget::Exit => {
                self.allow_close = true;
            }
        }
    }

    pub(super) fn open_rename_dialog(
        &mut self,
        target: RenameTarget,
        name: Option<&str>,
        opener: egui::Id,
    ) {
        self.session_switcher_open = false;
        self.focus_renamed_session = None;
        self.open_prompt(
            ActivePromptKind::Rename(RenamePrompt {
                target,
                buffer: name.unwrap_or_default().to_owned(),
                error: None,
                request_id: None,
                submitting: false,
            }),
            Some(opener),
        );
    }

    pub(super) fn close_rename_dialog(&mut self) {
        if self.active_prompt_tag() == Some(ActivePromptTag::Rename) {
            let _ = self.finish_prompt();
        }
    }

    fn set_rename_error(&mut self, message: impl Into<String>) {
        if let Some((dialog, needs_initial_focus)) = self.rename_prompt_mut() {
            dialog.error = Some(message.into());
            dialog.submitting = false;
            *needs_initial_focus = true;
        }
    }

    pub(super) fn submit_rename(&mut self) {
        let Some(dialog) = self.rename_prompt() else {
            return;
        };
        if dialog.submitting {
            return;
        }
        let target = dialog.target;
        let trimmed = dialog.buffer.trim();
        if trimmed.len() > MAX_SESSION_NAME_BYTES {
            self.set_rename_error("Session name is longer than 80 UTF-8 bytes.");
            return;
        }
        let name = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        let excluded_id = match target {
            RenameTarget::Current => self.working_session.id,
            RenameTarget::Saved(session_id) => Some(session_id),
        };
        if let Some(name) = &name {
            let saved_conflict = self.sessions.iter().any(|session| {
                Some(session.id) != excluded_id && session.name.as_ref() == Some(name)
            });
            let active_conflict = matches!(
                target,
                RenameTarget::Saved(session_id)
                    if self.working_session.id != Some(session_id)
                        && self.working_session.name.as_ref() == Some(name)
            );
            if saved_conflict || active_conflict {
                self.set_rename_error("Another session already uses that name.");
                return;
            }
        }

        match target {
            RenameTarget::Current => {
                if self.working_session.name != name {
                    self.working_session.name = name;
                    self.note_session_dirty();
                }
                self.close_rename_dialog();
            }
            RenameTarget::Saved(session_id) => {
                let updated_at_ms = match unix_now_ms() {
                    Ok(value) => value,
                    Err(error) => {
                        self.set_rename_error(error);
                        return;
                    }
                };
                self.rename_request_id = self.rename_request_id.wrapping_add(1);
                let request_id = self.rename_request_id;
                let send_result = self.storage.as_ref().map_or_else(
                    || Err("Storage worker is unavailable".to_owned()),
                    |worker| {
                        worker
                            .rename_session(request_id, session_id, name, updated_at_ms)
                            .map_err(|error| format!("Could not request session rename: {error}"))
                    },
                );
                match send_result {
                    Ok(()) => {
                        if let Some((dialog, _)) = self.rename_prompt_mut() {
                            dialog.request_id = Some(request_id);
                            dialog.submitting = true;
                            dialog.error = None;
                        }
                    }
                    Err(error) => {
                        self.set_storage_failure(StorageOperation::Rename, None, error);
                        self.set_rename_error(storage_failure_message(StorageOperation::Rename));
                    }
                }
            }
        }
    }

    pub(super) fn handle_session_renamed(
        &mut self,
        request_id: u64,
        session: Option<SessionMetadata>,
    ) {
        let matches_dialog = self
            .rename_prompt()
            .is_some_and(|dialog| dialog.request_id == Some(request_id));
        if matches_dialog {
            if let Some(session) = session {
                self.close_rename_dialog();
                self.focus_after_prompt = None;
                self.focus_renamed_session = Some(session.id);
                self.clear_storage_failure(StorageOperation::Rename);
            } else {
                self.set_rename_error("The saved session no longer exists.");
            }
        }
        self.refresh_session_lists();
    }

    pub(super) fn handle_session_rename_failed(
        &mut self,
        request_id: u64,
        session_id: SessionId,
        failure: crate::storage::StorageFailure,
    ) {
        let matches_dialog = self.rename_prompt().is_some_and(|dialog| {
            dialog.request_id == Some(request_id)
                && dialog.target == RenameTarget::Saved(session_id)
        });
        if matches_dialog {
            self.handle_storage_failure(failure, None);
            self.set_rename_error(storage_failure_message(StorageOperation::Rename));
        } else {
            tracing::debug!(
                request_id,
                ?session_id,
                "ignored stale session rename failure"
            );
        }
    }

    pub(super) fn reset_statistics(&mut self) {
        self.working_session.reset_statistics();
        self.note_session_dirty();
        let _ = self.finish_prompt();
    }

    pub(super) fn prompt_delete_session(
        &mut self,
        session_id: Option<SessionId>,
        display_name: impl Into<String>,
        current: bool,
        opener: Option<egui::Id>,
    ) {
        self.session_switcher_open = false;
        self.open_prompt(
            ActivePromptKind::DeleteSession(DeleteSessionPrompt {
                session_id,
                display_name: display_name.into(),
                current,
            }),
            opener,
        );
    }

    pub(super) fn delete_prompted_session(&mut self) {
        let Some(ActivePromptKind::DeleteSession(prompt)) = self.finish_prompt() else {
            return;
        };
        let Some(session_id) = prompt.session_id else {
            self.new_working_session();
            return;
        };
        self.deleting_session = true;
        let send_result = self.storage.as_ref().map_or_else(
            || Err("Storage worker is unavailable".to_owned()),
            |worker| {
                worker
                    .send(StorageCommand::DeleteSession { session_id })
                    .map_err(|error| format!("Could not request session deletion: {error}"))
            },
        );
        if let Err(error) = send_result {
            self.deleting_session = false;
            self.set_storage_failure(StorageOperation::Delete, None, error);
        }
    }

    pub(super) fn delete_all_sessions(&mut self) {
        if self.active_prompt_tag() != Some(ActivePromptTag::DeleteAll) {
            return;
        }
        let _ = self.finish_prompt();
        self.deleting_all = true;
        let send_result = self.storage.as_ref().map_or_else(
            || Err("Storage worker is unavailable".to_owned()),
            |worker| {
                worker
                    .send(StorageCommand::DeleteAll)
                    .map_err(|error| format!("Could not request complete deletion: {error}"))
            },
        );
        if let Err(error) = send_result {
            self.deleting_all = false;
            self.set_storage_failure(StorageOperation::DeleteAll, None, error);
        }
    }

    pub(super) fn handle_close_request(&mut self, ctx: &egui::Context) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.active_prompt.is_none() && self.pending_boundary_after_save.is_none() {
                self.request_boundary(BoundaryTarget::Exit);
            }
        }
        if self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

pub(super) fn unix_now_ms() -> Result<i64, String> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock precedes the Unix epoch".to_owned())?
        .as_millis();
    i64::try_from(milliseconds).map_err(|_| "system time exceeds the storage range".to_owned())
}

fn metric_recovery_message(issue: &MetricRecoveryIssue) -> String {
    match issue {
        MetricRecoveryIssue::Unknown { metric_id } => {
            format!("Unsupported saved metric: {metric_id}")
        }
        MetricRecoveryIssue::Duplicate { metric_id } => {
            format!("Duplicate saved metric was ignored: {metric_id}")
        }
        MetricRecoveryIssue::Invalid { metric_id, details } => {
            format!("Could not restore saved metric {metric_id}: {details}")
        }
    }
}

pub(super) fn boundary_policy(dirty: bool, autosave: bool) -> BoundaryPolicy {
    match (dirty, autosave) {
        (false, _) => BoundaryPolicy::Proceed,
        (true, true) => BoundaryPolicy::Save,
        (true, false) => BoundaryPolicy::Prompt,
    }
}

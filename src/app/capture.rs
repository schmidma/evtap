use super::*;

impl App {
    pub(super) fn request_scan(&mut self) {
        self.request_scan_with_remembered_selection(true);
    }

    fn request_recovery_scan(&mut self) {
        self.request_scan_with_remembered_selection(false);
    }

    fn request_scan_with_remembered_selection(&mut self, select_remembered: bool) {
        self.devices = None;
        self.selected_device = None;
        self.scan_warning = None;
        self.scan_error = None;
        self.select_remembered_after_scan = select_remembered;
        if let Err(error) = self.scanner.start_scan() {
            self.devices = Some(Vec::new());
            self.scan_error = Some(format!("Could not start device scan: {error:#}"));
        }
    }

    pub(super) fn apply_scan_report(&mut self, report: scanner::ScanReport) {
        let issue_count = report.issues.len();
        let permission_denied = report
            .issues
            .iter()
            .filter(|issue| issue.kind == scanner::DeviceScanIssueKind::PermissionDenied)
            .count();
        self.scan_warning = if report.devices.is_empty() {
            if permission_denied > 0 {
                Some(ScanWarning::PermissionDenied {
                    count: permission_denied,
                })
            } else if issue_count > 0 {
                Some(ScanWarning::Unavailable { count: issue_count })
            } else {
                Some(ScanWarning::NoKeyboardDetected)
            }
        } else if issue_count > 0 {
            Some(ScanWarning::Incomplete {
                issue_count,
                permission_denied,
            })
        } else {
            None
        };
        self.scan_error = None;
        self.devices = Some(report.devices);
        if self.select_remembered_after_scan {
            self.select_remembered_device();
        } else {
            self.selected_device = None;
            self.select_remembered_after_scan = true;
        }
    }

    pub(super) fn drain_scanner_events(&mut self) {
        while let Some(event) = self.scanner.try_recv_event() {
            match event {
                scanner::Event::ScanFinished { result } => match result {
                    Ok(report) => self.apply_scan_report(report),
                    Err(error) => {
                        self.devices = Some(Vec::new());
                        self.selected_device = None;
                        self.scan_warning = None;
                        self.scan_error = Some(format!("Device scan failed: {error}"));
                        self.select_remembered_after_scan = true;
                    }
                },
            }
        }
    }

    pub(super) fn select_remembered_device(&mut self) {
        let Some(name) = self.working_session.keyboard.display_name.as_deref() else {
            return;
        };
        let Some(devices) = &self.devices else {
            return;
        };
        let mut matches = devices
            .iter()
            .enumerate()
            .filter(|(_, device)| device.name == name)
            .map(|(index, _)| index);
        let first = matches.next();
        self.selected_device = if first.is_some() && matches.next().is_none() {
            first
        } else {
            None
        };
    }

    pub(super) fn drain_listener_events(&mut self) {
        while let Some(event) = self
            .listener
            .as_mut()
            .and_then(ListenerHandle::try_recv_event)
        {
            match event {
                listener::Event::Connected => {
                    self.listener_state = ListenerState::Listening;
                    self.capture_error = None;
                    self.working_session.start_capture();
                    info!("listener connected to keyboard");
                }
                listener::Event::Stopped { reason } => {
                    let is_error = reason.is_error();
                    let message = reason.to_string();
                    self.finish_listener_stop(message.clone(), is_error);
                    info!(%message, "listener stopped");
                }
                listener::Event::Input {
                    timestamp,
                    key_code,
                    kind,
                } => self.process_input(timestamp, key_code, kind),
            }
        }
    }

    pub(super) fn finish_listener_stop(&mut self, message: String, is_error: bool) {
        self.listener = None;
        if self.working_session.finish_capture_segment() {
            self.note_session_dirty();
        }
        self.clear_in_flight();
        self.listener_state = if is_error {
            self.capture_error = Some(message);
            self.request_recovery_scan();
            ListenerState::Failed
        } else {
            self.capture_error = None;
            ListenerState::Idle
        };
        if let Some(target) = self.pending_boundary_after_stop.take() {
            if self.active_prompt.is_some() {
                self.pending_boundary_after_stop = Some(target);
            } else {
                self.continue_boundary(target);
            }
        } else if self.settings.autosave_enabled() && self.working_dirty() {
            self.request_save(None);
        }
    }

    pub(super) fn process_input(
        &mut self,
        timestamp: SystemTime,
        key_code: KeyCode,
        kind: KeyEventKind,
    ) {
        let code = key_code.code();
        let xkb_code = (code + 8).into();
        let text = self.xkb_state.key_get_utf8(xkb_code);
        let text = (!text.is_empty()).then_some(text);

        match kind {
            KeyEventKind::Press => {
                self.xkb_state.update_key(xkb_code, xkb::KeyDirection::Down);
            }
            KeyEventKind::Release => {
                self.xkb_state.update_key(xkb_code, xkb::KeyDirection::Up);
            }
            KeyEventKind::Repeat => {}
        }

        let key = self.working_session.physical_key(code, || {
            let debug_name = format!("{key_code:?}");
            debug_name
                .strip_prefix("KEY_")
                .unwrap_or(&debug_name)
                .to_owned()
        });
        let role = if key_code == KeyCode::KEY_BACKSPACE {
            KeyRole::Backspace
        } else {
            KeyRole::Other
        };
        let event = KeyEvent::new(key, text, timestamp, kind, role);
        self.working_session.process(&event);
        self.note_session_dirty();
    }

    pub(super) fn clear_in_flight(&mut self) {
        self.working_session.clear_in_flight();
        self.reinit_xkb();
    }

    pub(super) fn begin_listening(&mut self, device_index: usize) {
        let Some(device) = self
            .devices
            .as_ref()
            .and_then(|devices| devices.get(device_index))
            .cloned()
        else {
            return;
        };
        self.working_session.keyboard = KeyboardContext {
            display_name: Some(device.name.clone()),
            model: self.model.clone(),
            layout: self.layout.clone(),
            variant: self.variant.clone(),
        };
        self.working_session.last_opened_at_ms =
            unix_now_ms().unwrap_or(self.working_session.last_opened_at_ms);
        self.working_session.restored = false;
        self.note_session_dirty();
        self.listener = Some(listener::spawn(device.path, self.wake_signal.clone()));
        self.listener_state = ListenerState::Connecting;
        self.capture_error = None;
    }

    pub(super) fn stop_listener(&mut self) {
        let stop_result = self.listener.as_ref().map(ListenerHandle::stop);
        match stop_result {
            Some(Ok(())) => self.listener_state = ListenerState::Stopping,
            Some(Err(error)) => {
                self.finish_listener_stop(format!("Could not stop listener: {error:#}"), true);
            }
            None => {}
        }
    }
}

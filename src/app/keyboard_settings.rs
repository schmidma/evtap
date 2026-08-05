use super::*;

impl App {
    pub(super) fn update_variants(&mut self) {
        self.available_variants = xkb_helper::get_variants(&self.layout);
        if !self.available_variants.contains(&self.variant) {
            self.variant.clear();
        }
    }

    pub(super) fn reinit_xkb(&mut self) {
        match init_keyboard_state(&self.model, &self.layout, &self.variant) {
            Ok(state) => {
                self.xkb_state = state;
                self.keyboard_error = None;
            }
            Err(error) => {
                let message = format!("Could not apply keyboard configuration: {error:#}");
                error!(%message);
                self.keyboard_error = Some(message);
            }
        }
    }

    pub(super) fn apply_keyboard_settings(&mut self) -> bool {
        let state = match init_keyboard_state(&self.model, &self.layout, &self.variant) {
            Ok(state) => {
                self.keyboard_error = None;
                state
            }
            Err(error) => {
                let message = format!("Could not apply keyboard configuration: {error:#}");
                error!(%message);
                self.keyboard_error = Some(message);
                return false;
            }
        };

        let previous_model = self.settings.keyboard_model().to_owned();
        let previous_layout = self.settings.keyboard_layout().to_owned();
        let previous_variant = self.settings.keyboard_variant().to_owned();
        self.settings.set_keyboard(
            self.model.clone(),
            self.layout.clone(),
            self.variant.clone(),
        );
        if !self.save_settings() {
            self.settings
                .set_keyboard(previous_model, previous_layout, previous_variant);
            return false;
        }

        self.xkb_state = state;
        self.working_session.keyboard.model.clone_from(&self.model);
        self.working_session
            .keyboard
            .layout
            .clone_from(&self.layout);
        self.working_session
            .keyboard
            .variant
            .clone_from(&self.variant);
        if self.working_session.id.is_some() || self.session_has_content() {
            self.note_session_dirty();
        }
        true
    }

    pub(super) fn save_settings(&mut self) -> bool {
        if self.settings_load_failed {
            self.settings_error = Some(
                "Settings were not changed because the existing settings file could not be read. Fix or remove it before changing preferences."
                    .to_owned(),
            );
            return false;
        }
        match self.settings_store.save(&self.settings) {
            Ok(()) => {
                self.settings_error = None;
                true
            }
            Err(error) => {
                self.settings_error = Some(format!("Could not save settings: {error}"));
                false
            }
        }
    }
}

pub(super) fn init_keyboard_state(model: &str, layout: &str, variant: &str) -> Result<xkb::State> {
    let context = Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = Keymap::new_from_names(
        &context,
        "",
        model,
        layout,
        variant,
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .wrap_err("failed to create XKB keymap")?;
    Ok(xkb::State::new(&keymap))
}

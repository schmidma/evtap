use std::{sync::Arc, time::SystemTime};

/// Stable identity and display label for a physical key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PhysicalKey {
    code: u16,
    label: Arc<str>,
}

impl PhysicalKey {
    pub fn new(code: u16, label: impl Into<Arc<str>>) -> Self {
        Self {
            code,
            label: label.into(),
        }
    }

    pub fn code(&self) -> u16 {
        self.code
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyEventKind {
    Press,
    Release,
    Repeat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyRole {
    Backspace,
    Other,
}

/// UI- and backend-independent input consumed by metrics.
#[derive(Clone, Debug)]
pub struct KeyEvent {
    key: PhysicalKey,
    text: Option<String>,
    timestamp: SystemTime,
    kind: KeyEventKind,
    role: KeyRole,
}

impl KeyEvent {
    pub fn new(
        key: PhysicalKey,
        text: Option<String>,
        timestamp: SystemTime,
        kind: KeyEventKind,
        role: KeyRole,
    ) -> Self {
        Self {
            key,
            text,
            timestamp,
            kind,
            role,
        }
    }

    pub fn key(&self) -> &PhysicalKey {
        &self.key
    }

    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn timestamp(&self) -> SystemTime {
        self.timestamp
    }

    pub fn kind(&self) -> KeyEventKind {
        self.kind
    }

    pub fn role(&self) -> KeyRole {
        self.role
    }
}

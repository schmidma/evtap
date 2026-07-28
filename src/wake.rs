use std::sync::Arc;

/// Wakes the application event loop when a background worker has new data.
#[derive(Clone)]
pub struct WakeSignal {
    callback: Arc<dyn Fn() + Send + Sync>,
}

impl WakeSignal {
    pub fn new(callback: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            callback: Arc::new(callback),
        }
    }

    pub fn notify(&self) {
        (self.callback)();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::WakeSignal;

    #[test]
    fn cloned_signal_invokes_callback() {
        let wake_count = Arc::new(AtomicUsize::new(0));
        let callback_count = Arc::clone(&wake_count);
        let signal = WakeSignal::new(move || {
            callback_count.fetch_add(1, Ordering::Relaxed);
        });

        signal.clone().notify();

        assert_eq!(wake_count.load(Ordering::Relaxed), 1);
    }
}

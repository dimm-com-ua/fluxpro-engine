//! Shared shutdown notification for the queue runner.

use tokio::sync::watch;

/// Cloneable shutdown signal shared by a runner and its owner.
#[derive(Clone)]
pub struct Shutdown {
    tx: watch::Sender<bool>,
    rx: watch::Receiver<bool>,
}

impl Shutdown {
    /// Creates a shared shutdown signal initially set to `false`.
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self { tx, rx }
    }
    /// Notifies subscribed runners to stop accepting new work.
    pub fn trigger(&self) {
        self.tx.send(true).unwrap();
    }
    /// Returns a receiver for this shared shutdown signal.
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.rx.clone()
    }
}

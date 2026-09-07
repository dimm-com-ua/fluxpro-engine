use tokio::sync::watch;

#[derive(Clone)]
pub struct Shutdown {
    tx: watch::Sender<bool>,
    rx: watch::Receiver<bool>,
}

impl Shutdown {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self { tx, rx }
    }
    pub fn trigger(&self) {
        self.tx.send(true).unwrap();
    }
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.rx.clone()
    }
}

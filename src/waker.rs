//! Tells the window's event pump that something changed. Workers hold one of these and don't know about Tauri.

use std::sync::mpsc::Sender;

#[derive(Clone, Default)]
pub struct Waker(Option<Sender<()>>);

impl Waker {
    pub fn new(tx: Sender<()>) -> Self {
        Waker(Some(tx))
    }

    pub fn request_repaint(&self) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(());
        }
    }
}

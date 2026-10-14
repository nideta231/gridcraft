//! Save dialogs that don't hold the frame, and what the answer to them is for.
//!
//! # Why this exists
//!
//! `Services::pick_save` is the blocking shape: the host shows a panel and returns the chosen path
//! in the same call. That works on the desktop (`rfd`) and cannot work on a host that draws its own
//! picker between frames — the browser, and Android, where there is no native dialog to call at all.
//! Those hosts return a receiver instead ([`Services::pick_save_async`]) and the app does what the
//! blocking call would have done on the frame the path arrives, exactly as
//! [`crate::dialogs`]/[`crate::backstage`] already do for pictures and opening.
//!
//! # The cost of not having it
//!
//! Handing the host finished *bytes* instead (the web `download` path) looks equivalent and is not:
//! the app is never told where the file went, so `DocState::path` stays `None` and the workbook is
//! still "Book1" with unsaved changes — the title bar never changes, `Ctrl+S` asks for a location
//! again, and closing prompts to save what was just saved. Every save dialog in this app goes
//! through [`SheetApp::ask_save`] so that cannot happen.
//!
//! [`Services::pick_save_async`]: crate::Services::pick_save_async

use std::sync::mpsc::{Receiver, TryRecvError};

use crate::SheetApp;

/// A save dialog the host is showing: the answer arrives on `rx`, and `after` is what the blocking
/// `pick_save` would have run with it.
pub(crate) struct PendingSave {
    rx: Receiver<Option<String>>,
    after: Box<dyn FnOnce(&mut SheetApp, &str)>,
}

impl SheetApp {
    /// Whether a save dialog is open (its answer hasn't arrived yet).
    pub fn save_dialog_open(&self) -> bool {
        self.pending_save.is_some()
    }

    /// Ask for a path to save to, and do `after` with it: straight away for a blocking host dialog,
    /// on a later frame for an asynchronous one.
    ///
    /// Returns false when the host has neither, which is how a caller knows to fall back to its
    /// browser-download path — and only then, because a host with a picker must never be handed
    /// bytes it cannot name.
    pub(crate) fn ask_save(&mut self, suggested: &str, after: impl FnOnce(&mut SheetApp, &str) + 'static) -> bool {
        // One dialog at a time: the app has one pending action, and while it is waiting the answer
        // to it decides what happens.
        if self.pending_save.is_some() {
            return true;
        }
        if let Some(pick) = &self.services.pick_save {
            if let Some(path) = pick(suggested) {
                after(self, &path);
            }
            return true;
        }
        let Some(ask) = &self.services.pick_save_async else {
            return false;
        };
        self.pending_save = Some(PendingSave { rx: ask(suggested), after: Box::new(after) });
        true
    }

    /// Use the answer, once there is one. Called every frame from [`SheetApp::logic`].
    pub(crate) fn poll_save(&mut self) {
        let Some(pending) = self.pending_save.as_ref() else {
            return;
        };
        match pending.rx.try_recv() {
            Err(TryRecvError::Empty) => {}
            Ok(answer) => {
                let Some(PendingSave { after, .. }) = self.pending_save.take() else {
                    return;
                };
                // Nothing to do with a cancelled dialog, which is what an empty answer is.
                if let Some(path) = answer {
                    after(self, &path);
                }
            }
            // The host went away without answering (its sender was dropped): a cancellation, not a
            // dialog this app will wait on for ever.
            Err(TryRecvError::Disconnected) => self.pending_save = None,
        }
    }
}

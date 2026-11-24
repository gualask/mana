//! The candidates of an item of a source, asked for on a thread of their
//! own while the screens wait, so that the viewer chooses one: the remote
//! never waits on the network.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::Arc;

use super::reopen;
use crate::net::Client;
use crate::source::Selection;
use crate::source::run::{Candidate, Cause, Failure, Outcome};

/// The answer on its way; dropping it gives the request up.
pub struct Candidates {
    answer: Receiver<Result<Vec<Candidate>, Failure>>,
    cancelled: Arc<AtomicBool>,
}

impl Candidates {
    /// Asks the source of `selection` for the candidates of its item.
    pub fn ask(selection: &Selection) -> Self {
        let (sender, answer) = channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (selection, stop, failed) = (selection.clone(), cancelled.clone(), sender.clone());
        let started = std::thread::Builder::new().name("candidates".into()).spawn(move || {
            let found = match Client::new() {
                Ok(client) => reopen::engine(&selection, &client, &stop).sources(&selection.item),
                Err(detail) => Err(failure(detail)),
            };
            let _ = sender.send(found);
        });
        if let Err(error) = started {
            let _ = failed.send(Err(failure(error.to_string())));
        }
        Self { answer, cancelled }
    }

    /// The answer, once it came: the candidates in the source's order, or
    /// why there are none.
    pub fn poll(&self) -> Option<Result<Vec<Candidate>, Failure>> {
        match self.answer.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(Failure { outcome: Outcome::Cancelled, at: "sources".into(), cause: Cause::None }))
            }
        }
    }
}

/// The candidate to play without asking the viewer: an item's only one.
/// With several, the viewer chooses: Mana never passes to another by itself.
pub fn chosen_at_once(found: &[Candidate]) -> Option<&Candidate> {
    match found {
        [only] => Some(only),
        _ => None,
    }
}

/// Why no request could be made, as the system says.
fn failure(detail: String) -> Failure {
    Failure { outcome: Outcome::NetworkError, at: "sources".into(), cause: Cause::Problem(detail) }
}

impl Drop for Candidates {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

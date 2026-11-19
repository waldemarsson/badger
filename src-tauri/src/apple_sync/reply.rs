//! Replies from the EventKit worker thread, with a deadline that can't lose a result.
//!
//! A call that times out may still complete on the worker afterwards. Waiting closes the
//! reply channel when the deadline passes, so a late result either arrived first (and is
//! returned after all) or is handed back to the worker, which undoes it (a created phone
//! item is deleted again instead of staying unpaired and being imported as a duplicate).

use std::time::Duration;

use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyError {
    TimedOut,
    /// The worker went away without answering.
    Stopped,
}

/// Waits up to `limit` for the worker's reply.
pub async fn await_reply<T>(
    mut rx: oneshot::Receiver<T>,
    limit: Duration,
) -> Result<T, ReplyError> {
    match tokio::time::timeout(limit, &mut rx).await {
        Ok(reply) => reply.map_err(|_| ReplyError::Stopped),
        Err(_) => {
            // From here on the worker's send fails and it gets its value back.
            rx.close();
            rx.try_recv().map_err(|_| ReplyError::TimedOut)
        }
    }
}

/// Worker side: sends `value`, or hands it to `undo` when nobody waits any more.
pub fn deliver<T>(tx: oneshot::Sender<T>, value: T, undo: impl FnOnce(T)) {
    if let Err(value) = tx.send(value) {
        undo(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const LIMIT: Duration = Duration::from_millis(50);

    #[tokio::test]
    async fn an_answer_in_time_is_delivered_and_not_undone() {
        let (tx, rx) = oneshot::channel();
        let undone = Arc::new(Mutex::new(Vec::new()));
        let u = undone.clone();
        deliver(tx, "ek-1", move |v| u.lock().unwrap().push(v));
        assert_eq!(await_reply(rx, LIMIT).await, Ok("ek-1"));
        assert!(undone.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_late_answer_is_handed_back_to_be_undone() {
        let (tx, rx) = oneshot::channel();
        let undone = Arc::new(Mutex::new(Vec::new()));
        let u = undone.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(LIMIT * 4);
            deliver(tx, "ek-late", move |v| u.lock().unwrap().push(v));
        });
        assert_eq!(await_reply(rx, LIMIT).await, Err(ReplyError::TimedOut));
        worker.join().unwrap();
        assert_eq!(*undone.lock().unwrap(), vec!["ek-late"], "no orphan");
    }

    #[tokio::test]
    async fn a_worker_that_stops_without_answering_is_reported() {
        let (tx, rx) = oneshot::channel::<()>();
        drop(tx);
        assert_eq!(await_reply(rx, LIMIT).await, Err(ReplyError::Stopped));
    }
}

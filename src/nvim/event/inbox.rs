//! Sending and closing share a lock so a wake cannot race handle destruction.

use std::collections::VecDeque;
use std::sync::Mutex;

struct State<T, W> {
    wake: Option<W>,
    messages: VecDeque<T>,
}

pub struct Inbox<T, W> {
    state: Mutex<State<T, W>>,
}

impl<T, W> Inbox<T, W> {
    pub fn new(wake: W) -> Self {
        Self {
            state: Mutex::new(State {
                wake: Some(wake),
                messages: VecDeque::new(),
            }),
        }
    }

    pub fn send(&self, message: T, wake: impl FnOnce(&W)) -> Result<(), T> {
        let mut state = self.state.lock().unwrap();
        if state.wake.is_none() {
            return Err(message);
        }
        state.messages.push_back(message);
        wake(state.wake.as_ref().unwrap());
        Ok(())
    }

    pub fn take(&self) -> VecDeque<T> {
        std::mem::take(&mut self.state.lock().unwrap().messages)
    }

    pub fn close(&self) -> VecDeque<T> {
        let mut state = self.state.lock().unwrap();
        let wake = state.wake.take();
        let messages = std::mem::take(&mut state.messages);
        drop(state);
        drop(wake);
        messages
    }

    pub fn len(&self) -> usize {
        self.state.lock().unwrap().messages.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn close_waits_for_inflight_wake_and_rejects_late_sends() {
        let inbox = Arc::new(Inbox::new(()));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        std::thread::scope(|scope| {
            let sender = inbox.clone();
            let sender_entered = entered.clone();
            let sender_release = release.clone();
            scope.spawn(move || {
                sender
                    .send(17, |_| {
                        sender_entered.wait();
                        sender_release.wait();
                    })
                    .unwrap();
            });
            entered.wait();
            let closing = inbox.clone();
            let closer = scope.spawn(move || closing.close());
            release.wait();
            let messages = closer.join().unwrap();
            assert_eq!(messages.into_iter().collect::<Vec<_>>(), vec![17]);
        });
        assert_eq!(inbox.send(23, |_| panic!("wake after close")), Err(23));
        assert!(inbox.take().is_empty());
    }
}

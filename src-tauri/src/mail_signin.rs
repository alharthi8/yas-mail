//! A mailbox sign-in can be cancelled only before its browser callback is accepted.
use crate::Result;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

struct Attempt {
    id: u64,
    cancelled: bool,
    notify: Arc<Notify>,
}

#[derive(Default)]
pub struct Registry {
    attempts: Mutex<HashMap<String, Attempt>>,
}

pub struct Pending {
    registry: Arc<Registry>,
    window: String,
    id: u64,
    notify: Arc<Notify>,
}

impl Registry {
    pub fn begin(self: &Arc<Self>, window: &str) -> Result<Pending> {
        let mut attempts = self
            .attempts
            .lock()
            .map_err(|_| "Sign-in is busy. Please try again.")?;
        if attempts.contains_key(window) {
            return Err("A sign-in is already open in this window. Finish it or cancel before trying again.".into());
        }
        let id = rand::random();
        let notify = Arc::new(Notify::new());
        attempts.insert(
            window.into(),
            Attempt {
                id,
                cancelled: false,
                notify: notify.clone(),
            },
        );
        Ok(Pending {
            registry: self.clone(),
            window: window.into(),
            id,
            notify,
        })
    }

    pub fn cancel(&self, window: &str) -> bool {
        let Ok(mut attempts) = self.attempts.lock() else {
            return false;
        };
        let Some(attempt) = attempts.get_mut(window) else {
            return false;
        };
        attempt.cancelled = true;
        attempt.notify.notify_one();
        true
    }
}

impl Pending {
    pub async fn cancelled(&self) {
        self.notify.notified().await;
    }

    pub fn accept_callback(&self) -> bool {
        let Ok(mut attempts) = self.registry.attempts.lock() else {
            return false;
        };
        let ours = attempts
            .get(&self.window)
            .is_some_and(|attempt| attempt.id == self.id);
        if !ours {
            return false;
        }
        // Removing under the same lock linearizes acceptance with cancellation:
        // a successful cancel can never be reported after token exchange starts.
        !attempts
            .remove(&self.window)
            .expect("checked attempt exists")
            .cancelled
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Ok(mut attempts) = self.registry.attempts.lock() {
            if attempts
                .get(&self.window)
                .is_some_and(|attempt| attempt.id == self.id)
            {
                attempts.remove(&self.window);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn cancellation_is_window_scoped_and_survives_an_unpolled_waiter() {
        let registry = Arc::new(Registry::default());
        let first = registry.begin("first").unwrap();
        let second = registry.begin("second").unwrap();
        assert!(registry.cancel("first"));
        tokio::time::timeout(Duration::from_millis(100), first.cancelled())
            .await
            .unwrap();
        assert!(!first.accept_callback());
        assert!(
            tokio::time::timeout(Duration::from_millis(10), second.cancelled())
                .await
                .is_err()
        );
        assert!(second.accept_callback());
        assert!(!registry.cancel("second"));
        assert!(!registry.cancel("missing"));
    }

    #[test]
    fn callback_acceptance_and_cancel_have_one_winner() {
        let registry = Arc::new(Registry::default());
        let accepted = registry.begin("main").unwrap();
        assert!(accepted.accept_callback());
        assert!(!registry.cancel("main"));
        let cancelled = registry.begin("main").unwrap();
        assert!(registry.cancel("main"));
        assert!(!cancelled.accept_callback());
    }

    #[test]
    fn guards_clear_failed_attempts_without_removing_a_new_attempt() {
        let registry = Arc::new(Registry::default());
        let first = registry.begin("main").unwrap();
        assert!(registry.begin("main").is_err());
        drop(first);
        assert!(!registry.cancel("main"));
        let accepted = registry.begin("main").unwrap();
        assert!(accepted.accept_callback());
        let newer = registry.begin("main").unwrap();
        drop(accepted);
        assert!(registry.cancel("main"));
        drop(newer);
        assert!(!registry.cancel("main"));
    }
}

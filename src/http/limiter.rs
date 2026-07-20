use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

/// Simple per-key login rate limiter: 8 attempts per 60 seconds.
pub struct LoginLimiter {
    attempts: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl LoginLimiter {
    pub fn new() -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
        }
    }

    pub fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        let queue = map.entry(key.to_string()).or_default();
        while let Some(front) = queue.front() {
            if now.duration_since(*front).as_secs() >= 60 {
                queue.pop_front();
            } else {
                break;
            }
        }
        if queue.len() >= 8 {
            return false;
        }
        queue.push_back(now);
        true
    }
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::new()
    }
}

use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationKey {
    pub account_id: String,
    pub metric: String,
    pub threshold: u8,
    pub window_id: String,
}

#[derive(Default)]
pub struct NotificationEngine {
    delivered: HashSet<NotificationKey>,
}

impl NotificationEngine {
    pub fn restore(keys: impl IntoIterator<Item = NotificationKey>) -> Self {
        Self {
            delivered: keys.into_iter().collect(),
        }
    }
    pub fn should_deliver(
        &mut self,
        key: NotificationKey,
        fresh: bool,
        backfilled: bool,
        quiet_hours: Option<(NaiveTime, NaiveTime)>,
        now: DateTime<Utc>,
    ) -> bool {
        if !fresh
            || backfilled
            || self.delivered.contains(&key)
            || quiet_hours.is_some_and(|(start, end)| in_quiet_hours(now.time(), start, end))
        {
            return false;
        }
        self.delivered.insert(key);
        true
    }
    pub fn delivered(&self) -> &HashSet<NotificationKey> {
        &self.delivered
    }
}

fn in_quiet_hours(now: NaiveTime, start: NaiveTime, end: NaiveTime) -> bool {
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> NotificationKey {
        NotificationKey {
            account_id: "a".into(),
            metric: "quota".into(),
            threshold: 80,
            window_id: "w".into(),
        }
    }
    #[test]
    fn deduplicates_after_first_delivery() {
        let mut e = NotificationEngine::default();
        assert!(e.should_deliver(key(), true, false, None, Utc::now()));
        assert!(!e.should_deliver(key(), true, false, None, Utc::now()));
    }
    #[test]
    fn suppresses_stale_and_backfill() {
        let mut e = NotificationEngine::default();
        assert!(!e.should_deliver(key(), false, false, None, Utc::now()));
        assert!(!e.should_deliver(key(), true, true, None, Utc::now()));
    }
}

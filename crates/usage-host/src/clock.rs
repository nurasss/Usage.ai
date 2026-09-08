use chrono::{DateTime, Utc};

pub trait ClockHost: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl ClockHost for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub DateTime<Utc>);

impl ClockHost for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

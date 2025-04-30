use std::time::{Duration, Instant};
use tokio::time::{Interval, MissedTickBehavior};

pub fn new_skip_interval_after(period: Duration) -> Interval {
    new_skip_interval_at(Instant::now() + period, period)
}

pub fn new_skip_interval(period: Duration) -> Interval {
    new_skip_interval_at(Instant::now(), period)
}

pub fn new_skip_interval_at(start: Instant, period: Duration) -> Interval {
    let mut interval = tokio::time::interval_at(start.into(), period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    interval
}

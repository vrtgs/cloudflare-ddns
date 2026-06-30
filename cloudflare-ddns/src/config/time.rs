use serde::{Deserialize, Deserializer};
use std::time::Duration;
use toml::value::Datetime;

#[derive(Debug, Eq, Ord, PartialOrd, PartialEq)]
pub struct Time(pub Duration);

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = Datetime::deserialize(deserializer)?;

        match val {
            Datetime {
                date: None,
                time:
                    Some(toml::value::Time {
                        hour,
                        minute,
                        second,
                        nanosecond,
                    }),
                offset: None,
            } => {
                let duration = Duration::from_hours(u64::from(hour))
                    .saturating_add(Duration::from_mins(u64::from(minute)))
                    .saturating_add(Duration::from_secs(u64::from(second.unwrap_or(0))))
                    .saturating_add(Duration::from_nanos(u64::from(nanosecond.unwrap_or(0))));

                Ok(Time(duration))
            }
            _ => Err(serde::de::Error::custom("expected a toml time value")),
        }
    }
}

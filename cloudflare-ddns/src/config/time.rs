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
            } => Ok(Time(Duration::new(
                (hour as u64 * 60 * 60) + (minute as u64 * 60) + second as u64,
                nanosecond,
            ))),
            _ => Err(serde::de::Error::custom(
                "expected a time value in the format of 'HH:MM:SS(.nnnnnnnnn optional)'",
            )),
        }
    }
}

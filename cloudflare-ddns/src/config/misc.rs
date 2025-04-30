use crate::config::time::Time;
use crate::config::Deserializable;
use anyhow::Result;
use serde::Deserialize;
use std::num::NonZeroU8;
use std::time::Duration;

#[derive(Debug, Eq, Ord, PartialOrd, PartialEq, Deserialize)]
pub struct RefreshConfig {
    #[serde(default = "RefreshConfig::default_interval")]
    interval: Time,
    #[serde(default = "RefreshConfig::default_network_detection")]
    #[serde(alias = "network-detection")]
    network_detection: bool,
}

impl RefreshConfig {
    #[inline]
    const fn default_interval() -> Time {
        Time(Duration::from_secs(60 * 60)) // 1 hour
    }

    #[inline]
    const fn default_network_detection() -> bool {
        true
    }

    pub fn interval(&self) -> Duration {
        self.interval.0
    }
    pub fn network_detection(&self) -> bool {
        self.network_detection
    }
}

#[derive(Debug, Eq, Ord, PartialOrd, PartialEq, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "GeneralConfig::default_max_errors")]
    max_errors: NonZeroU8,
}

impl GeneralConfig {
    #[inline]
    const fn default_max_errors() -> NonZeroU8 {
        unsafe { NonZeroU8::new_unchecked(5) }
    }

    pub fn max_errors(&self) -> NonZeroU8 {
        self.max_errors
    }
}

#[derive(Debug, Eq, Ord, PartialOrd, PartialEq, Deserialize)]
pub struct MiscConfig {
    refresh: RefreshConfig,
    general: GeneralConfig,
}

impl MiscConfig {
    pub fn refresh(&self) -> &RefreshConfig {
        &self.refresh
    }

    pub fn general(&self) -> &GeneralConfig {
        &self.general
    }
}

impl Deserializable for MiscConfig {
    async fn deserialize(text: &str) -> Result<Self> {
        Ok(toml::de::from_str(text)?)
    }
}

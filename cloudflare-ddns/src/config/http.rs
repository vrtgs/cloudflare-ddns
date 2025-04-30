use crate::config::time::Time;
use crate::config::Deserializable;
use anyhow::Result;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Eq, Ord, PartialOrd, PartialEq, Deserialize)]
pub struct ClientConfig {
    #[serde(default = "ClientConfig::default_max_retries")]
    #[serde(alias = "max-retries")]
    max_retries: u8,
    #[serde(default = "ClientConfig::default_timeout")]
    #[serde(alias = "retry-interval")]
    retry_interval: Time,
    #[serde(default = "ClientConfig::default_timeout")]
    timeout: Time,
    #[serde(default = "ClientConfig::default_max_idle_per_host")]
    #[serde(alias = "max-idle-per-host")]
    max_idle_per_host: usize,
}

impl ClientConfig {
    #[inline]
    const fn default_max_retries() -> u8 {
        5
    }

    #[inline]
    const fn default_timeout() -> Time {
        Time(Duration::from_secs(30))
    }

    #[inline]
    const fn default_max_idle_per_host() -> usize {
        usize::MAX
    }

    pub fn max_retries(&self) -> u8 {
        self.max_retries
    }
    pub fn retry_interval(&self) -> Duration {
        self.retry_interval.0
    }
    pub fn timeout(&self) -> Duration {
        self.timeout.0
    }
    pub fn max_idle_per_host(&self) -> usize {
        self.max_idle_per_host
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            max_retries: Self::default_max_retries(),
            retry_interval: Self::default_timeout(),
            timeout: Self::default_timeout(),
            max_idle_per_host: Self::default_max_idle_per_host(),
        }
    }
}

#[derive(Debug, Eq, Ord, PartialOrd, PartialEq, Deserialize)]
pub struct HttpConfig {
    client: ClientConfig,
}

impl HttpConfig {
    pub fn client(&self) -> &ClientConfig {
        &self.client
    }
}

impl Deserializable for HttpConfig {
    async fn deserialize(text: &str) -> Result<Self> {
        Ok(toml::de::from_str(text)?)
    }
}

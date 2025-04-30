use crate::config::api_fields::{Account, ApiFields, Auth, Zone};
use crate::config::http::HttpConfig;
use crate::config::ip_source::{IpSource, Sources};
use crate::config::misc::MiscConfig;
use crate::retrying_client::{RequestBuilder, AUTHORIZATION_EMAIL, AUTHORIZATION_KEY};
use reqwest::header::AUTHORIZATION;
use std::num::NonZeroU8;
use std::path::Path;
use std::sync::Arc;

pub mod api_fields;
mod http;
pub mod ip_source;
pub mod listener;
mod misc;
mod time;

trait Deserializable: Sized {
    async fn deserialize(text: &str) -> anyhow::Result<Self>;
}

async fn deserialize_from_file<T: Deserializable>(path: impl AsRef<Path>) -> anyhow::Result<T> {
    T::deserialize(&tokio::fs::read_to_string(path).await?).await
}

#[derive(Debug, Eq, Ord, PartialOrd, PartialEq, Clone)]
pub(crate) struct CfgInner {
    api_fields: Arc<ApiFields>,
    http: Arc<HttpConfig>,
    misc: Arc<MiscConfig>,
    ip_sources: Arc<Sources>,
}

impl CfgInner {
    pub(crate) fn new(
        api_fields: ApiFields,
        http: HttpConfig,
        misc: MiscConfig,
        ip_sources: Sources,
    ) -> Self {
        Self {
            api_fields: api_fields.into(),
            http: http.into(),
            misc: misc.into(),
            ip_sources: ip_sources.into(),
        }
    }
}

/// Cheaply cloneable to read-only config
#[derive(Debug, Clone)]
pub struct Config(Arc<CfgInner>);

impl Config {
    pub fn ip_sources(&self) -> impl Iterator<Item = IpSource> + '_ {
        self.0.ip_sources.sources()
    }

    pub fn http(&self) -> &HttpConfig {
        &self.0.http
    }

    pub fn misc(&self) -> &MiscConfig {
        &self.0.misc
    }

    pub fn zone(&self) -> &Zone {
        &self.0.api_fields.zone
    }

    pub fn account(&self) -> &Account {
        &self.0.api_fields.account
    }

    pub fn concurrent_resolve(&self) -> NonZeroU8 {
        self.0.ip_sources.concurrent_resolve
    }

    pub fn authorize_request(&self, request: RequestBuilder) -> RequestBuilder {
        let request = request.header(AUTHORIZATION_EMAIL, self.account().email.clone());

        match &self.account().auth {
            Auth::Token(token_header) => request.header(AUTHORIZATION, token_header.clone()),
            Auth::Key(key_header) => request.header(AUTHORIZATION_KEY, key_header.clone()),
        }
    }
}

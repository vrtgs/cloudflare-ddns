use crate::addr_helper::IpType;
use crate::config::Deserializable;
use anyhow::Result;
use reqwest::header::HeaderValue;
use serde::de::Error;
use serde::{Deserialize, Deserializer};

#[derive(Eq, Ord, PartialOrd, PartialEq, Debug)]
pub(super) enum Auth {
    Token(HeaderValue),
    Key(HeaderValue),
}

#[derive(Eq, Ord, PartialOrd, PartialEq, Debug)]
pub struct Account {
    pub(super) email: HeaderValue,
    pub(super) auth: Auth,
}

macro_rules! invalid_header {
    ($field:literal) => {
        Error::custom(concat!($field, " can't be parsed as a valid http header"))
    };
}

impl<'de> Deserialize<'de> for Account {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct AccountInner {
            email: Box<str>,
            #[serde(alias = "api-token")]
            auth_token: Option<Box<str>>,
            #[serde(alias = "auth-key")]
            auth_key: Option<Box<str>>,
        }
        let inner = AccountInner::deserialize(deserializer)?;

        let email = HeaderValue::from_str(&inner.email).map_err(|_| invalid_header!("email"))?;

        let auth = match (inner.auth_token, inner.auth_key) {
            (Some(token), None) => Auth::Token(
                HeaderValue::from_str(&("Bearer ".to_owned() + &token))
                    .map_err(|_| invalid_header!("auth-token"))?,
            ),
            (None, Some(key)) => {
                Auth::Key(HeaderValue::from_str(&key).map_err(|_| invalid_header!("auth-key"))?)
            }
            (None, None) => return Err(Error::missing_field("auth-token")),
            (Some(_), Some(_)) => return Err(Error::custom("auth-token and auth-key conflict")),
        };

        Ok(Account { auth, email })
    }
}

#[derive(Eq, Ord, PartialOrd, PartialEq, Debug)]
pub struct Zone {
    id: Box<str>,
    record: Box<str>,
    proxied: bool,
    ip_type: IpType,
}

impl<'de> Deserialize<'de> for Zone {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ZoneInner {
            id: Box<str>,
            record: String,

            #[serde(default)]
            proxied: bool,

            #[serde(default)]
            ip: IpType,
        }

        let ZoneInner {
            id,
            record,
            proxied,
            ip,
        } = ZoneInner::deserialize(deserializer)?;

        let record = idna::domain_to_ascii(&record)
            .map_err(|_| Error::custom("Invalid UTS #46 domain"))?
            .into_boxed_str();

        Ok(Zone {
            id,
            record,
            proxied,
            ip_type: ip,
        })
    }
}
impl Zone {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn record(&self) -> &str {
        &self.record
    }

    pub fn proxied(&self) -> bool {
        self.proxied
    }

    pub fn ip_type(&self) -> IpType {
        self.ip_type
    }
}

#[derive(Eq, Ord, PartialOrd, PartialEq, Deserialize, Debug)]
pub struct ApiFields {
    pub(crate) account: Account,
    pub(crate) zone: Zone,
}

impl Deserializable for ApiFields {
    async fn deserialize(text: &str) -> Result<Self> {
        Ok(toml::de::from_str(text)?)
    }
}

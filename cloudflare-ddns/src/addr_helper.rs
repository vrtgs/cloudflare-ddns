use serde::de::Error;
use serde::{Deserialize, Deserializer};
use std::net;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AddrParseError {
    #[error("The input data was too long to even be considered an address")]
    TooLong,
    #[error("invalid encoding on the addresses bytes")]
    InvalidEncoding,
    #[error(transparent)]
    Parse(#[from] net::AddrParseError),
}

pub trait AddrParseExt: Sized {
    fn parse_ascii_bytes(b: &[u8]) -> Result<Self, AddrParseError>;
}

macro_rules! test_gen {
    (Ipv4Addr => $item:item) => {
        #[cfg(test)]
        mod ip_v4_max_test {
            use super::*;
            #[test]
            $item
        }
    };
    (Ipv6Addr => $item:item) => {
        #[cfg(test)]
        mod ip_v6_max_test {
            use super::*;
            #[test]
            $item
        }
    };
}

macro_rules! impl_addr {
    ($ty: ident, max: $biggest_addr:literal) => {
        impl AddrParseExt for $ty {
            fn parse_ascii_bytes(b: &[u8]) -> Result<Self, AddrParseError> {
                let b = b.trim_ascii();

                if b.len() > ($biggest_addr).len() {
                    return Err(AddrParseError::TooLong);
                }

                b.is_ascii()
                    .then(|| unsafe { std::str::from_utf8_unchecked(b) })
                    .ok_or(AddrParseError::InvalidEncoding)
                    .and_then(|s| <$ty>::from_str(s).map_err(Into::into))
            }
        }

        test_gen! {
            $ty => fn test_ip_max() {
                assert_eq!(
                    ($biggest_addr).len(),
                    <$ty>::from(std::array::from_fn(|_| u8::MAX)).to_string().len()
                )
            }
        }
    };
}

impl_addr! {
    Ipv4Addr,
    max: b"xxx.xxx.xxx.xxx"
}

impl_addr! {
    Ipv6Addr,
    max: b"xxxx:xxxx:xxxx:xxxx:xxxx:xxxx:xxxx:xxxx"
}

impl AddrParseExt for IpAddr {
    fn parse_ascii_bytes(bytes: &[u8]) -> Result<Self, AddrParseError> {
        Ipv4Addr::parse_ascii_bytes(bytes)
            .map(IpAddr::V4)
            .or_else(|_| Ipv6Addr::parse_ascii_bytes(bytes).map(IpAddr::V6))
    }
}

#[derive(Debug, Error)]
#[error("unknown ip type {0}")]
pub struct IpTypeError(String);

#[derive(Ord, PartialOrd, Eq, PartialEq, Copy, Clone, Debug, Default)]
pub enum IpType {
    Any,
    Both,
    V6,
    #[default]
    V4,
}

impl<'de> Deserialize<'de> for IpType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).and_then(|s| Self::from_str(&s).map_err(Error::custom))
    }
}

impl FromStr for IpType {
    type Err = IpTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &*s.to_lowercase() {
            "any" => Ok(Self::Any),
            "both" => Ok(Self::Both),
            "v4" | "ipv4" => Ok(Self::V4),
            "v6" | "ipv6" => Ok(Self::V6),
            _ => Err(IpTypeError(s.to_string())),
        }
    }
}

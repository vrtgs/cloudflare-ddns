use crate::abort_unreachable;
use crate::config::Config;
use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Body, Client, ClientBuilder, IntoUrl, Method, Request, Response};
use std::time::Duration;

macro_rules! from_static {
    ($($vis: vis const $name: ident: $ty: ty = $val: expr;)*) => {$(
        #[allow(clippy::declare_interior_mutable_const)]
        $vis const $name: $ty = <$ty>::from_static($val);
    )*};
}

from_static! {
    pub const AUTHORIZATION_EMAIL: HeaderName = "x-auth-email";
    pub const AUTHORIZATION_KEY: HeaderName = "x-auth-key";

    const JSON_MIME: HeaderValue  = "application/json";
}

pub struct RequestBuilder {
    client: RetryingClient,
    req: reqwest::Result<Request>,
}

impl RequestBuilder {
    pub fn header(mut self, key: HeaderName, value: HeaderValue) -> RequestBuilder {
        if let Ok(ref mut req) = self.req {
            req.headers_mut().append(key, value);
        }
        self
    }

    pub fn body(mut self, body: impl Into<Body>) -> RequestBuilder {
        if let Ok(ref mut req) = self.req {
            *req.body_mut() = Some(body.into());
        }
        self
    }

    pub fn json(self, body: impl Into<Body>) -> RequestBuilder {
        self.header(CONTENT_TYPE, JSON_MIME).body(body)
    }

    pub async fn send(self) -> reqwest::Result<Response> {
        match self.req {
            Ok(req) => self.client.execute(req).await,
            Err(e) => Err(e),
        }
    }
}

#[derive(Clone)]
pub struct RetryingClient {
    client: Client,
    max_retries: u8,
    retry_interval: Duration,
}

impl RetryingClient {
    pub fn new(cfg: &Config) -> Self {
        let _cfg = cfg;
        macro_rules! get {
            ($id: ident) => {
                _cfg.http().client().$id()
            };
        }

        let max_retries = get!(max_retries);
        let retry_interval = get!(retry_interval);

        let builder = ClientBuilder::new()
            .timeout(get!(timeout))
            .pool_idle_timeout(get!(timeout).checked_mul(max_retries as u32 + 1))
            .pool_max_idle_per_host(get!(max_idle_per_host))
            .use_rustls_tls();

        #[cfg(feature = "trace")]
        let builder = builder
            .pool_idle_timeout(Duration::ZERO)
            .pool_max_idle_per_host(0);

        builder
            .build()
            .map(|client| RetryingClient {
                client,
                max_retries,
                retry_interval,
            })
            .unwrap_or_else(|e| abort_unreachable!("ClientBuilder failed {e}"))
    }

    /// See [`Client::get`]
    pub fn get<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::GET, url)
    }

    /// See [`Client::patch`]
    pub fn patch<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::PATCH, url)
    }

    /// See [`Client::request`]
    pub fn request<U: IntoUrl>(&self, method: Method, url: U) -> RequestBuilder {
        RequestBuilder {
            client: self.clone(),
            req: url.into_url().map(|url| Request::new(method, url)),
        }
    }

    /// See [`Client::execute`]
    pub async fn execute(&self, request: Request) -> reqwest::Result<Response> {
        for i in 0..self.max_retries.saturating_sub(1) {
            if let Some(req) = request.try_clone() {
                match self.client.execute(req).await {
                    Ok(resp) => return Ok(resp),
                    Err(_) => {
                        let sleep_for = self
                            .retry_interval
                            .checked_mul((i / 2).max(1) as u32)
                            .unwrap_or(Duration::MAX);

                        tokio::time::sleep(sleep_for).await
                    }
                }
            } else {
                abort_unreachable!("tried to use a streaming request");
            }
        }

        self.client.execute(request).await
    }
}

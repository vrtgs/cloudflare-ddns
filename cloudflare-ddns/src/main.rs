#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

extern crate core;

use crate::addr_helper::IpType;
use crate::config::Config;
use crate::config::ip_source::GetIpError;
use crate::json::EscapeExt;
use crate::network_listener::has_internet;
use crate::one_or_more::OneOrMore;
use crate::retrying_client::RetryingClient;
use crate::time::new_skip_interval;
use crate::updaters::{UpdaterEvent, UpdaterExitStatus};
use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::borrow::Cow;
use std::cell::Cell;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::num::NonZeroU8;
use std::panic::AssertUnwindSafe;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::thread::Builder;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::{join, try_join};

mod addr_helper;
mod config;
mod console_listener;
mod err;
mod global_rt;
mod json;
mod network_listener;
mod num_cpus;
mod one_or_more;
mod pre;
mod retrying_client;
mod time;
mod updaters;

struct DDNSContext {
    client: RetryingClient,
    user_messages: UserMessages,
}

#[derive(Debug)]
struct Record<T> {
    id: Box<str>,
    ip: T,
}

type RecordV4 = Record<Ipv4Addr>;
type RecordV6 = Record<Ipv6Addr>;

impl DDNSContext {
    fn new(cfg: Config) -> Self {
        DDNSContext {
            client: RetryingClient::new(&cfg),
            user_messages: UserMessages::new(cfg.misc().general().max_errors()),
        }
    }

    async fn get_ips(&self, cfg: &Config) -> Result<(Option<Ipv4Addr>, Option<Ipv6Addr>)> {
        let last_err = Cell::new(None);

        let iter = cfg.ip_sources().map(|x| x.resolve_ip(&self.client, cfg));
        let mut stream = futures::stream::iter(iter)
            .buffer_unordered(cfg.concurrent_resolve().get() as usize)
            .filter_map(|x| {
                std::future::ready({
                    match x {
                        Ok(x) => Some(x),
                        Err(err) => {
                            last_err.set(Some(err));
                            None
                        }
                    }
                })
            });

        let ip_ty = cfg.zone().ip_type();

        let mut ipv4 = None;
        let mut ipv6 = None;

        while let Some(addr) = stream.next().await {
            match addr {
                IpAddr::V4(addr) if ipv4.is_none() => ipv4 = Some(addr),
                IpAddr::V6(addr) if ipv6.is_none() => ipv6 = Some(addr),
                // take only the first IP
                // filter the rest
                IpAddr::V4(_) | IpAddr::V6(_) => {}
            }

            let exit_cond = match ip_ty {
                IpType::Any => ipv4.is_some() || ipv6.is_some(),
                IpType::Both => ipv4.is_some() && ipv6.is_some(),
                IpType::V6 => ipv4.is_some(),
                IpType::V4 => ipv6.is_some(),
            };

            if exit_cond {
                break;
            }
        }

        Ok((ipv4, ipv6))
    }

    async fn get_records(&self, cfg: &Config) -> Result<(Option<RecordV4>, Option<RecordV6>)> {
        #[derive(Debug, Deserialize)]
        struct FullIpTypeRecord<T> {
            id: Box<str>,
            name: Box<str>,
            #[serde(rename = "content")]
            ip: T,
        }

        #[derive(Debug, Deserialize)]
        pub struct GetResponse<T> {
            result: OneOrMore<FullIpTypeRecord<T>>,
        }

        async fn get_bytes(this: &DDNSContext, cfg: &Config, record_type: &str) -> Result<Bytes> {
            let url = format!(
                "https://api.cloudflare.com/client/v4/zones/{zone_id}/dns_records?type={record_type}&name={record}",
                zone_id = cfg.zone().id(),
                record = cfg.zone().record()
            );

            cfg.authorize_request(this.client.get(url))
                .send()
                .await?
                .bytes()
                .await
                .map_err(anyhow::Error::new)
        }

        trait IpVersion: DeserializeOwned {
            const RECORD: &str;
        }

        impl IpVersion for Ipv4Addr {
            const RECORD: &str = "A";
        }

        impl IpVersion for Ipv6Addr {
            const RECORD: &str = "AAAA";
        }

        async fn get_record_typed<T: IpVersion>(
            this: &DDNSContext,
            cfg: &Config,
        ) -> Result<Record<T>> {
            let response = get_bytes(this, cfg, T::RECORD).await?;
            let FullIpTypeRecord { name, id, ip } =
                match serde_json::from_slice::<GetResponse<T>>(&response)?.result {
                    OneOrMore::Zero => {
                        bail!("found zero corresponding records {}", cfg.zone().record())
                    }
                    OneOrMore::More => bail!(
                        "found too many corresponding `{TYPE}` records for {}",
                        cfg.zone().record(),
                        TYPE = T::RECORD,
                    ),
                    OneOrMore::One(record) => record,
                };

            anyhow::ensure!(
                &*name == cfg.zone().record(),
                "Expected {} found {name}",
                cfg.zone().record()
            );

            Ok(Record { id, ip })
        }

        Ok(match cfg.zone().ip_type() {
            IpType::Any => {
                let (v4, v6) = join!(
                    get_record_typed::<Ipv4Addr>(self, cfg),
                    get_record_typed::<Ipv6Addr>(self, cfg)
                );
                match (v4, v6) {
                    (Err(v4_err), Err(v6_err)) => bail!(v4_err.context(v6_err)),
                    (v4, v6) => (v4.ok(), v6.ok()),
                }
            }
            IpType::Both => {
                let (v4, v6) = try_join!(
                    get_record_typed::<Ipv4Addr>(self, cfg),
                    get_record_typed::<Ipv6Addr>(self, cfg)
                )?;
                (Some(v4), Some(v6))
            }
            IpType::V6 => (None, Some(get_record_typed::<Ipv6Addr>(self, cfg).await?)),
            IpType::V4 => (Some(get_record_typed::<Ipv4Addr>(self, cfg).await?), None),
        })
    }

    async fn update_record(&self, id: &str, ip: IpAddr, cfg: &Config) -> Result<()> {
        let request_json = format! {
            r###"{{"type":"{record_type}","name":"{record}","content":"{ip}","proxied":{proxied}}}"###,
            record_type = match ip {
                IpAddr::V4(_) => "A",
                IpAddr::V6(_) => "AAAA"
            },
            record = cfg.zone().record().escape_json(),
            proxied = cfg.zone().proxied()
        };

        let url = format! {
            "https://api.cloudflare.com/client/v4/zones/{zone_id}/dns_records/{record_id}",
            zone_id = cfg.zone().id(),
            record_id = id
        };

        let response = cfg
            .authorize_request(self.client.patch(url))
            .json(request_json)
            .send()
            .await?;

        let failure = !response.status().is_success();

        let bytes = response
            .bytes()
            .await
            .with_context(|| "unable to retrieve bytes")?;

        #[derive(Debug, Deserialize)]
        pub struct PatchResponse {
            success: bool,
        }

        let response = serde_json::from_slice::<PatchResponse>(&bytes)
            .with_context(|| "unable to deserialize patch response json")?;

        if failure || !response.success {
            bail!("Bad response: {}", String::from_utf8_lossy(&bytes))
        }

        Ok(())
    }

    pub async fn run_ddns(&self, cfg: Config) -> Result<bool> {
        let ((v4_record, v6_record), (v4_ip, v6_ip)) =
            try_join!(self.get_records(&cfg), self.get_ips(&cfg))?;

        let has_ip_source = match (&v4_record, &v6_record) {
            (Some(_), Some(_)) => v4_ip.is_some() || v6_ip.is_some(),
            (Some(_), None) => v4_ip.is_some(),
            (None, Some(_)) => v6_ip.is_some(),
            (None, None) => {
                unreachable!("this case should have errored before reaching this point")
            }
        };

        ensure!(has_ip_source, GetIpError::NoIpSources);

        async fn update<T: Into<IpAddr> + Eq>(
            this: &DDNSContext,
            cfg: &Config,
            record: Option<Record<T>>,
            ip: Option<T>,
        ) -> Result<bool> {
            if let Some(record) = record
                && let Some(ip) = ip
            {
                if record.ip == ip {
                    return Ok(false);
                }

                this.update_record(&record.id, ip.into(), cfg).await?
            }

            Ok(false)
        }

        let (changed_v4, changed_v6) = try_join!(
            update(self, &cfg, v4_record, v4_ip),
            update(self, &cfg, v6_record, v6_ip),
        )?;

        Ok(changed_v4 || changed_v6)
    }
}

#[derive(Clone)]
struct UserMessages {
    errors: Arc<Semaphore>,
    warning: Arc<Semaphore>,
}

impl UserMessages {
    fn new(max_errors: NonZeroU8) -> Self {
        let permits = max_errors.get() as usize;
        UserMessages {
            errors: Arc::new(Semaphore::new(permits)),
            warning: Arc::new(Semaphore::new(permits)),
        }
    }

    async fn custom_error(&self, fun: impl FnOnce() + Send + 'static) {
        err::spawn_message_box(Arc::clone(&self.errors), fun).await
    }

    async fn custom_warning(&self, fun: impl FnOnce() + Send + 'static) {
        err::spawn_message_box(Arc::clone(&self.warning), fun).await
    }

    async fn error(&self, msg: impl Into<Cow<'static, str>>) {
        let msg = msg.into();
        self.custom_error(move || err::error(&msg)).await
    }

    async fn warning(&self, msg: impl Into<Cow<'static, str>>) {
        let msg = msg.into();
        self.custom_warning(move || err::warn(&msg)).await
    }
}

enum Action {
    Restart,
    Exit(u8),
}

async fn real_main() -> Result<Action> {
    let (ctx, mut updaters_manager, cfg_store) = config::listener::load().await?;
    let network_detection = cfg_store.load_config().misc().refresh().network_detection();

    if network_detection {
        network_listener::subscribe(&mut updaters_manager)?;
    }
    err::exit::subscribe(&mut updaters_manager)?;
    console_listener::subscribe(&mut updaters_manager)?;

    let mut interval = new_skip_interval(cfg_store.load_config().misc().refresh().interval());

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !has_internet().await {
                    dbg_println!("no internet available skipping update");
                    continue;
                }

                dbg_println!("updating");
                match ctx.run_ddns(cfg_store.load_config()).await {
                    Err(err) => ctx.user_messages.error(err.to_string()).await,
                    Ok(true) => dbg_println!("successfully updated"),
                    Ok(false) => dbg_println!("IP didn't change skipping record update"),
                }
            },
            res = updaters_manager.watch() => match res {
                UpdaterEvent::Update => interval.reset_immediately(),
                UpdaterEvent::ServiceEvent(exit) => {
                    match *exit.status() {
                        UpdaterExitStatus::Success => {},
                        UpdaterExitStatus::Panic | UpdaterExitStatus::Error(_) => {
                            ctx.user_messages.error(format!("Updater abruptly exited: {exit}")).await
                        }
                        UpdaterExitStatus::TriggerExit(code) => {
                            updaters_manager.shutdown().await;
                            return Ok(Action::Exit(code));
                        },
                        UpdaterExitStatus::TriggerRestart => return Ok(Action::Restart),
                    }
                }
            }
        }
    }
}

#[cfg(feature = "trace")]
fn make_runtime() -> tokio::runtime::Handle {
    (*util::GLOBAL_TOKIO_RUNTIME).clone()
}

#[cfg(not(feature = "trace"))]
fn make_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(num_cpus::num_cpus().get())
        .enable_all()
        .build()
        .expect("failed to build runtime")
}

fn main() -> ExitCode {
    pre::pre_run();
    #[cfg(feature = "trace")]
    console_subscriber::init();

    let mut runtime = make_runtime();
    loop {
        let exit = std::panic::catch_unwind(AssertUnwindSafe(|| runtime.block_on(real_main())));

        match exit {
            // Non-Recoverable
            Ok(Ok(Action::Exit(exit))) => {
                dbg_println!("Shutting down the runtime...");
                drop(runtime);
                dbg_println!("Exiting...");
                return ExitCode::from(exit);
            }
            Ok(Err(e)) => {
                dbg_println!("Fatal init error");
                dbg_println!("Aborting...");
                // best effort clean up
                let _ = Builder::new().spawn(move || drop(runtime));
                abort!("{e}")
            }

            // Recoverable
            Ok(Ok(Action::Restart)) => dbg_println!("Restarting..."),
            Err(_) => {
                // old runtime might be in an invalid state
                // replace it and drop it on a new thread to avoid hanging
                let old_runtime = std::mem::replace(&mut runtime, make_runtime());
                thread::spawn(move || drop(old_runtime));

                dbg_println!("Panicked!!");
                dbg_println!("Retrying in 15s...");
                thread::sleep(Duration::from_secs(15));
                dbg_println!("Retrying")
            }
        }
    }
}

use crate::config::ip_source::Sources;
use crate::config::{deserialize_from_file, CfgInner, Config};
use crate::updaters::{Updater, UpdatersManager};
use crate::{non_zero, DDNSContext, UserMessages};
use anyhow::Result;
use anyhow::{anyhow, Context};
use arc_swap::ArcSwap;
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    new_debouncer_opt, DebounceEventHandler, DebounceEventResult, FileIdMap,
};
use std::io;
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::task::AbortHandle;

pub struct ConfigStorage {
    cfg: Arc<ArcSwap<CfgInner>>,
    update_task: AbortHandle,
}

impl Drop for ConfigStorage {
    fn drop(&mut self) {
        self.update_task.abort()
    }
}

impl ConfigStorage {
    #[inline]
    pub fn load_config(&self) -> Config {
        Config(self.cfg.load_full())
    }
}

struct FsEventHandler(tokio::sync::watch::Sender<DebounceEventResult>);

impl DebounceEventHandler for FsEventHandler {
    fn handle_event(&mut self, event: DebounceEventResult) {
        let _ = self.0.send(event);
    }
}

async fn listen(
    cfg: Weak<ArcSwap<CfgInner>>,
    updater: &Updater,
    msg_bx_handle: UserMessages,
) -> Result<bool> {
    let (tx, mut rx) = tokio::sync::watch::channel(Ok(vec![]));

    const POLL_INTERVAL: Duration = Duration::from_secs(30);

    let _watcher = tokio::task::spawn_blocking(move || {
        let mut watcher = new_debouncer_opt::<_, RecommendedWatcher, _>(
            POLL_INTERVAL,
            None,
            FsEventHandler(tx),
            FileIdMap::new(),
            notify::Config::default().with_compare_contents(true),
        )?;

        watcher.watch(
            Path::new("./config/sources.toml"),
            RecursiveMode::NonRecursive,
        )?;
        watcher.watch(Path::new("./config/api.toml"), RecursiveMode::NonRecursive)?;
        anyhow::Ok(watcher)
    })
    .await??;

    let shutdown = async {
        let cfg_dropped = async {
            loop {
                if Weak::upgrade(&cfg).is_none() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(10)).await
            }
        };

        tokio::select! {
            _ = updater.wait_shutdown() => (),
            _ = cfg_dropped => (),
        }
    };

    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            Ok(()) = rx.changed() => {
                let events = {
                    let borrow = rx.borrow_and_update();
                    borrow.as_ref().map_err(|e|{
                        e.iter().map(|e| format!("listen event error: {e}")).collect::<Vec<_>>()
                    }).map_err(|e| anyhow!("Error listening to config {e:?}")).cloned()
                };

                let events = match events {
                    Ok(events) => events,
                    Err(e) => {
                        msg_bx_handle.error(e.to_string()).await;
                        continue
                    },
                };

                macro_rules! change_occurred_in {
                    ($path:literal in $events:expr) => { $events.iter().any(|e| e.paths.iter().any(|p| p.ends_with($path))) };
                }

                macro_rules! lazy_reload_config {
                    ($path:literal; $part:ident; $restart:literal) => {
                        if change_occurred_in!($path in events) {
                            match deserialize_from_file(concat!("./config/", $path)).await {
                                Ok(part) => {
                                    #[allow(unreachable_code)]
                                    #[allow(unused)]
                                    #[allow(clippy::diverging_sub_expression)]
                                    if false {
                                        fn infer_part_type<T>(_: T, _: Arc<T>) -> ! {
                                            todo!()
                                        }

                                        let cfg: CfgInner = ::std::unreachable!();
                                        infer_part_type(part, cfg.$part)
                                    }
                                    let Some(cfg) = Weak::upgrade(&cfg) else { break };
                                    let old_cfg = cfg.load();
                                    if part == *old_cfg.$part { continue }

                                    let mut new_cfg = CfgInner::clone(&old_cfg);
                                    new_cfg.$part = Arc::new(part);
                                    cfg.store(Arc::new(new_cfg));
                                    if $restart { return Ok(true); }
                                    if updater.update().is_err() { break }
                                }
                                Err(e) => msg_bx_handle.warning(format!("config listen error: {e}")).await
                            }
                        }
                    };
                }

                lazy_reload_config!("api.toml"; api_fields; true);
                lazy_reload_config!("http.toml"; http; true);
                lazy_reload_config!("misc.toml";  misc; true);
                lazy_reload_config!("sources.toml"; ip_sources; false);
            }
            _ = &mut shutdown => break,
            else => break
        }
    }

    anyhow::Ok(false)
}

pub async fn load() -> Result<(DDNSContext, UpdatersManager, ConfigStorage)> {
    if !tokio::fs::try_exists("./config").await? {
        tokio::fs::create_dir_all("./config").await?;
    }
    if !tokio::fs::metadata("./config").await?.is_dir() {
        anyhow::bail!("./config is not a directory")
    }

    macro_rules! exists_or_include {
        ($($name: literal),*) => {
            tokio::try_join!($(async {
                if !tokio::fs::try_exists(concat!("./config/", $name, ".toml")).await? {
                    tokio::fs::write(concat!("./config/", $name, ".toml"), include_str!(concat!("../../includes/", $name, ".toml"))).await?;
                }
                Ok::<_, io::Error>(())
            }),*)
        };
    }

    exists_or_include!("api", "http", "misc", "sources")?;

    let ip_sources = match deserialize_from_file("./config/sources.toml").await {
        Ok(x) => x,
        Err(err) => {
            UserMessages::new(non_zero!(1))
                .warning(format!("{err}\n\n\n\n...Using default config..."))
                .await;
            Sources::default()
        }
    };

    macro_rules! load_config {
        ($($name:ident, $path:expr, $msg:expr $(;)+)*) => {
            $(let $name = deserialize_from_file($path)
                .await
                .context($msg)?;)*
        };
    }

    load_config!(
        http_config, "./config/http.toml", "Invalid Http config";
        services_config, "./config/misc.toml", "Invalid Services config";
        api_fields, "./config/api.toml", "Invalid API Fields config";
    );

    let cfg = Arc::new(CfgInner::new(
        api_fields,
        http_config,
        services_config,
        ip_sources,
    ));

    let cfg_store = Arc::new(ArcSwap::new(Arc::clone(&cfg)));
    let cfg_weak = Arc::downgrade(&cfg_store);

    let ctx = DDNSContext::new(Config(cfg));
    let user_messages = ctx.user_messages.clone();
    let mut updater_manager = UpdatersManager::new();

    let (updater, jh_entry) = updater_manager.add_updater("config-listener");
    let update_task = tokio::spawn(async move {
        let res = listen(cfg_weak, &updater, user_messages).await;
        match res {
            Ok(true) => updater.trigger_restart(),
            Ok(false) => updater.exit(anyhow::Ok(())),
            Err(err) => updater.exit(Err(err)),
        }
    });
    let storage = ConfigStorage {
        cfg: cfg_store,
        update_task: update_task.abort_handle(),
    };

    jh_entry.insert(update_task);

    Ok((ctx, updater_manager, storage))
}

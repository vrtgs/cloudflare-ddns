use crate::updaters::Updater;
use crate::{abort, global_rt};
use anyhow::Result;
use dbus::nonblock::{Proxy, SyncConnection};
use futures::{StreamExt, TryStreamExt};
use std::fs::OpenOptions;
use std::hint::cold_path;
use std::io::{Read, Write};
use std::num::NonZero;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use std::{io, thread};
use tempfile::TempPath;
use tokio::net::UnixListener;
use tokio::sync::OnceCell as TokioOnceCell;
use tokio::task::JoinHandle;

trait ArcExt<T> {
    fn leak(this: Self) -> &'static T;
}

impl<T> ArcExt<T> for Arc<T> {
    fn leak(this: Self) -> &'static T {
        // since we don't decrement this counter,
        // it will always be greater than 1, therefore, the allocation is valid
        unsafe { &*Arc::into_raw(this) }
    }
}

#[derive(Debug, thiserror::Error)]
enum DbusError {
    #[error(transparent)]
    Init(#[from] &'static dbus::Error),
    #[error(transparent)]
    Connection(#[from] dbus::Error),
}

async fn check_network_status() -> Result<bool, DbusError> {
    static DBUS_CONN: LazyLock<Result<&SyncConnection, dbus::Error>> = LazyLock::new(|| {
        let (resource, conn) = dbus_tokio::connection::new_system_sync()?;

        global_rt::spawn(resource);

        Ok(Arc::leak(conn))
    });

    // Get a proxy to the NetworkManager object
    let proxy = Proxy::new(
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        Duration::from_secs(10),
        DBUS_CONN.as_ref().copied()?,
    );

    // Call the Get method on the org.freedesktop.DBus.Properties interface
    let (connectivity,): (dbus::arg::Variant<u32>,) = proxy
        .method_call(
            "org.freedesktop.DBus.Properties",
            "Get",
            ("org.freedesktop.NetworkManager", "Connectivity"),
        )
        .await?;

    let connectivity = connectivity.0;

    // value can be:
    //
    // 0: Unknown
    // 1: None
    // 2: Portal
    // 3: Limited
    // 4: Full
    Ok(matches!(connectivity, 0 | 2 | 3 | 4))
}

pub async fn has_internet() -> bool {
    static SUPPORTS_NETWORK_MANAGER: TokioOnceCell<bool> = TokioOnceCell::const_new();

    let mut ret_val = None::<bool>;

    let supports_network_manager = *SUPPORTS_NETWORK_MANAGER
        .get_or_init(|| async {
            match check_network_status().await {
                Ok(val) => {
                    ret_val = Some(val);
                    true
                }
                Err(_) => false,
            }
        })
        .await;

    match supports_network_manager {
        true => {
            if let Some(val) = ret_val {
                cold_path();
                return val;
            }

            match check_network_status().await {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("Unexpected error checking internet {e} switching to fallback");
                    super::fallback_has_internet().await
                }
            }
        }
        false => super::fallback_has_internet().await,
    }
}

async fn place_dispatcher() -> Result<()> {
    const DISPATCHER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dispatcher-bin"));

    let locations = include!("./dispatcher-locations")
        .map(Path::new)
        .map(|loc| loc.join(include_str!("./dispatcher-name")));

    let futures = locations.map(|location| async move {
        tokio::task::spawn_blocking(move || {
            if let Some(parent) = location.parent() {
                let eq_contents = |loc: &Path| {
                    let file = io::BufReader::new(std::fs::File::open(loc)?);

                    enum CmpErr {
                        Io(io::Error),
                        Cmp,
                    }

                    let mut dispatcher_bytes = DISPATCHER.iter();
                    let res = file.bytes().try_fold((), |(), byte| {
                        byte.map_err(CmpErr::Io).and_then(|byte| {
                            match Some(byte) == dispatcher_bytes.next().copied() {
                                true => Ok(()),
                                false => Err(CmpErr::Cmp),
                            }
                        })
                    });

                    let eq = match res {
                        Ok(()) => true,
                        Err(CmpErr::Cmp) => false,
                        Err(CmpErr::Io(io)) => return Err(io),
                    };

                    Ok(eq && dispatcher_bytes.as_slice().is_empty())
                };

                let invalid = |loc: &Path| {
                    let exists = loc.try_exists()?;
                    let valid = exists && eq_contents(loc)?;
                    Ok::<_, io::Error>(!valid)
                };

                if invalid(&location)? && parent.try_exists()? {
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .mode(0o555)
                        .open(location)?
                        .write_all(DISPATCHER)?;
                }
            }
            Ok(())
        })
        .await?
    });

    let buffer = futures
        .len()
        .min(thread::available_parallelism().map_or(1, NonZero::get));

    futures::stream::iter(futures)
        .buffer_unordered(buffer)
        .try_collect()
        .await
}

async fn listen(updater: &Updater) -> Result<()> {
    place_dispatcher().await?;

    const SOCK: &str = include_str!("./socket-path");

    let sock = tokio::task::spawn_blocking(|| {
        let path = Path::new(SOCK);
        if !path.is_absolute() {
            abort!("unix socket path is not absolute")
        }

        if path.try_exists()? {
            std::fs::remove_file(path)?
        }

        TempPath::try_from_path(path)
    })
    .await??;

    let listener = UnixListener::bind(&sock)?;

    tokio::task::spawn_blocking(|| {
        std::fs::set_permissions(SOCK, std::fs::Permissions::from_mode(0o777))
    })
    .await??;

    loop {
        let (_stream, _peer) = listener.accept().await?;
        if updater.update().is_err() {
            return Ok(());
        }
    }
}

pub fn subscribe(updater: Updater) -> JoinHandle<()> {
    tokio::spawn(async move {
        let res = tokio::select! {
            res = listen(&updater) => res,
            _ = updater.wait_shutdown() => Ok(())
        };
        updater.exit(res)
    })
}

use crate::abort_unreachable;
use ahash::{HashMap, HashMapExt};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::hash_map::{Entry, VacantEntry};
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Weak};
use tokio::sync::Notify;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

pub enum UpdaterEvent {
    Update,
    ServiceEvent(UpdaterExit),
}

pub enum UpdaterExitStatus {
    Success,
    Panic,
    TriggerRestart,
    TriggerExit(u8),
    Error(anyhow::Error),
}

pub struct UpdaterExit {
    name: &'static str,
    status: UpdaterExitStatus,
}

impl UpdaterExit {
    /// returns the name of the exited service
    #[allow(dead_code)] // future flexibility
    pub fn name(&self) -> &str {
        self.name
    }

    pub fn status(&self) -> &UpdaterExitStatus {
        &self.status
    }
}

impl Display for UpdaterExitStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdaterExitStatus::Success => write!(f, "successfully exited"),
            UpdaterExitStatus::Panic => write!(f, "died unexpectedly"),
            UpdaterExitStatus::Error(e) => write!(f, "exited with the error: {e}"),
            UpdaterExitStatus::TriggerRestart => write!(f, "triggered a restart"),
            UpdaterExitStatus::TriggerExit(code) => {
                write!(f, "triggered an exit with code: {code}")
            }
        }
    }
}

impl Display for UpdaterExit {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "updater <{}> {}", self.name, self.status)
    }
}

pub struct UpdatersManager {
    rcv: UnboundedReceiver<UpdaterExit>,
    snd: UnboundedSender<UpdaterExit>,
    notifier: Arc<Notify>,
    active_services: HashMap<&'static str, JoinHandle<()>>,
    shutdown: tokio::sync::watch::Sender<()>,
}

impl UpdatersManager {
    #[inline(always)]
    pub fn new() -> Self {
        let (snd, rcv) = tokio::sync::mpsc::unbounded_channel();
        let (shutdown, _) = tokio::sync::watch::channel(());
        UpdatersManager {
            rcv,
            snd,
            notifier: Arc::new(Notify::new()),
            active_services: HashMap::new(),
            shutdown,
        }
    }

    /// watches for service changes
    pub async fn watch(&mut self) -> UpdaterEvent {
        tokio::select! {
            _ = self.notifier.notified() => UpdaterEvent::Update,
            state = self.rcv.recv() => {
                let Some(state) = state else {
                    abort_unreachable!("channel should never close we always hold at least one sender")
                };

                assert!(
                    self.active_services.remove(state.name).is_some(),
                    "the updater {name} didn't give a join handle", name = state.name
                );

                UpdaterEvent::ServiceEvent(state)
            }
        }
    }

    #[inline(always)]
    #[must_use = "updater will instantly exit and trigger an exit event on drop, and you must add your JoinHandle"]
    pub fn add_updater(&mut self, name: &'static str) -> (Updater, JhEntry<'_>) {
        let Entry::Vacant(entry) = self.active_services.entry(name) else {
            abort_unreachable!("updater must have a unique name")
        };

        let snd = self.snd.clone();

        let entry = JhEntry {
            entry: Some(entry),
            send_fail: &mut self.snd,
            name,
        };

        (
            Updater {
                name,
                notifier: Arc::downgrade(&self.notifier),
                snd: Some(snd),
                shutdown: self.shutdown.subscribe(),
            },
            entry,
        )
    }

    pub async fn shutdown(self) {
        async fn forward_panic(join_handle: JoinHandle<()>) {
            if let Err(e) = join_handle.await {
                if let Ok(panic) = e.try_into_panic() {
                    std::panic::resume_unwind(panic)
                }
            }
        }

        let _ = self.shutdown.send(());
        self.active_services
            .into_values()
            .map(forward_panic)
            .collect::<FuturesUnordered<_>>()
            .collect::<()>()
            .await;
    }
}

pub struct JhEntry<'a> {
    entry: Option<VacantEntry<'a, &'static str, JoinHandle<()>>>,
    send_fail: &'a mut UnboundedSender<UpdaterExit>,
    name: &'static str,
}

impl JhEntry<'_> {
    pub fn insert(mut self, jh: JoinHandle<()>) {
        self.entry.take().unwrap().insert(jh);
    }
}

impl Drop for JhEntry<'_> {
    fn drop(&mut self) {
        if self.entry.is_some() {
            // we do this and trigger the watch to panic on no join handle provided
            let _ = self.send_fail.send(UpdaterExit {
                name: self.name,
                status: UpdaterExitStatus::Success,
            });
        }
    }
}

pub struct Updater {
    name: &'static str,
    notifier: Weak<Notify>,
    snd: Option<UnboundedSender<UpdaterExit>>,
    shutdown: tokio::sync::watch::Receiver<()>,
}

#[derive(Debug, thiserror::Error)]
#[error("updater disconnected from update manager")]
pub struct UpdateError;

impl Updater {
    #[inline(always)]
    pub fn update(&self) -> Result<(), UpdateError> {
        Weak::upgrade(&self.notifier)
            .map(|notifier| notifier.notify_waiters())
            .ok_or(UpdateError)
    }

    pub async fn wait_shutdown(&self) {
        let _ = self.shutdown.clone().changed().await;
    }

    pub fn exit(mut self, err: Result<(), impl Into<anyhow::Error>>) {
        let status = match err {
            Ok(()) => UpdaterExitStatus::Success,
            Err(err) => UpdaterExitStatus::Error(err.into()),
        };

        let snd = self.snd.take().unwrap();
        let _ = snd.send(UpdaterExit {
            name: self.name,
            status,
        });
    }

    pub fn trigger_exit(mut self, code: u8) {
        let snd = self.snd.take().unwrap();
        let _ = snd.send(UpdaterExit {
            name: self.name,
            status: UpdaterExitStatus::TriggerExit(code),
        });
    }

    pub fn trigger_restart(mut self) {
        let snd = self.snd.take().unwrap();
        let _ = snd.send(UpdaterExit {
            name: self.name,
            status: UpdaterExitStatus::TriggerRestart,
        });
    }
}

impl Drop for Updater {
    fn drop(&mut self) {
        if let Some(snd) = self.snd.take() {
            let status = match std::thread::panicking() {
                false => UpdaterExitStatus::Success,
                true => UpdaterExitStatus::Panic,
            };

            let _ = snd.send(UpdaterExit {
                name: self.name,
                status,
            });
        }
    }
}

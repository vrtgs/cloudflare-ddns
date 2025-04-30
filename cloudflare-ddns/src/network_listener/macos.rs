#![cfg(target_os = "macos")]

use crate::updaters::Updater;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use system_configuration::network_reachability::{
    ReachabilityFlags, SCNetworkReachability, SchedulingError, SetCallbackError,
};
use tokio::task::JoinHandle;

#[derive(thiserror::Error, Debug)]
pub enum UpdaterError {
    #[error("Couldn't set the callback to network events: {0}")]
    Callback(#[from] SetCallbackError),

    #[error("Couldn't Schedule callback execution with CFRunloop: {0}")]
    Runloop(#[from] SchedulingError),
}

pub async fn has_internet() -> bool {
    let sc = SCNetworkReachability::from(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
    sc.reachability().map_or(false, has_internet_from_flags)
}

fn has_internet_from_flags(flags: ReachabilityFlags) -> bool {
    flags.contains(ReachabilityFlags::REACHABLE)
        && (!flags.contains(ReachabilityFlags::CONNECTION_REQUIRED)
            || ((flags.contains(ReachabilityFlags::CONNECTION_ON_DEMAND)
                || flags.contains(ReachabilityFlags::CONNECTION_ON_TRAFFIC))
                && !flags.contains(ReachabilityFlags::INTERVENTION_REQUIRED))
            || flags.contains(ReachabilityFlags::IS_WWAN))
}

pub fn subscribe(updater: Updater) -> JoinHandle<()> {
    tokio::task::spawn(async move {
        let res = super::fallback_listen(&updater).await;
        updater.exit(res)
    })
}

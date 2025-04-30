use crate::updaters::UpdatersManager;
use std::convert::Infallible;

macro_rules! wait_for_any {
    ($($fut: expr),* $(,)?) => {
        tokio::select! {
            $(Some(()) = $fut => (),)*
            else => ::std::future::pending().await
        }
    };
}

#[cfg(windows)]
mod sys {
    use tokio::signal::windows as signal;

    pub(super) async fn recv_exit() {
        let mut ctrl_c = signal::ctrl_c().unwrap();
        let mut r#break = signal::ctrl_break().unwrap();
        let mut close = signal::ctrl_close().unwrap();
        let mut shutdown = signal::ctrl_shutdown().unwrap();
        wait_for_any!(ctrl_c.recv(), r#break.recv(), close.recv(), shutdown.recv())
    }
}

#[cfg(unix)]
mod sys {
    use tokio::signal::unix as signal;

    pub(super) async fn recv_exit() {
        let mut terminate = signal::signal(signal::SignalKind::terminate()).unwrap();
        let mut quit = signal::signal(signal::SignalKind::quit()).unwrap();
        let mut hangup = signal::signal(signal::SignalKind::hangup()).unwrap();
        let mut interrupt = signal::signal(signal::SignalKind::interrupt()).unwrap();
        wait_for_any!(
            terminate.recv(),
            quit.recv(),
            hangup.recv(),
            interrupt.recv()
        )
    }
}

pub fn subscribe(updaters_manager: &mut UpdatersManager) -> Result<(), Infallible> {
    let (updater, jh_entry) = updaters_manager.add_updater("shutdown-listener");
    jh_entry.insert(tokio::spawn(async {
        tokio::select! {
            _ = sys::recv_exit() => updater.trigger_exit(0),
            _ = updater.wait_shutdown() => {}
        }
    }));

    Ok(())
}

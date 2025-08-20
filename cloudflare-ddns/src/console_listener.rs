#[cfg(debug_assertions)]
mod r#impl {
    use crate::updaters::{Updater, UpdatersManager};
    use std::convert::Infallible;
    use std::sync::LazyLock;
    use std::{io, thread};
    use tokio::sync::Mutex;
    use tokio::sync::mpsc::Receiver;

    enum Status {
        Success,
        TriggerExit,
        TriggerRestart,
    }

    async fn listen(updater: &Updater) -> io::Result<Status> {
        // stdin is globally shared, so this also needs to be globally shared.
        // it won't end too well if we restart only to have to thread trying to read from stdin,
        // and we use a tokio mutex as we hold the receiver across a recv await point.
        static LINES: LazyLock<Mutex<Receiver<io::Result<String>>>> = LazyLock::new(|| {
            let (tx, rx) = tokio::sync::mpsc::channel(2);

            thread::spawn(move || {
                for line in io::stdin().lines() {
                    if tx.blocking_send(line).is_err() {
                        return;
                    }
                }
            });

            Mutex::new(rx)
        });

        while let Some(mut line) = LINES.lock().await.recv().await.transpose()? {
            line.make_ascii_lowercase();
            match line.trim() {
                "update" | "resolve" => {
                    if updater.update().is_err() {
                        return Ok(Status::Success);
                    }
                }
                "exit" => return Ok(Status::TriggerExit),
                "restart" => return Ok(Status::TriggerRestart),
                _ => continue,
            }
        }

        Ok(Status::Success)
    }

    pub fn subscribe(updaters_manager: &mut UpdatersManager) -> Result<(), Infallible> {
        let (updater, jh_entry) = updaters_manager.add_updater("console-listener");
        jh_entry.insert(tokio::spawn(async move {
            let res = tokio::select! {
                _ = updater.wait_shutdown() => Ok(Status::Success),
                res = listen(&updater) => res,
            };

            match res {
                Ok(Status::Success) => updater.exit(Ok::<(), Infallible>(())),
                Ok(Status::TriggerRestart) => updater.trigger_restart(),
                Ok(Status::TriggerExit) => updater.trigger_exit(0),
                Err(e) => updater.exit(Err(e)),
            }
        }));

        Ok(())
    }
}

#[cfg(not(debug_assertions))]
mod r#impl {
    use crate::updaters::UpdatersManager;
    use std::convert::Infallible;

    #[inline]
    pub fn subscribe(_: &mut UpdatersManager) -> Result<(), Infallible> {
        Ok(())
    }
}

pub use r#impl::*;

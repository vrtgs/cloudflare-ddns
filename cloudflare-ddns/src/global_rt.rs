use std::convert::Infallible;
use std::future::Future;
use std::sync::LazyLock;
use std::thread;
use tokio::runtime::Handle as TokioHandle;
use tokio::task::JoinHandle;

#[allow(dead_code)]
pub static GLOBAL_TOKIO_RUNTIME: LazyLock<TokioHandle> = LazyLock::new(|| {
    macro_rules! rt_abort {
        () => {{ |e| crate::abort!("failed to initialize the global tokio runtime due to {e}") }};
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(rt_abort!());

    let handle = runtime.handle().clone();
    thread::Builder::new()
        .spawn(move || runtime.block_on(std::future::pending::<Infallible>()))
        .unwrap_or_else(rt_abort!());

    handle
});

#[allow(dead_code)]
pub fn spawn<F>(f: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send,
{
    GLOBAL_TOKIO_RUNTIME.spawn(f)
}

use std::future::Future;
use std::time::Duration;
use futures::StreamExt;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
pub use wasm::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    pub use tokio::task::JoinHandle;
    pub use tokio::time::sleep;
    use tokio_stream::wrappers::IntervalStream;

    pub fn spawn<F>(future: F) -> JoinHandle<()>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        tokio::spawn(future)
    }

    pub fn interval(period: Duration) -> impl futures::Stream<Item = ()> {
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        IntervalStream::new(interval).map(|_| ())
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::*;
    use gloo_timers::future::{IntervalStream, TimeoutFuture};

    pub struct JoinHandle<T>(std::marker::PhantomData<T>);

    pub fn spawn<F>(future: F) -> JoinHandle<()>
    where
        F: Future<Output = ()> + 'static,
    {
        wasm_bindgen_futures::spawn_local(future);
        JoinHandle(std::marker::PhantomData)
    }

    pub async fn sleep(duration: Duration) {
        TimeoutFuture::new(duration.as_millis() as u32).await
    }

    pub fn interval(period: Duration) -> impl futures::Stream<Item = ()> {
        IntervalStream::new(period.as_millis() as u32).map(|_| ())
    }
}

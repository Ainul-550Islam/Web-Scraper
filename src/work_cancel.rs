//! Cancellation scope for synchronous, owned blocking jobs.
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
thread_local! { static STOP: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) }; }
pub(crate) struct Scope(Option<Arc<AtomicBool>>);
pub(crate) fn begin(stop: Arc<AtomicBool>) -> Scope {
    Scope(STOP.with(|s| s.replace(Some(stop))))
}
impl Drop for Scope {
    fn drop(&mut self) {
        STOP.with(|s| {
            s.replace(self.0.take());
        });
    }
}
pub(crate) fn check() -> std::io::Result<()> {
    if STOP.with(|s| {
        s.borrow()
            .as_ref()
            .is_some_and(|stop| stop.load(Ordering::Relaxed))
    }) {
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "job cancelled",
        ))
    } else {
        Ok(())
    }
}

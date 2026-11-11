//! Panic isolation shared by background workers.

use std::{
    any::Any,
    panic::{self, AssertUnwindSafe},
    thread,
};

/// Name prefix of every red-table background thread.
pub(crate) const THREAD_PREFIX: &str = "red-table-";

/// Runs one worker request and reports a panic as that request's error.
pub(crate) fn catch_panic<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    panic::catch_unwind(AssertUnwindSafe(work))
        .unwrap_or_else(|payload| Err(format!("image processing failed: {}", message(&*payload))))
}

/// Whether the current thread is a red-table background worker whose panics
/// are reported through request results instead of the terminal.
pub(crate) fn is_worker_thread() -> bool {
    thread::current()
        .name()
        .is_some_and(|name| name.starts_with(THREAD_PREFIX))
}

fn message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unexpected panic")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panics_become_request_errors_and_results_pass_through() {
        assert_eq!(catch_panic(|| Ok::<_, String>(7)), Ok(7));
        assert_eq!(
            catch_panic(|| Err::<(), _>("bad".to_string())),
            Err("bad".into())
        );
        let error = catch_panic::<()>(|| panic!("decoder index {}", 3)).unwrap_err();
        assert_eq!(error, "image processing failed: decoder index 3");
        let error = catch_panic::<()>(|| std::panic::panic_any(5_u8)).unwrap_err();
        assert!(error.ends_with("unexpected panic"));
    }

    #[test]
    fn only_named_red_table_threads_are_workers() {
        assert!(!is_worker_thread());
        let named = thread::Builder::new()
            .name(format!("{THREAD_PREFIX}test"))
            .spawn(is_worker_thread)
            .unwrap()
            .join()
            .unwrap();
        assert!(named);
    }
}

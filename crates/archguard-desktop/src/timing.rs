//! Time spent showing view states, for the snapshot build's benchmark
//! (`bench.rs`) and `ARCHGUARD_DESKTOP_TIMING=1`, which logs every push.
//! Without the `snapshot` feature [`measure`] only runs its closure.

#[cfg(feature = "snapshot")]
thread_local! {
    /// Milliseconds and count of measured pushes since the last [`take`].
    static TOTAL: std::cell::Cell<(f64, usize)> = const { std::cell::Cell::new((0.0, 0)) };
}

/// Run `f`, adding its time to the total under `label`.
#[cfg(feature = "snapshot")]
pub fn measure<R>(label: &str, f: impl FnOnce() -> R) -> R {
    let start = std::time::Instant::now();
    let r = f();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    TOTAL.with(|t| {
        let (sum, n) = t.get();
        t.set((sum + ms, n + 1));
    });
    if std::env::var_os("ARCHGUARD_DESKTOP_TIMING").is_some() {
        eprintln!("timing {label}: {ms:.2} ms");
    }
    r
}

#[cfg(not(feature = "snapshot"))]
pub fn measure<R>(_label: &str, f: impl FnOnce() -> R) -> R {
    f()
}

/// Milliseconds and number of pushes measured since the last call.
#[cfg(feature = "snapshot")]
pub fn take() -> (f64, usize) {
    TOTAL.with(|t| t.replace((0.0, 0)))
}

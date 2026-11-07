//! Headless rendering for snapshot mode: no window, no Dock icon, nothing
//! on the user's screen. With `ARCHGUARD_DESKTOP_SNAPSHOT` set (and no
//! `ARCHGUARD_DESKTOP_HOLD`, which needs a real window for renderer
//! measurements) the app uses Slint's software renderer on an off-screen
//! [`MinimalSoftwareWindow`]; `take_snapshot` renders it into the PNG.
//!
//! The event loop runs timers and `invoke_from_event_loop` jobs, as the
//! snapshot hooks and the worker threads need, until `quit_event_loop`.

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, WindowAdapter, WindowEvent};
use slint::{EventLoopError, PhysicalSize, PlatformError};

/// Logical window size and scale of the snapshots (as the real window).
const SIZE: (u32, u32) = (1480, 920);
const SCALE: f32 = 2.0;

type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    wake: Condvar,
    quit: AtomicBool,
}

struct Proxy(Arc<Queue>);

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.0.quit.store(true, Ordering::SeqCst);
        self.0.wake.notify_all();
        Ok(())
    }

    fn invoke_from_event_loop(&self, event: Job) -> Result<(), EventLoopError> {
        self.0
            .jobs
            .lock()
            .map_err(|_| EventLoopError::EventLoopTerminated)?
            .push_back(event);
        self.0.wake.notify_all();
        Ok(())
    }
}

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    queue: Arc<Queue>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.queue.clone())))
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        while !self.queue.quit.load(Ordering::SeqCst) {
            slint::platform::update_timers_and_animations();
            let jobs: Vec<Job> = match self.queue.jobs.lock() {
                Ok(mut q) => q.drain(..).collect(),
                Err(_) => break,
            };
            for job in jobs {
                job();
            }
            // Keep the window's dirty state current; the snapshot renders
            // the whole scene itself.
            self.window.draw_if_needed(|_| {});
            let wait = slint::platform::duration_until_next_timer_update()
                .unwrap_or(Duration::from_millis(50))
                .min(Duration::from_millis(50));
            if let Ok(q) = self.queue.jobs.lock()
                && q.is_empty()
                && !self.queue.quit.load(Ordering::SeqCst)
            {
                let _ = self.queue.wake.wait_timeout(q, wait);
            }
        }
        Ok(())
    }
}

/// Whether this run renders headless (decided once, before the window).
pub fn active() -> bool {
    std::env::var_os("ARCHGUARD_DESKTOP_SNAPSHOT").is_some()
        && std::env::var_os("ARCHGUARD_DESKTOP_HOLD").is_none()
}

/// Logical window size: `ARCHGUARD_DESKTOP_SIZE=WIDTHxHEIGHT`, else [`SIZE`].
fn size() -> (u32, u32) {
    std::env::var("ARCHGUARD_DESKTOP_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or(SIZE)
}

/// Install the headless platform if [`active`]. Call before creating the
/// first window.
pub fn install() -> Result<(), PlatformError> {
    if !active() {
        return Ok(());
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    let (w, h) = size();
    window.set_size(PhysicalSize::new(
        (w as f32 * SCALE) as u32,
        (h as f32 * SCALE) as u32,
    ));
    slint::platform::set_platform(Box::new(Headless {
        window,
        queue: Arc::new(Queue::default()),
    }))
    .map_err(|e| PlatformError::Other(e.to_string()))
}

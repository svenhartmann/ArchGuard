//! Callbacks that fire faster than a view can be rebuilt, coalesced on a
//! `slint::Timer`: search fields wait for a pause in typing
//! ([`Coalesce::debounce`]), drags apply their summed movement at most
//! once per frame ([`Coalesce::throttle`]).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{Timer, TimerMode};

/// The pause after the last keystroke before a search runs.
pub const TYPING: Duration = Duration::from_millis(150);
/// One frame at 60 Hz.
pub const FRAME: Duration = Duration::from_millis(16);

/// Arguments waiting for `run`, merged while its timer runs.
pub struct Coalesce<A> {
    pending: RefCell<Option<A>>,
    timer: Timer,
    delay: Duration,
    /// Debounce: every call restarts the timer.
    restart: bool,
    merge: fn(A, A) -> A,
    run: Box<dyn Fn(A)>,
}

impl<A: 'static> Coalesce<A> {
    /// `run` with the last argument once calls pause for `delay`.
    pub fn debounce(delay: Duration, run: impl Fn(A) + 'static) -> Rc<Self> {
        Self::new(delay, true, |_, last| last, run)
    }

    /// `run` at most once per `delay`, with the calls' arguments merged.
    pub fn throttle(delay: Duration, merge: fn(A, A) -> A, run: impl Fn(A) + 'static) -> Rc<Self> {
        Self::new(delay, false, merge, run)
    }

    fn new(
        delay: Duration,
        restart: bool,
        merge: fn(A, A) -> A,
        run: impl Fn(A) + 'static,
    ) -> Rc<Self> {
        Rc::new(Coalesce {
            pending: RefCell::new(None),
            timer: Timer::default(),
            delay,
            restart,
            merge,
            run: Box::new(run),
        })
    }

    /// Queue `a`; `run` follows when the timer fires.
    pub fn call(self: &Rc<Self>, a: A) {
        let merged = match self.pending.take() {
            Some(p) => (self.merge)(p, a),
            None => a,
        };
        self.pending.replace(Some(merged));
        if self.restart || !self.timer.running() {
            let me = Rc::downgrade(self);
            self.timer
                .start(TimerMode::SingleShot, self.delay, move || {
                    if let Some(me) = me.upgrade() {
                        me.flush();
                    }
                });
        }
    }

    /// Run what is queued now (e.g. Enter in a search field); whether
    /// anything was queued.
    pub fn flush(&self) -> bool {
        self.timer.stop();
        let pending = self.pending.take();
        let queued = pending.is_some();
        if let Some(a) = pending {
            (self.run)(a);
        }
        queued
    }

    /// A callback that queues its argument.
    pub fn callback(self: &Rc<Self>) -> impl Fn(A) + 'static {
        let me = self.clone();
        move |a| me.call(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn recorder<A: 'static>() -> (Rc<RefCell<Vec<A>>>, impl Fn(A) + 'static) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        (log, move |a| l.borrow_mut().push(a))
    }

    /// Keystrokes before the pause run one search, with the last text.
    #[test]
    fn a_debounce_runs_the_last_call_once() {
        let (log, run) = recorder::<String>();
        let d = Coalesce::debounce(TYPING, run);
        for q in ["s", "st", "sta"] {
            d.call(q.to_string());
        }
        assert!(log.borrow().is_empty(), "nothing runs while typing");
        assert!(d.timer.running());
        assert!(d.flush());
        assert!(!d.flush(), "nothing left");
        assert_eq!(*log.borrow(), ["sta"]);
        assert!(!d.timer.running());
    }

    /// Moves within a frame add up to one step.
    #[test]
    fn a_throttle_merges_the_calls() {
        let (log, run) = recorder::<f32>();
        let t = Coalesce::throttle(FRAME, |a, b| a + b, run);
        for _ in 0..4 {
            t.call(0.25);
        }
        t.flush();
        t.call(2.0);
        t.flush();
        assert_eq!(*log.borrow(), [1.0, 2.0]);
    }
}

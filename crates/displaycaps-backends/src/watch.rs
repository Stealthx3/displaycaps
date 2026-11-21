// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-backends/src/watch.rs; the sound
// device and sound-server event sources are dropped, a manual trigger and a timer remain.
//! Change events: a watch re-probes the watched scope when it is triggered and delivers the new
//! snapshot with its diff. Events whose re-probe changes nothing in scope are dropped.
//!
//! Triggers:
//! - [`Watch::trigger`], whenever the application knows something may have changed;
//! - a timer, every [`crate::runtime::ProbeOptions::watch_interval`] when set.
//!
//! Event sources of their own (DRM uevents for hotplug, compositor output changes) are added with
//! their backends in phases 3 to 5; they will push [`Cause`]s into the same loop. Only the
//! watched scope is re-probed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use displaycaps_model::{ProbeScope, Snapshot, SnapshotDiff};

use crate::probe::probe_inner;
use crate::root::SystemRoot;
use crate::runtime::{ProbeOptions, Shared};

/// What triggered an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// The first snapshot of the watch.
    Initial,
    /// The application called [`Watch::trigger`].
    Manual,
    /// The watch interval elapsed.
    Timer,
}

/// One update.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    /// The new snapshot (also published as the runtime's latest).
    pub snapshot: Arc<Snapshot>,
    /// Changes against the previous snapshot of this watch.
    pub diff: SnapshotDiff,
    /// What triggered it.
    pub causes: Vec<Cause>,
    /// Time from noticing the trigger to the new snapshot.
    pub latency: Duration,
}

/// A handle that triggers a watch from any thread (for the C ABI, which keeps the watch itself on
/// its event thread). Triggering a finished watch does nothing.
#[derive(Debug, Clone)]
pub struct Trigger(mpsc::Sender<()>);

impl Trigger {
    /// Asks the watch to re-probe now.
    pub fn fire(&self) {
        let _ = self.0.send(());
    }
}

/// A running watch. Dropping it stops the watch thread.
pub struct Watch {
    rx: mpsc::Receiver<WatchEvent>,
    trigger: mpsc::Sender<()>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Watch {
    pub(crate) fn start(
        root: SystemRoot,
        scope: ProbeScope,
        opts: ProbeOptions,
        shared: Arc<Shared>,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (trigger, trigger_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let handle = thread::Builder::new()
            .name("displaycaps-watch".into())
            .spawn(move || run(root, scope, opts, shared, tx, trigger_rx, s))?;
        Ok(Self {
            rx,
            trigger,
            stop,
            handle: Some(handle),
        })
    }

    /// Asks the watch to re-probe now. An update arrives only if something in scope changed.
    pub fn trigger(&self) {
        let _ = self.trigger.send(());
    }

    /// A handle that triggers this watch from another thread.
    pub fn trigger_handle(&self) -> Trigger {
        Trigger(self.trigger.clone())
    }

    /// Waits for the next update.
    pub fn recv(&self) -> Option<WatchEvent> {
        self.rx.recv().ok()
    }

    /// Waits up to `timeout` for the next update.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<WatchEvent> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// The next update, if one is waiting.
    pub fn try_recv(&self) -> Option<WatchEvent> {
        self.rx.try_recv().ok()
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// How often the loop looks at the stop flag and the timer.
const TICK: Duration = Duration::from_millis(5);

fn run(
    root: SystemRoot,
    scope: ProbeScope,
    opts: ProbeOptions,
    shared: Arc<Shared>,
    tx: mpsc::Sender<WatchEvent>,
    triggers: mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
) {
    let trace = std::env::var_os("DISPLAYCAPS_WATCH_TRACE").is_some();
    let probe = || {
        let mut snap = probe_inner(&root, &scope, &opts);
        snap.generation = shared.next_generation();
        shared.publish(snap)
    };
    let t0 = Instant::now();
    let mut prev = probe();
    let first = WatchEvent {
        snapshot: prev.clone(),
        diff: SnapshotDiff {
            from_generation: 0,
            to_generation: prev.generation,
            added: prev.endpoints.iter().map(|e| e.id.clone()).collect(),
            ..SnapshotDiff::default()
        },
        causes: vec![Cause::Initial],
        latency: t0.elapsed(),
    };
    if tx.send(first).is_err() {
        return;
    }
    let mut next_timer = opts.watch_interval.map(|d| Instant::now() + d);
    while !stop.load(Ordering::SeqCst) {
        let mut causes = vec![];
        if triggers.recv_timeout(TICK).is_ok() {
            // Coalesce a burst of triggers into one re-probe.
            while triggers.try_recv().is_ok() {}
            causes.push(Cause::Manual);
        }
        if let (Some(at), Some(every)) = (next_timer, opts.watch_interval)
            && Instant::now() >= at
        {
            next_timer = Some(Instant::now() + every);
            causes.push(Cause::Timer);
        }
        if causes.is_empty() {
            continue;
        }
        let noticed = Instant::now();
        let snap = probe();
        let diff = snap.diff(&prev);
        let latency = noticed.elapsed();
        if trace {
            eprintln!(
                "watch: probe {:.2} ms, causes {causes:?}",
                latency.as_secs_f64() * 1e3
            );
        }
        prev = snap.clone();
        if diff.is_empty() {
            continue;
        }
        let ev = WatchEvent {
            snapshot: snap,
            diff,
            causes,
            latency,
        };
        if tx.send(ev).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Runtime;
    use crate::testutil::TempFixture;
    use displaycaps_model::LayerKind;

    const WAIT: Duration = Duration::from_secs(5);

    fn scope() -> ProbeScope {
        ProbeScope::nothing().with_layer(LayerKind::Environment)
    }

    #[test]
    fn initial_event_then_only_real_changes() {
        let f = TempFixture::new("ID=debian\nVERSION_ID=\"12\"\n");
        let rt = Runtime::new(SystemRoot::fixture(f.path()).unwrap()).unwrap();
        let w = rt.watch(&scope(), &ProbeOptions::default()).unwrap();
        let first = w.recv_timeout(WAIT).expect("initial snapshot");
        assert_eq!(first.causes, [Cause::Initial]);
        assert_eq!(first.diff.from_generation, 0);
        assert!(first.snapshot.environment.is_some());

        // Nothing changed: the trigger re-probes but delivers nothing.
        w.trigger();
        assert!(w.recv_timeout(Duration::from_millis(200)).is_none());

        // The system changed: one event with the diff.
        f.set_os_release("ID=debian\nVERSION_ID=\"13\"\n");
        w.trigger();
        let ev = w.recv_timeout(WAIT).expect("changed snapshot");
        assert_eq!(ev.causes, [Cause::Manual]);
        assert!(ev.diff.system_changed);
        assert!(ev.snapshot.generation > first.snapshot.generation);
        assert_eq!(rt.latest().unwrap().generation, ev.snapshot.generation);
    }

    #[test]
    fn timer_triggers_reprobes() {
        let f = TempFixture::new("ID=debian\n");
        let rt = Runtime::new(SystemRoot::fixture(f.path()).unwrap()).unwrap();
        let opts = ProbeOptions::default().with_watch_interval(Duration::from_millis(20));
        let w = rt.watch(&scope(), &opts).unwrap();
        w.recv_timeout(WAIT).expect("initial snapshot");
        f.set_os_release("ID=ubuntu\nID_LIKE=debian\n");
        let ev = w.recv_timeout(WAIT).expect("timer update");
        assert!(ev.causes.contains(&Cause::Timer));
        assert!(ev.diff.system_changed);
    }

    #[test]
    fn dropping_the_watch_stops_it() {
        let f = TempFixture::new("ID=debian\n");
        let rt = Runtime::new(SystemRoot::fixture(f.path()).unwrap()).unwrap();
        let w = rt.watch(&scope(), &ProbeOptions::default()).unwrap();
        w.recv_timeout(WAIT).expect("initial snapshot");
        let start = Instant::now();
        drop(w);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}

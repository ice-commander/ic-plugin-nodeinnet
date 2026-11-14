//! The one thread that calls the host back: everything else marks what changed and returns.

use crate::account::{header_label, is_connected};
use crate::pairing::{PAIRING_VIEW, PAIR_VIEW};
use crate::view::VIEW_ID;
use std::ffi::CString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const ACCOUNT: u32 = 1 << 0;
pub const MULTICAST: u32 = 1 << 1;
pub const OPEN_PAIR: u32 = 1 << 2;
pub const PAIR: u32 = 1 << 3;
pub const OPEN_PAIRING: u32 = 1 << 4;
pub const PAIRING: u32 = 1 << 5;
pub const HEADER: u32 = 1 << 6;
pub const SETTINGS: u32 = 1 << 7;
pub const DRIVES: u32 = 1 << 8;

const GAP: Duration = Duration::from_millis(100);

#[derive(Default)]
struct Pending {
    bits: u32,
    deadlines: Vec<(&'static str, Instant)>,
    stop: bool,
}

struct Notifier {
    pending: Mutex<Pending>,
    wake: Condvar,
}

fn notifier() -> &'static Notifier {
    static HELD: OnceLock<Notifier> = OnceLock::new();
    HELD.get_or_init(|| Notifier {
        pending: Mutex::new(Pending::default()),
        wake: Condvar::new(),
    })
}

fn pending() -> MutexGuard<'static, Pending> {
    notifier()
        .pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn thread() -> MutexGuard<'static, Option<JoinHandle<()>>> {
    static HELD: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

static MARKED: [AtomicU64; 32] = [const { AtomicU64::new(0) }; 32];

pub fn mark(bits: u32) {
    for (bit, count) in MARKED.iter().enumerate() {
        if bits & (1 << bit) != 0 {
            count.fetch_add(1, Ordering::SeqCst);
        }
    }
    pending().bits |= bits;
    notifier().wake.notify_one();
}

/// Tests only.
pub fn marked(bit: u32) -> u64 {
    MARKED[bit.trailing_zeros() as usize % 32].load(Ordering::SeqCst)
}

pub fn invalidate_at(view: &'static str, at: Instant) {
    let mut held = pending();
    match held.deadlines.iter_mut().find(|(id, _)| *id == view) {
        Some((_, when)) => *when = (*when).min(at),
        None => held.deadlines.push((view, at)),
    }
    drop(held);
    notifier().wake.notify_one();
}

pub fn start() {
    let mut held = thread();
    if held.as_ref().is_some_and(|running| !running.is_finished()) {
        return;
    }
    pending().stop = false;
    *held = std::thread::Builder::new()
        .name("nin-notify".to_string())
        .spawn(run)
        .ok();
}

pub fn stop() {
    let running = thread().take();
    pending().stop = true;
    notifier().wake.notify_one();
    if let Some(running) = running {
        let _ = running.join();
    }
}

fn run() {
    let mut last: Option<Instant> = None;
    loop {
        let (bits, due) = {
            let mut held = pending();
            loop {
                if held.stop {
                    return;
                }
                let now = Instant::now();
                let open = last.map(|at| at + GAP);
                let deadline = held.deadlines.iter().map(|(_, at)| *at).min();
                let work = held.bits != 0 || deadline.is_some_and(|at| at <= now);
                let until = match (work, deadline) {
                    (true, _) => open.filter(|at| *at > now),
                    (false, Some(at)) => Some(open.map_or(at, |open| open.max(at))),
                    (false, None) => None,
                };
                held = match until {
                    None if work => break,
                    None => notifier()
                        .wake
                        .wait(held)
                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                    Some(at) => {
                        notifier()
                            .wake
                            .wait_timeout(held, at.saturating_duration_since(now))
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .0
                    }
                };
            }
            let now = Instant::now();
            let due: Vec<&'static str> = held
                .deadlines
                .iter()
                .filter(|(_, at)| *at <= now)
                .map(|(id, _)| *id)
                .collect();
            held.deadlines.retain(|(_, at)| *at > now);
            (std::mem::take(&mut held.bits), due)
        };
        last = Some(Instant::now());
        if bits & ACCOUNT != 0 {
            account();
        }
        if bits & HEADER != 0 {
            crate::paint_eye();
        }
        let mut stale = due;
        for (bit, view) in [
            (SETTINGS, VIEW_ID),
            (PAIR, PAIR_VIEW),
            (PAIRING, PAIRING_VIEW),
        ] {
            if bits & bit != 0 && !stale.contains(&view) {
                stale.push(view);
            }
        }
        if bits & ACCOUNT != 0 || !crate::view::showing() {
            stale.retain(|view| *view != VIEW_ID);
        }
        for view in stale {
            invalidate(view);
        }
        if bits & OPEN_PAIR != 0 && crate::pairing::anything_to_show() {
            open(PAIR_VIEW);
        }
        if bits & OPEN_PAIRING != 0 && crate::pairing::outgoing_to_show() {
            open(PAIRING_VIEW);
        }
        if bits & DRIVES != 0 && crate::drives::changed() {
            drives_changed();
        }
        if bits & MULTICAST != 0 {
            crate::lan::multicast();
        }
    }
}

fn drives_changed() {
    let host = crate::host();
    if host.is_null() {
        return;
    }
    unsafe { ((*host).drives_changed)() };
}

fn invalidate(view: &str) {
    let host = crate::host();
    if host.is_null() {
        return;
    }
    if let Ok(view) = CString::new(view) {
        unsafe { ((*host).view_invalidate)(view.as_ptr()) };
    }
}

fn open(view: &str) {
    let host = crate::host();
    if host.is_null() {
        return;
    }
    if let Ok(view) = CString::new(view) {
        unsafe { ((*host).open_view)(view.as_ptr(), std::ptr::null(), 0) };
    }
}

fn account() {
    let host = crate::host();
    if host.is_null() {
        return;
    }
    let held = crate::snapshot();
    let others = crate::sources::devices_online(&held.device_id, &crate::net::snapshot().nodes);
    let label = header_label(&held, others.len());
    if let (Ok(view), Ok(button), Ok(label)) = (
        CString::new(VIEW_ID),
        CString::new(crate::HEADER_ID),
        CString::new(label),
    ) {
        let drawn = if is_connected(&held) {
            crate::CONNECTED
        } else {
            crate::DISCONNECTED
        };
        unsafe {
            if let Ok(drawn) = CString::new(drawn) {
                ((*host).set_header_icon)(button.as_ptr(), drawn.as_ptr());
            }
            ((*host).set_header_label)(button.as_ptr(), label.as_ptr());
            ((*host).view_invalidate)(view.as_ptr());
            ((*host).pinned_connections_changed)();
        }
    }
}

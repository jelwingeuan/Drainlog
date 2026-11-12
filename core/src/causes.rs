//! Maps each DarkWake / awake interval to a cause, and each cause to one fix (plus undo).

use crate::ledger::{Interval, State, is_user_wake};
use crate::parse::Assertion;
use std::collections::HashMap;

/// A sleep-blocking assertion held from `start` to `end` by a non-system process.
#[derive(Debug, Clone)]
pub struct Hold {
    pub process: String,
    pub kind: String,
    pub start: i64,
    pub end: i64,
}

/// powerd and WindowServer hold these for the display and lid; that is the OS, not a cause.
const SYSTEM: [&str; 3] = ["powerd", "WindowServer", "kernel_task"];

/// Pairs Created/TurnedOn/Summary/Held rows with their Released/TimedOut/... row.
/// `asserts` must be sorted by ts. Holds still open end at `now`.
pub fn holds(asserts: &[Assertion], now: i64) -> Vec<Hold> {
    let mut open: HashMap<(i64, &str, &str), usize> = HashMap::new();
    let mut out: Vec<Hold> = Vec::new();
    for a in asserts
        .iter()
        .filter(|a| !SYSTEM.contains(&a.process.as_str()))
    {
        let key = (a.pid, a.kind.as_str(), a.name.as_str());
        match a.action.as_str() {
            "Released" | "TimedOut" | "ClientDied" | "TurnedOff" | "CapExpired" => {
                if let Some(i) = open.remove(&key) {
                    out[i].end = a.ts;
                }
            }
            _ => {
                open.entry(key).or_insert_with(|| {
                    out.push(Hold {
                        process: a.process.clone(),
                        kind: a.kind.clone(),
                        start: a.ts,
                        end: now,
                    });
                    out.len() - 1
                });
            }
        }
    }
    out
}

pub fn overlap(h: &Hold, start: i64, end: i64) -> i64 {
    h.end.min(end) - h.start.max(start)
}

/// The cause key for a DarkWake or awake interval inside a night.
/// A holder wins when it held a blocking assertion through at least half of the interval,
/// unless the interval is a short (< 10 min) DarkWake that a wake reason explains.
pub fn classify(iv: &Interval, sched: &[i64], holds: &[Hold]) -> String {
    let dur = iv.end - iv.start;
    let woke = iv.state == State::DarkWake && !iv.via_sleep;
    if !woke || dur >= 600 {
        let best = holds
            .iter()
            .map(|h| (overlap(h, iv.start, iv.end), h))
            .filter(|&(o, _)| o >= 0 && 2 * o >= dur)
            .max_by_key(|&(o, _)| o);
        if let Some((_, h)) = best {
            return format!("assertion:{}", h.process);
        }
    }
    let r = iv.reason.as_str();
    // Not woken by a reason (stayed up while trying to sleep, or a full wake), or the DarkWake
    // that starts the user's own wake (lid, power button): time awake, not a background wake.
    if !woke || is_user_wake(r) {
        return "awake".into();
    }
    let has = |keys: &[&str]| keys.iter().any(|k| r.contains(k));
    let key = if sched.iter().any(|&t| (t - iv.start).abs() <= 60) {
        "scheduled"
    } else if has(&["Maintenance", "SleepService"]) {
        "powernap"
    } else if has(&["ARPT", "E_RX_IP_PACKET", "TCPKeepAlive", "network", "en0"]) {
        "network"
    } else if has(&["wifibt", "HID"])
        || r.to_ascii_lowercase().contains("bluetooth")
        || r.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|t| t == "BT")
    {
        "bluetooth"
    } else if iv.flags.contains('P') {
        "powernap"
    } else {
        "other"
    };
    key.into()
}

/// Lower-case-first label; "Mostly {label}." reads as a sentence.
pub fn label(key: &str, clamshell: bool) -> String {
    match key {
        "baseline" => "baseline sleep drain".into(),
        "unexplained" => "unexplained".into(),
        "powernap" => "Power Nap / maintenance wakes".into(),
        "scheduled" => "scheduled wakes".into(),
        "bluetooth" => "Bluetooth / Wi-Fi chip wakes".into(),
        "network" => "network (TCPKeepAlive) wakes".into(),
        "awake" if clamshell => "awake time with the lid closed".into(),
        "awake" => "awake time while idle".into(),
        "other" => "other wakes".into(),
        // These daemons hold assertions for someone else: name what it means.
        "assertion:coreaudiod" => "audio playing (coreaudiod) holding it awake".into(),
        "assertion:runningboardd" => "an app (via runningboardd) holding it awake".into(),
        _ => format!("{} holding it awake", key.trim_start_matches("assertion:")),
    }
}

/// One fix and its undo. shortcut: a scheduled wake owned by the system (com.apple.alarm.*)
/// re-arms itself, so `cancelall` may not stick; Stage 2 checks fixes against later nights.
pub fn fix(key: &str) -> (String, String) {
    let (f, u) = match key {
        "powernap" => ("sudo pmset -b powernap 0", "sudo pmset -b powernap 1"),
        "network" => (
            "sudo pmset -b tcpkeepalive 0",
            "sudo pmset -b tcpkeepalive 1",
        ),
        "scheduled" => (
            "sudo pmset schedule cancelall",
            "sudo pmset schedule wake \"MM/DD/YYYY HH:MM:SS\" for any wake you still need",
        ),
        "bluetooth" => (
            "turn Bluetooth off before closing the lid",
            "turn Bluetooth back on",
        ),
        "awake" => (
            "run pmset -g assertions before closing the lid and quit what keeps the Mac awake",
            "nothing to undo",
        ),
        "assertion:caffeinate" => (
            "stop caffeinate before closing the lid: killall caffeinate",
            "run caffeinate again when you need it",
        ),
        "assertion:coreaudiod" => (
            "stop audio playback before closing the lid",
            "play it again",
        ),
        "assertion:runningboardd" => (
            "run pmset -g assertions before closing the lid and quit the app it names",
            "open the app again",
        ),
        k if k.starts_with("assertion:") => {
            let p = &k["assertion:".len()..];
            return (
                format!("quit {p} before closing the lid"),
                format!("open {p} again"),
            );
        }
        _ => ("", ""),
    };
    (f.into(), u.into())
}

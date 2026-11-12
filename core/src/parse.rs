//! Parsers for `pmset -g log`, `pmset -g sched` and `pmset -g assertions` text.
//! Pure functions: text in, rows out. Nothing here touches the system.

use chrono::{DateTime, NaiveDateTime, TimeZone};

/// One timestamped `pmset -g log` line other than Assertions,
/// or a planned wake (`kind == "Sched"`, `ts` = the planned wake time).
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub ts: i64,
    /// The Domain column: "Sleep", "DarkWake", "Wake", "Wake Requests", "WakeDetails", ...
    pub kind: String,
    /// Sleep: the quoted reason ("Clamshell Sleep"). (Dark)Wake: the "due to" text.
    /// Wake Requests: the starred (chosen) request. Sched: who scheduled it. Others: the message.
    pub reason: String,
    /// The capability letters in brackets on (Dark)Wake lines, e.g. "CDNPB".
    pub flags: String,
    pub on_battery: Option<bool>,
    pub charge_pct: Option<i64>,
    /// The trailing "N secs" (how long the state lasted), when present.
    pub duration_s: Option<i64>,
    pub message: String,
    /// FNV-1a of the raw line; (ts, hash) is the dedupe key.
    pub hash: i64,
}

/// One assertion line from the log, or a "Held" row from a `pmset -g assertions` snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Assertion {
    pub ts: i64,
    pub pid: i64,
    pub process: String,
    /// Created, Released, TimedOut, ClientDied, TurnedOn, TurnedOff, Summary, CapExpired, Held.
    pub action: String,
    /// The assertion type, e.g. "PreventUserIdleSystemSleep".
    pub kind: String,
    pub name: String,
    pub hash: i64,
}

/// Stable 64-bit FNV-1a (std's hasher is not guaranteed stable across Rust versions).
pub fn fnv1a(s: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    h as i64
}

/// "2026-11-13 02:55:52 +0800" -> unix seconds.
pub fn parse_ts(s: &str) -> Option<i64> {
    DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z")
        .ok()
        .map(|d| d.timestamp())
}

/// Parses the whole `pmset -g log` text. Lines without a leading timestamp and a tab
/// (headers, "DriverReason:" continuations, the trailing live-assertion dump) are skipped.
pub fn parse_log(text: &str) -> (Vec<Event>, Vec<Assertion>) {
    let (mut events, mut asserts) = (Vec::new(), Vec::new());
    for line in text.lines() {
        let Some(ts) = line.get(..25).and_then(parse_ts) else {
            continue;
        };
        let Some((domain, rest)) = line[25..].split_once('\t') else {
            continue;
        };
        let kind = domain.trim();
        let msg = rest.split('\t').next().unwrap_or("").trim();
        let hash = fnv1a(line.trim_end());
        if kind == "Assertions" {
            asserts.extend(parse_assertion(ts, msg, hash));
            continue;
        }
        let ev = event(ts, kind, msg, hash);
        if kind == "Wake Requests" {
            events.extend(sched_from_request(&ev.reason, &line[20..25]));
        }
        events.push(ev);
    }
    (events, asserts)
}

fn event(ts: i64, kind: &str, msg: &str, hash: i64) -> Event {
    let due = msg.split_once("due to ").map(|(_, r)| r);
    let until_using = |r: &str| r.split(" Using ").next().unwrap_or("").trim().to_string();
    let reason = match kind {
        "Sleep" | "SleepAborted" => due
            .map(
                |r| match r.strip_prefix('\'').and_then(|q| q.split_once('\'')) {
                    Some((quoted, _)) => quoted.to_string(),
                    None => until_using(r),
                },
            )
            .unwrap_or_default(),
        "DarkWake" | "Wake" => due.map(until_using).unwrap_or_default(),
        "Wake Requests" => msg
            .split_once("[*")
            .and_then(|(_, r)| r.split_once(']'))
            .map(|(r, _)| r.to_string())
            .unwrap_or_default(),
        _ => msg.to_string(),
    };
    let flags = msg
        .split_once('[')
        .and_then(|(_, r)| r.split_once(']'))
        .map(|(f, _)| f)
        .filter(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_uppercase()))
        .unwrap_or("");
    let on_battery = if msg.contains("Using AC") {
        Some(false)
    } else if msg.to_ascii_lowercase().contains("using batt") {
        Some(true)
    } else {
        None
    };
    let charge_pct = msg
        .split_once("(Charge:")
        .and_then(|(_, r)| r.split('%').next()?.trim().parse().ok());
    // The duration only ever follows the "(Charge:N%)" field.
    let duration_s = msg
        .split_once("%)")
        .and_then(|(_, r)| r.trim().strip_suffix("secs")?.trim().parse().ok());
    Event {
        ts,
        kind: kind.to_string(),
        reason,
        flags: flags.to_string(),
        on_battery,
        charge_pct,
        duration_s,
        message: msg.to_string(),
        hash,
    }
}

/// A starred `request=UserWake` in "Wake Requests" is a calendar/alarm wake that powerd armed
/// (the RTC alarm). Turn it into a Sched row at its wakeAt time (local time, line's offset).
fn sched_from_request(req: &str, offset: &str) -> Option<Event> {
    if !req.contains("request=UserWake") {
        return None;
    }
    let at = req.split_once("wakeAt=")?.1.get(..19)?;
    let ts = parse_ts(&format!("{at} {offset}"))?;
    let info = req
        .split_once("info=\"")
        .and_then(|(_, r)| r.split_once('"'))
        .map_or("", |(i, _)| i);
    Some(sched(ts, info, req))
}

/// Parses `pmset -g sched` ("[0]  wake at 11/13/2026 03:40:42 by 'owner'") in the given
/// time zone. Only one-off wake entries count. shortcut: "Repeating power events" are ignored.
pub fn parse_sched<Tz: TimeZone>(text: &str, tz: &Tz) -> Vec<Event> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((_, rest)) = line.split_once("] ") else {
            continue;
        };
        let Some((what, rest)) = rest.trim_start().split_once(" at ") else {
            continue;
        };
        if !what.starts_with("wake") {
            continue;
        }
        let Some(naive) = rest
            .get(..19)
            .and_then(|d| NaiveDateTime::parse_from_str(d, "%m/%d/%Y %H:%M:%S").ok())
        else {
            continue;
        };
        let Some(ts) = tz
            .from_local_datetime(&naive)
            .earliest()
            .map(|d| d.timestamp())
        else {
            continue;
        };
        let owner = rest
            .split_once(" by '")
            .map_or("", |(_, o)| o.trim_end().trim_end_matches('\''));
        out.push(sched(ts, owner, line.trim()));
    }
    out
}

/// Sched rows from both sources dedupe on (ts, owner): "com.x.alarm,350" and "com.x.alarm" match.
fn sched(ts: i64, owner: &str, message: &str) -> Event {
    let owner = match owner.rsplit_once(',') {
        Some((o, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => o,
        _ => owner,
    };
    Event {
        ts,
        kind: "Sched".into(),
        reason: owner.to_string(),
        flags: String::new(),
        on_battery: None,
        charge_pct: None,
        duration_s: None,
        message: message.to_string(),
        hash: fnv1a(&format!("sched|{ts}|{owner}")),
    }
}

/// `PID 491(mDNSResponder) Created MaintenanceWake "mDNSResponder:maintenance" 00:00:00  id:...`
fn parse_assertion(ts: i64, msg: &str, hash: i64) -> Option<Assertion> {
    let (pid, rest) = msg.strip_prefix("PID ")?.split_once('(')?;
    let (process, rest) = rest.split_once(") ")?;
    let (action, rest) = rest.split_once(' ')?;
    let (kind, rest) = rest.split_once(" \"")?;
    let name = rest
        .split_once("\" ")
        .map_or(rest.trim_end_matches('"'), |(n, _)| n);
    Some(Assertion {
        ts,
        pid: pid.parse().ok()?,
        process: process.into(),
        action: action.into(),
        kind: kind.trim().into(),
        name: name.into(),
        hash,
    })
}

/// `pmset -g assertions` "Listed by owning process" rows, as action "Held". The snapshot's
/// first line carries its timestamp; `now` is the fallback.
pub fn parse_assertions_snapshot(text: &str, now: i64) -> Vec<Assertion> {
    let ts = text
        .lines()
        .next()
        .and_then(|l| l.get(..25))
        .and_then(parse_ts)
        .unwrap_or(now);
    let row = |line: &str| -> Option<Assertion> {
        // pid 32088(Claude): [0x00076fcc00019037] 00:13:44 NoIdleSleepAssertion named: "Electron"
        let (pid, rest) = line.trim_start().strip_prefix("pid ")?.split_once('(')?;
        let (process, rest) = rest.split_once("): ")?;
        let (_, rest) = rest.split_once("] ")?;
        let (_, rest) = rest.split_once(' ')?;
        let (kind, name) = rest.split_once(" named: ")?;
        Some(Assertion {
            ts,
            pid: pid.parse().ok()?,
            process: process.into(),
            action: "Held".into(),
            kind: kind.trim().into(),
            name: name.trim().trim_matches('"').into(),
            hash: fnv1a(line.trim()),
        })
    };
    text.lines().filter_map(row).collect()
}

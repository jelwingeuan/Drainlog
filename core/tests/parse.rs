//! Parser tests on real captures from this Mac (tests/fixtures/pmset_log_sample.txt is a
//! trimmed copy of a real `pmset -g log`; all of it is on AC power).

use chrono::FixedOffset;
use drainlog::battery::{self, Sample};
use drainlog::parse::{self, parse_ts};
use drainlog::store;
use std::collections::BTreeMap;

const SAMPLE: &str = include_str!("fixtures/pmset_log_sample.txt");

fn ts(s: &str) -> i64 {
    parse_ts(s).unwrap()
}

#[test]
fn counts_per_kind() {
    let (events, asserts) = parse::parse_log(SAMPLE);
    let mut counts = BTreeMap::new();
    for e in &events {
        *counts.entry(e.kind.as_str()).or_insert(0) += 1;
    }
    let expected = BTreeMap::from([
        ("DarkWake", 10),
        ("HibernateStats", 9),
        ("Kernel Client Acks", 18),
        ("Notification", 5),
        ("PM Client Acks", 6),
        ("Sched", 1), // derived from the one starred request=UserWake
        ("Sleep", 9),
        ("SleepAborted", 1),
        ("Wake", 6),
        ("Wake Requests", 7),
        ("WakeDetails", 16),
        ("WakeTime", 7),
        ("com.apple.sleepservices.sessionStarted", 7),
        ("com.apple.sleepservices.sessionTerminated", 7),
    ]);
    assert_eq!(counts, expected);
    // 92 Assertions lines, 5 of them "Summary- [System: ...]" without a PID.
    assert_eq!(asserts.len(), 87);
}

#[test]
fn event_fields() {
    let (events, _) = parse::parse_log(SAMPLE);
    let find = |kind: &str, at: &str| {
        events
            .iter()
            .find(|e| e.kind == kind && e.ts == ts(at))
            .unwrap()
    };

    let dw = find("DarkWake", "2026-11-06 03:21:36 +0800");
    assert_eq!(dw.reason, "NUB.SPMI0Sw3IRQ nub-spmi.0x02 rtc/Maintenance");
    assert_eq!(dw.flags, "CDNPB");
    assert_eq!(
        (dw.on_battery, dw.charge_pct, dw.duration_s),
        (Some(false), Some(80), Some(12))
    );

    let lid_close = find("Sleep", "2026-11-06 03:53:41 +0800");
    assert!(lid_close.message.starts_with("Entering DarkWake state"));
    assert_eq!(
        (lid_close.reason.as_str(), lid_close.duration_s),
        ("Clamshell Sleep", None)
    );

    let sleep = find("Sleep", "2026-11-06 03:21:48 +0800");
    assert_eq!(
        (
            sleep.reason.as_str(),
            sleep.flags.as_str(),
            sleep.duration_s
        ),
        ("Maintenance Sleep", "", Some(900))
    );

    let wake = find("Wake", "2026-11-06 06:44:47 +0800");
    assert_eq!(
        wake.reason,
        "smc.sysState.Wake(0x70070000) lid SMC.OutboxNotEmpty/UserActivity Assertion"
    );
    assert_eq!((wake.flags.as_str(), wake.duration_s), ("CDNVA", None));

    let notif = find("Wake", "2026-11-06 03:37:54 +0800");
    assert_eq!(
        (
            notif.reason.as_str(),
            notif.flags.as_str(),
            notif.duration_s
        ),
        ("Notification", "CDNVAPB", Some(11))
    );

    let req = find("Wake Requests", "2026-11-06 03:21:49 +0800");
    assert!(
        req.reason
            .starts_with("process=PowerUIAgent request=Maintenance deltaSecs=899"),
        "{}",
        req.reason
    );

    let sched = events.iter().find(|e| e.kind == "Sched").unwrap();
    assert_eq!(sched.ts, ts("2026-11-06 21:04:27 +0800"));
    assert_eq!(
        sched.reason,
        "com.apple.alarm.user-invisible-com.apple.acmd.alarm"
    );

    let aborted = events.iter().find(|e| e.kind == "SleepAborted").unwrap();
    assert_eq!(
        aborted.reason,
        "(TCP KeepAlive activity -- TCPKeepAlive=active)"
    );

    // No events from header rows, DriverReason continuations or the trailing assertion dump.
    assert!(
        events
            .iter()
            .all(|e| !e.kind.starts_with(':') && !e.kind.is_empty())
    );
}

#[test]
fn assertion_fields() {
    let (_, asserts) = parse::parse_log(SAMPLE);
    let a = &asserts[0];
    assert_eq!((a.ts, a.pid), (ts("2026-11-06 03:21:36 +0800"), 491));
    assert_eq!(
        (a.process.as_str(), a.action.as_str()),
        ("mDNSResponder", "Created")
    );
    assert_eq!(
        (a.kind.as_str(), a.name.as_str()),
        ("MaintenanceWake", "mDNSResponder:maintenance")
    );

    let chrome = asserts.iter().find(|a| a.pid == 62885).unwrap();
    assert_eq!(
        (
            chrome.process.as_str(),
            chrome.kind.as_str(),
            chrome.name.as_str()
        ),
        (
            "Google Chrome",
            "NoDisplaySleepAssertion",
            "Video Wake Lock"
        )
    );

    let caf = asserts.iter().find(|a| a.process == "caffeinate").unwrap();
    assert_eq!(
        (caf.pid, caf.action.as_str(), caf.kind.as_str()),
        (64337, "Summary", "PreventUserIdleSystemSleep")
    );
    assert_eq!(caf.name, "caffeinate command-line tool");

    let kernel = asserts.iter().find(|a| a.pid == 0).unwrap();
    assert_eq!(
        (
            kernel.process.as_str(),
            kernel.kind.as_str(),
            kernel.name.as_str()
        ),
        ("kernel_task", "Kernel Assertion", "NETWORK")
    );

    let actions: std::collections::BTreeSet<_> =
        asserts.iter().map(|a| a.action.as_str()).collect();
    for want in [
        "Created",
        "Released",
        "TimedOut",
        "ClientDied",
        "TurnedOn",
        "TurnedOff",
        "Summary",
        "CapExpired",
    ] {
        assert!(actions.contains(want), "missing {want}");
    }
}

#[test]
fn snapshots() {
    let held = parse::parse_assertions_snapshot(include_str!("fixtures/assertions.txt"), 0);
    assert_eq!(held.len(), 5);
    assert!(
        held.iter()
            .all(|a| a.ts == ts("2026-11-13 03:15:17 +0800") && a.action == "Held")
    );
    assert_eq!((held[0].pid, held[0].process.as_str()), (32088, "Claude"));
    assert_eq!(
        (held[0].kind.as_str(), held[0].name.as_str()),
        ("NoIdleSleepAssertion", "Electron")
    );
    assert_eq!(held[4].process, "WindowServer");

    let sched = parse::parse_sched(
        include_str!("fixtures/sched.txt"),
        &FixedOffset::east_opt(8 * 3600).unwrap(),
    );
    assert_eq!(sched.len(), 2);
    assert_eq!(sched[0].ts, ts("2026-11-13 03:40:42 +0800"));
    assert_eq!(
        sched[0].reason,
        "com.apple.alarm.user-invisible-com.apple.acmd.alarm"
    );
    assert_eq!(sched[1].ts, ts("2026-11-13 08:08:03 +0800"));
}

#[test]
fn battery_props() {
    // Apple silicon, as read on this Mac (AppleRaw* keys; CurrentCapacity is a percent).
    let m1 = |k: &str| match k {
        "AppleRawCurrentCapacity" => Some(2940),
        "AppleRawMaxCapacity" => Some(3913),
        "CurrentCapacity" => Some(79),
        "MaxCapacity" => Some(100),
        "ExternalConnected" => Some(1),
        _ => None,
    };
    assert_eq!(
        battery::from_props(7, m1),
        Some(Sample {
            ts: 7,
            mah: 2940,
            max_mah: 3913,
            pct: 79,
            on_ac: true
        })
    );
    // Intel shape (untested on hardware): CurrentCapacity/MaxCapacity in mAh.
    let intel = |k: &str| match k {
        "CurrentCapacity" => Some(3000),
        "MaxCapacity" => Some(4000),
        "ExternalConnected" => Some(0),
        _ => None,
    };
    assert_eq!(
        battery::from_props(7, intel),
        Some(Sample {
            ts: 7,
            mah: 3000,
            max_mah: 4000,
            pct: 75,
            on_ac: false
        })
    );
    // Percent only, no mAh anywhere: no sample rather than a made-up one.
    assert_eq!(
        battery::from_props(7, |k| (k == "CurrentCapacity" || k == "MaxCapacity")
            .then_some(100)),
        None
    );
}

#[test]
fn ingest_is_idempotent() {
    let conn = store::open(":memory:").unwrap();
    let (ev, asr) = store::ingest_log(&conn, SAMPLE).unwrap();
    assert_eq!((ev, asr), (109 - 18 - 6, 87)); // the "Client Acks" domains are not stored
    assert_eq!(store::ingest_log(&conn, SAMPLE).unwrap(), (0, 0));
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 85);
    // All on AC: no night. Lid close 03:38 -> lid open 06:44 and idle sleep 20:55 -> power
    // button 00:31 are night-shaped, so they are counted as skipped.
    let l = store::rebuild(&conn, ts("2026-11-13 03:15:17 +0800")).unwrap();
    assert!(l.nights.is_empty());
    assert_eq!(l.skipped_ac, 2);
}

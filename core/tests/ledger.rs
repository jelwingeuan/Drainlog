//! Ledger and pricing tests on SYNTHETIC battery nights (tests/fixtures/synthetic/*.txt).
//! The real log on this Mac has no battery nights, so these are hand-made in the exact
//! `pmset -g log` format with "#SAMPLE" lines standing in for IOKit samples.

use chrono::FixedOffset;
use drainlog::battery::Sample;
use drainlog::ledger::Ledger;
use drainlog::parse::{self, parse_ts};
use drainlog::store;
use rusqlite::Connection;

const CLEAN: &str = include_str!("fixtures/synthetic/clean_nights.txt");
const POWERNAP: &str = include_str!("fixtures/synthetic/powernap_night.txt");
const SCHEDULED: &str = include_str!("fixtures/synthetic/scheduled_night.txt");
const CAFFEINATE: &str = include_str!("fixtures/synthetic/caffeinate_night.txt");

fn ts(s: &str) -> i64 {
    parse_ts(s).unwrap()
}

/// Ingests the fixtures in order (log lines + #SAMPLE lines) and rebuilds.
fn run(texts: &[&str], extra: impl Fn(&Connection)) -> (Connection, Ledger) {
    let conn = store::open(":memory:").unwrap();
    for text in texts {
        store::ingest_log(&conn, text).unwrap();
        for rest in text.lines().filter_map(|l| l.strip_prefix("#SAMPLE ")) {
            let kv = |k: &str| -> i64 {
                rest[26..]
                    .split_whitespace()
                    .find_map(|p| p.strip_prefix(&format!("{k}=")))
                    .unwrap()
                    .parse()
                    .unwrap()
            };
            let s = Sample {
                ts: ts(&rest[..25]),
                mah: kv("mah"),
                max_mah: kv("max"),
                pct: kv("pct"),
                on_ac: kv("ac") != 0,
            };
            store::insert_sample(&conn, &s, "test").unwrap();
        }
    }
    extra(&conn);
    let ledger = store::rebuild(&conn, ts("2026-11-07 12:00:00 +0800")).unwrap();
    check_sums(&conn);
    (conn, ledger)
}

/// baseline + attributed + unexplained == measured drop, exactly, in the stored rows too.
fn check_sums(conn: &Connection) {
    let mut st = conn
        .prepare(
            "SELECT start_ts, drop_mah, baseline_mah, attributed_mah, unexplained_mah FROM nights",
        )
        .unwrap();
    let rows: Vec<(i64, i64, i64, i64, i64)> = st
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!rows.is_empty());
    for (night, drop, base, attr, unexpl) in rows {
        assert_eq!(base + attr + unexpl, drop, "night {night}");
        let sum: i64 = conn
            .query_row(
                "SELECT SUM(mah) FROM causes WHERE night = ?1",
                [night],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sum, drop, "causes of night {night}");
    }
}

/// (mah, estimated) of a cause in the latest night.
fn cause(conn: &Connection, key: &str) -> Option<(i64, bool)> {
    conn.query_row(
        "SELECT mah, estimated FROM causes WHERE cause = ?1 AND night = (SELECT MAX(start_ts) FROM nights)",
        [key],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

fn latest(conn: &Connection) -> (String, String, i64, bool, bool) {
    conn.query_row(
        "SELECT top_cause, headline, drop_mah, measured, estimated FROM nights ORDER BY start_ts DESC LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .unwrap()
}

#[test]
fn clean_night_is_normal_sleep_drain() {
    let (conn, l) = run(&[CLEAN], |_| {});
    assert_eq!(l.nights.len(), 2);
    let b = &l.baseline;
    assert!(b.calibrated && b.source == "clean" && b.clean_nights == 2);
    assert!((b.mah_per_h - 10.0).abs() < 0.01, "{}", b.mah_per_h);
    assert_eq!(b.dark_source, "default"); // 2 nights: too few to fit, flagged
    let (top, headline, drop, measured, estimated) = latest(&conn);
    assert_eq!(
        (top.as_str(), drop, measured, estimated),
        ("normal", 80, true, true)
    );
    assert_eq!(
        headline,
        "You lost 2% (80 mAh) overnight. Normal sleep drain."
    );
    assert_eq!(cause(&conn, "baseline"), Some((80, true)));
    assert_eq!(cause(&conn, "unexplained"), Some((0, false)));
    let n = &l.nights[1];
    assert!((3..=5).contains(&n.timeline.len()));
    assert!(
        n.timeline[1].1.starts_with("No wakes during the night"),
        "{:?}",
        n.timeline
    );
}

#[test]
fn clean_night_before_calibration_says_so() {
    // Only the first clean night: baseline from 1 night is not calibrated yet.
    let first_night: String = CLEAN
        .lines()
        .take_while(|l| !l.starts_with("2026-11-02 18"))
        .collect::<Vec<_>>()
        .join("\n");
    let (conn, l) = run(&[&first_night], |_| {});
    assert!(!l.baseline.calibrated && l.baseline.clean_nights == 1);
    let (top, headline, ..) = latest(&conn);
    assert_eq!(top, "normal");
    assert!(
        l.nights[0].timeline[2].1.contains("not calibrated yet"),
        "{headline}"
    );
}

#[test]
fn powernap_night() {
    let (conn, l) = run(&[CLEAN, POWERNAP], |_| {});
    assert_eq!(l.nights.len(), 3);
    assert_eq!(
        (l.baseline.dark_source, l.baseline.nights_fitted),
        ("fit", 3)
    );
    let (top, headline, drop, measured, estimated) = latest(&conn);
    assert_eq!(
        (top.as_str(), drop, measured, estimated),
        ("powernap", 350, true, true)
    );
    assert!(
        headline.starts_with("You lost 9% (350 mAh) overnight. Mostly Power Nap"),
        "{headline}"
    );
    let (nap, nap_est) = cause(&conn, "powernap").unwrap();
    assert!(nap_est, "29 of 31 DarkWakes are priced by the fitted rate");
    assert!(nap * 100 / drop > 60, "{nap}");
    // The two sampled DarkWakes are measured at 2 mAh each.
    let measured: Vec<f64> = conn
        .prepare("SELECT mah FROM intervals WHERE measured = 1 AND night = (SELECT MAX(start_ts) FROM nights)")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(measured, vec![2.0, 2.0]);
    let n = l.nights.last().unwrap();
    assert_eq!(n.bucket("powernap").unwrap().count, 31);
    assert!(
        n.timeline
            .iter()
            .any(|(_, t)| t == "Fix: sudo pmset -b powernap 0 (undo: sudo pmset -b powernap 1)."),
        "{:?}",
        n.timeline
    );
}

#[test]
fn scheduled_wake_night() {
    let sched = "Scheduled power events:\n [0]  wake at 11/05/2026 04:00:00 by 'pmset'\n [1]  wake at 11/05/2026 05:00:00 by 'pmset'\n";
    let (conn, l) = run(&[CLEAN, SCHEDULED], |c| {
        store::insert_events(
            c,
            &parse::parse_sched(sched, &FixedOffset::east_opt(8 * 3600).unwrap()),
        )
        .unwrap();
    });
    let (top, headline, drop, ..) = latest(&conn);
    assert_eq!((top.as_str(), drop), ("scheduled", 220));
    assert!(headline.contains("Mostly scheduled wakes."), "{headline}");
    // All four matched a schedule (2 from Wake Requests, 2 from the snapshot), even though
    // the reason text says rtc/Maintenance.
    assert_eq!(
        l.nights.last().unwrap().bucket("scheduled").unwrap().count,
        4
    );
    assert_eq!(cause(&conn, "powernap"), None);
    assert!(
        cause(&conn, "scheduled").unwrap().1,
        "priced by rate, so estimated"
    );
}

#[test]
fn caffeinate_night() {
    let (conn, l) = run(&[CLEAN, CAFFEINATE], |_| {});
    let (top, headline, drop, ..) = latest(&conn);
    assert_eq!((top.as_str(), drop), ("assertion:caffeinate", 255));
    assert_eq!(
        headline,
        "You lost 7% (255 mAh) overnight. Mostly caffeinate holding it awake."
    );
    // The held DarkWake had samples on both sides: measured, not estimated.
    assert_eq!(cause(&conn, "assertion:caffeinate"), Some((195, false)));
    assert_eq!(cause(&conn, "baseline"), Some((60, true)));
    assert_eq!(cause(&conn, "unexplained"), Some((0, false)));
    assert_eq!(
        l.baseline.clean_nights, 3,
        "the sleep after caffeinate exits is a clean stretch"
    );
    assert!(
        l.nights
            .last()
            .unwrap()
            .timeline
            .iter()
            .any(|(_, t)| t.contains("killall caffeinate"))
    );
}

#[test]
fn caffeinate_held_but_lid_sleep_won() {
    // Apple silicon on battery: lid-close sleep overrides `caffeinate -i`. The night stays
    // normal, and the report says the assertion lost.
    let held = "2026-11-02 22:55:00 +0800 Assertions          \tPID 777(caffeinate) Created PreventUserIdleSystemSleep \"caffeinate command-line tool\" 00:00:00  id:0x0x1 [System: PrevIdle DeclUser kDisp]\n\
                2026-11-03 06:00:00 +0800 Assertions          \tPID 777(caffeinate) Released PreventUserIdleSystemSleep \"caffeinate command-line tool\" 07:05:00  id:0x0x1 [System: DeclUser kDisp]\n";
    let text = format!("{CLEAN}\n{held}");
    let (conn, l) = run(&[&text], |_| {});
    let (top, ..) = latest(&conn);
    assert_eq!(top, "normal");
    let n = l.nights.last().unwrap();
    assert_eq!(
        n.timeline.last().unwrap().1,
        "caffeinate held PreventUserIdleSystemSleep, but lid-close sleep won."
    );
}

#[test]
fn coarse_night_without_bounding_samples() {
    // Same Power Nap night, but no sample near its start or end: the drop falls back to
    // Charge % x max_mah and is labelled coarse.
    let no_bounds: String = POWERNAP
        .lines()
        .filter(|l| {
            !(l.starts_with("#SAMPLE 2026-11-03 23:00:01")
                || l.starts_with("#SAMPLE 2026-11-04 07:00:02"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let (conn, l) = run(&[CLEAN, &no_bounds], |_| {});
    let (_, headline, drop, measured, _) = latest(&conn);
    assert!(!measured);
    assert_eq!(drop, ((87 - 78) as f64 * 3913.0 / 100.0).round() as i64);
    assert!(
        headline.starts_with("You lost about 9% (~352 mAh, from Charge %)"),
        "{headline}"
    );
    assert!(l.nights.last().unwrap().timeline[3].1.contains("coarse"));
}

#[test]
fn ac_nights_are_not_nights() {
    let on_ac = CLEAN.replace("Using Batt", "Using AC");
    let conn = store::open(":memory:").unwrap();
    store::ingest_log(&conn, &on_ac).unwrap();
    let l = store::rebuild(&conn, ts("2026-11-07 12:00:00 +0800")).unwrap();
    assert!(l.nights.is_empty());
    assert_eq!(l.skipped_ac, 2);
}

//! Ledger and pricing tests on SYNTHETIC battery nights (tests/fixtures/synthetic/*.txt).
//! The real log on this Mac has no battery nights, so these are hand-made in the exact
//! `pmset -g log` format with "#SAMPLE" lines standing in for IOKit samples.

mod common;

use chrono::FixedOffset;
use drainlog::ledger::Ledger;
use drainlog::parse::{self, parse_ts};
use drainlog::store;
use rusqlite::Connection;

const CLEAN: &str = include_str!("fixtures/synthetic/clean_nights.txt");
const POWERNAP: &str = include_str!("fixtures/synthetic/powernap_night.txt");
const SCHEDULED: &str = include_str!("fixtures/synthetic/scheduled_night.txt");
const CAFFEINATE: &str = include_str!("fixtures/synthetic/caffeinate_night.txt");
const BLUETOOTH: &str = include_str!("fixtures/synthetic/bluetooth_night.txt");
const TWO_CAUSES: &str = include_str!("fixtures/synthetic/two_causes_night.txt");
const CANT_TELL: &str = include_str!("fixtures/synthetic/cant_tell_night.txt");

fn ts(s: &str) -> i64 {
    parse_ts(s).unwrap()
}

/// Ingests the fixtures in order (log lines + #SAMPLE lines) and rebuilds.
fn run(texts: &[&str], extra: impl Fn(&Connection)) -> (Connection, Ledger) {
    let conn = store::open(":memory:").unwrap();
    for text in texts {
        store::ingest_log(&conn, text).unwrap();
        for s in common::samples(text) {
            store::insert_sample(&conn, &s, "test").unwrap();
        }
    }
    extra(&conn);
    let ledger = store::rebuild(&conn, ts("2026-11-10 12:00:00 +0800")).unwrap();
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
    // Only the first clean night. Its own sleep may not calibrate it, so with no other night
    // it is priced at the 4.0 mAh/h default: 48 of 80 mAh unexplained and a quiet log, so it
    // can't be told apart from a can't-tell night yet. The stored baseline (for the next
    // night) does use it.
    let first_night: String = CLEAN
        .lines()
        .take_while(|l| !l.starts_with("2026-11-02 18"))
        .collect::<Vec<_>>()
        .join("\n");
    let (conn, l) = run(&[&first_night], |_| {});
    assert!(!l.baseline.calibrated && l.baseline.clean_nights == 1);
    assert_eq!(l.nights[0].baseline.source, "default");
    let (top, headline, ..) = latest(&conn);
    assert_eq!(top, "cant_tell");
    assert_eq!(
        headline,
        "You lost 2% (80 mAh) overnight. Can't tell yet: baseline not measured."
    );
    assert_eq!(cause(&conn, "unexplained"), Some((48, false)));
    assert_eq!(
        l.nights[0].timeline[2].1,
        "Split estimated: baseline sleep 32 mAh at 4.0 mAh/h (default guess, not measured yet, est.); unexplained 48 mAh (60%)."
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
    // The fit uses the other nights only, and 2 are too few: 29 of 31 DarkWakes are priced
    // at the default rate, so unexplained is the largest part. 23 min of logged wakes is
    // too much activity to call it can't tell, so Power Nap is named, as estimated.
    assert_eq!(l.nights[2].baseline.dark_source, "default");
    assert_eq!(
        headline,
        "You lost 9% (350 mAh) overnight. Mostly Power Nap / maintenance wakes. 66% is unexplained."
    );
    assert_eq!(cause(&conn, "powernap"), Some((43, true)));
    assert_eq!(cause(&conn, "unexplained"), Some((231, false)));
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
fn uncalibrated_night_ranks_causes_by_logged_time() {
    // One night and no other, so default rates. A measured 60 s Power Nap wake (40 mAh) costs
    // more than 20 min held by caffeinate at the default rate (36 mAh). With no calibrated
    // baseline the mAh split is a guess, so the longer logged activity is named.
    let night = "2026-11-08 18:00:00 +0800 Wake                \tWake from Deep Idle [CDNVA] : due to smc.sysState.Wake(0x70070000) lid SMC.OutboxNotEmpty/UserActivity Assertion Using Batt (Charge:90%)\n\
                 2026-11-08 22:55:00 +0800 Assertions          \tPID 6161(caffeinate) Created PreventUserIdleSystemSleep \"caffeinate command-line tool\" 00:00:00  id:0x0x10000a161 [System: PrevIdle DeclUser kDisp]\n\
                 #SAMPLE 2026-11-08 22:58:00 +0800 mah=3400 max=3913 pct=87 ac=0\n\
                 2026-11-08 23:00:00 +0800 Sleep               \tEntering DarkWake state due to 'Clamshell Sleep':TCPKeepAlive=active Using Batt (Charge:87%) 1205 secs\n\
                 2026-11-08 23:20:00 +0800 Assertions          \tPID 6161(caffeinate) Released PreventUserIdleSystemSleep \"caffeinate command-line tool\" 00:25:00  id:0x0x10000a161 [System: PrevIdle DeclUser kDisp]\n\
                 2026-11-08 23:20:05 +0800 Sleep               \tEntering Sleep state due to 'Clamshell Sleep':TCPKeepAlive=active Using Batt (Charge:86%) 13195 secs\n\
                 2026-11-09 03:00:00 +0800 DarkWake            \tDarkWake from Deep Idle [CDNPB] : due to NUB.SPMI0Sw3IRQ nub-spmi.0x02 rtc/Maintenance Using Batt (Charge:85%) 60 secs\n\
                 #SAMPLE 2026-11-09 03:00:01 +0800 mah=3340 max=3913 pct=85 ac=0\n\
                 #SAMPLE 2026-11-09 03:00:59 +0800 mah=3300 max=3913 pct=84 ac=0\n\
                 2026-11-09 03:01:00 +0800 Sleep               \tEntering Sleep state due to 'Maintenance Sleep':TCPKeepAlive=active Using Batt (Charge:84%) 14340 secs\n\
                 2026-11-09 07:00:00 +0800 Wake                \tWake from Deep Idle [CDNVA] : due to smc.sysState.Wake(0x70070000) lid SMC.OutboxNotEmpty/UserActivity Assertion Using Batt (Charge:84%)\n\
                 #SAMPLE 2026-11-09 07:00:02 +0800 mah=3280 max=3913 pct=84 ac=0\n";
    let (conn, l) = run(&[night], |_| {});
    let n = l.nights.last().unwrap();
    assert_eq!(n.top_cause, "assertion:caffeinate");
    assert_eq!(cause(&conn, "powernap"), Some((40, false)));
    assert_eq!(cause(&conn, "assertion:caffeinate"), Some((36, true)));
    // Its clean stretch after the 03:00 wake counts for the next night, not for itself.
    assert_eq!((n.baseline.source, l.baseline.source), ("default", "clean"));
    assert_eq!(
        n.timeline[2].1,
        "Split estimated: baseline sleep 31 mAh at 4.0 mAh/h (default guess, not measured yet, est.); unexplained 13 mAh (11%)."
    );
}

#[test]
fn bluetooth_night() {
    // 47 wifibt DarkWakes of 10 s. The other nights (2 clean) are too few for a fitted
    // DarkWake rate, so the wakes are priced at the default rate: 14 mAh, with 37 mAh
    // unexplained. Only 7.8 min, but 47 logged wakes is not a quiet log: Bluetooth is named.
    let (conn, l) = run(&[CLEAN, BLUETOOTH], |_| {});
    let (top, headline, drop, ..) = latest(&conn);
    assert_eq!((top.as_str(), drop), ("bluetooth", 130));
    assert_eq!(
        headline,
        "You lost 3% (130 mAh) overnight. Mostly Bluetooth / Wi-Fi chip wakes. 28% is unexplained."
    );
    assert_eq!(cause(&conn, "unexplained"), Some((37, false)));
    assert_eq!(cause(&conn, "bluetooth"), Some((14, true)));
    // wifibt wins over the P (Power Nap) flag these DarkWake lines also carry.
    assert_eq!(cause(&conn, "powernap"), None);
    let n = l.nights.last().unwrap();
    assert_eq!(n.bucket("bluetooth").unwrap().count, 47);
    assert!(
        n.timeline
            .iter()
            .any(|(_, t)| t.starts_with("Fix: turn Bluetooth off before closing the lid")),
        "{:?}",
        n.timeline
    );
}

#[test]
fn two_causes_the_larger_wins() {
    let (conn, l) = run(&[CLEAN, TWO_CAUSES], |_| {});
    let (top, headline, drop, ..) = latest(&conn);
    assert_eq!((top.as_str(), drop), ("assertion:caffeinate", 197));
    assert_eq!(
        headline,
        "You lost 5% (197 mAh) overnight. Mostly caffeinate holding it awake."
    );
    assert_eq!(cause(&conn, "assertion:caffeinate"), Some((97, false)));
    // Power Nap woke it 27 times but cost less (at the default rate, as the 2 other nights
    // are too few for a fit): second, still in the causes table.
    assert_eq!(cause(&conn, "powernap"), Some((36, true)));
    let n = l.nights.last().unwrap();
    assert_eq!(n.bucket("powernap").unwrap().count, 27);
    assert_eq!(n.causes[1].cause, "assertion:caffeinate");
    assert_eq!(n.causes[2].cause, "powernap");
}

#[test]
fn cant_tell_night() {
    let (conn, l) = run(&[CLEAN, CANT_TELL], |_| {});
    let (top, headline, drop, measured, _) = latest(&conn);
    assert_eq!((top.as_str(), drop, measured), ("cant_tell", 350, true));
    assert_eq!(
        headline,
        "You lost 9% (350 mAh) overnight. Can't tell why from the logs."
    );
    assert_eq!(cause(&conn, "baseline"), Some((80, true)));
    assert_eq!(cause(&conn, "unexplained"), Some((270, false)));
    let n = l.nights.last().unwrap();
    assert!(
        n.timeline.iter().all(|(_, t)| !t.starts_with("Fix:")),
        "{:?}",
        n.timeline
    );
    assert_eq!(
        n.timeline.last().unwrap().1,
        "No fix to suggest: macOS logged no wake or hold covering the unexplained drain."
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

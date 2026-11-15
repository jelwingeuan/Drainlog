//! Replays the judge's REAL nights. Each row of judge/nights.csv whose capture folder
//! (tests/fixtures/nights/<night>/, written by judge/capture.sh) exists is ingested on its own,
//! and its latest night's top cause is compared with the cause written down before that night.
//! It passes when at least 90% match. Every miss is printed either way.

mod common;

use chrono::DateTime;
use drainlog::ledger::Night;
use drainlog::{parse, store};
use std::fs;
use std::io::Write;
use std::path::Path;

/// The latest night in one capture folder.
fn replay(dir: &Path) -> Option<Night> {
    let read = |f: &str| {
        let path = dir.join(f);
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    let snapshot = read("assertions.txt");
    // Its first line is the capture time with the Mac's UTC offset: replay's "now", and the
    // zone of the bare local times in sched.txt (CI runs in UTC).
    let taken = snapshot
        .get(..25)
        .and_then(|s| DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z").ok())
        .unwrap_or_else(|| panic!("{}: no capture time on line 1", dir.display()));
    let now = taken.timestamp();
    let conn = store::open(":memory:").unwrap();
    store::ingest_log(&conn, &read("pmset_log.txt")).unwrap();
    store::insert_assertions(&conn, &parse::parse_assertions_snapshot(&snapshot, now)).unwrap();
    let sched = parse::parse_sched(&read("sched.txt"), taken.offset());
    store::insert_events(&conn, &sched).unwrap();
    for s in common::samples(&read("samples.txt")) {
        store::insert_sample(&conn, &s, "capture").unwrap();
    }
    store::rebuild(&conn, now).unwrap().nights.pop()
}

#[test]
fn real_nights_match_the_expected_cause() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let csv = fs::read_to_string(root.join("../judge/nights.csv")).expect("judge/nights.csv");
    let (mut lines, mut misses, mut pending) = (Vec::new(), Vec::new(), 0);
    for row in csv.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let mut f = row.splitn(3, ',').map(str::trim);
        let (night, expected) = (f.next().unwrap_or(""), f.next().unwrap_or(""));
        let dir = root.join("tests/fixtures/nights").join(night);
        if night.is_empty() || !dir.is_dir() {
            pending += 1;
            continue;
        }
        // The notes carry "caffeinate held ..., but lid-close sleep won." for the README's
        // overridden-caffeinate nights.
        let (got, said) = match replay(&dir) {
            Some(n) => (n.top_cause, format!("{} {}", n.headline, n.notes.join(" "))),
            None => ("no night".to_string(), String::new()),
        };
        let line = format!("{night}: expected {expected}, got {got}. {said}")
            .trim_end()
            .to_string();
        if got != expected {
            misses.push(format!("MISS {line}"));
        }
        lines.push(line);
    }
    let (n, matched) = (lines.len(), lines.len() - misses.len());
    // Written straight to stderr, past the test harness's capture, so a plain `cargo test`
    // shows the score and every miss, pass or fail.
    let mut err = std::io::stderr();
    for line in lines.iter().chain(&misses) {
        let _ = writeln!(err, "replay: {line}");
    }
    let _ = writeln!(
        err,
        "replay: {n} real nights replayed, {matched} matched; the pass mark is 90% \
         ({pending} listed but not captured yet)."
    );
    // The Stage 1 target: the right top cause on at least 9 of 10 nights.
    assert!(
        matched * 10 >= n * 9,
        "{matched} of {n} real nights matched, under 90%:\n{}",
        misses.join("\n")
    );
}

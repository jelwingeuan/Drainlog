//! Writes a demo database from the SYNTHETIC test fixtures, for checking the app's UI while
//! the real database has no battery night yet:
//!   cargo run --example demo_db -- <out.db> [fixture ...]
//! Fixtures, in date order: clean, clean1 (only the first clean night), powernap, scheduled,
//! caffeinate, bluetooth, two_causes, cant_tell. With none named, all but clean1.
//! Then point the app at it: DRAINLOG_DB=<out.db> open build/Drainlog.app

#[path = "../tests/common/mod.rs"]
mod common;

use chrono::FixedOffset;
use drainlog::parse::{self, parse_ts};
use drainlog::store;

const CLEAN: &str = include_str!("../tests/fixtures/synthetic/clean_nights.txt");
const ALL: [&str; 7] = [
    "clean",
    "powernap",
    "scheduled",
    "caffeinate",
    "bluetooth",
    "two_causes",
    "cant_tell",
];

fn fixture(name: &str) -> Option<String> {
    Some(match name {
        "clean" => CLEAN.into(),
        // As in the ledger test: everything before the second clean night.
        "clean1" => CLEAN
            .lines()
            .take_while(|l| !l.starts_with("2026-11-02 18"))
            .collect::<Vec<_>>()
            .join("\n"),
        "powernap" => include_str!("../tests/fixtures/synthetic/powernap_night.txt").into(),
        "scheduled" => include_str!("../tests/fixtures/synthetic/scheduled_night.txt").into(),
        "caffeinate" => include_str!("../tests/fixtures/synthetic/caffeinate_night.txt").into(),
        "bluetooth" => include_str!("../tests/fixtures/synthetic/bluetooth_night.txt").into(),
        "two_causes" => include_str!("../tests/fixtures/synthetic/two_causes_night.txt").into(),
        "cant_tell" => include_str!("../tests/fixtures/synthetic/cant_tell_night.txt").into(),
        _ => return None,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((out, names)) = args.split_first() else {
        eprintln!("usage: demo_db <out.db> [fixture ...]");
        std::process::exit(2);
    };
    let names: Vec<&str> = if names.is_empty() {
        ALL.to_vec()
    } else {
        names.iter().map(String::as_str).collect()
    };
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{out}{suffix}"));
    }
    let conn = store::open(out).expect("open the demo database");
    for name in names {
        let text = fixture(name).unwrap_or_else(|| {
            eprintln!("unknown fixture {name}");
            std::process::exit(2);
        });
        store::ingest_log(&conn, &text).unwrap();
        for s in common::samples(&text) {
            store::insert_sample(&conn, &s, "demo").unwrap();
        }
        if name == "scheduled" {
            // The same schedule snapshot the ledger test adds for this night.
            let sched = "Scheduled power events:\n [0]  wake at 11/05/2026 04:00:00 by 'pmset'\n [1]  wake at 11/05/2026 05:00:00 by 'pmset'\n";
            let zone = FixedOffset::east_opt(8 * 3600).unwrap();
            store::insert_events(&conn, &parse::parse_sched(sched, &zone)).unwrap();
        }
    }
    let ledger = store::rebuild(&conn, parse_ts("2026-11-10 12:00:00 +0800").unwrap()).unwrap();
    println!("{} nights written to {out}", ledger.nights.len());
}

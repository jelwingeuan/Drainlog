//! SQLite store. Rust writes, the Swift app reads these tables directly.

use crate::battery::Sample;
use crate::ledger::{self, Ledger};
use crate::parse::{self, Assertion, Event};
use rusqlite::{Connection, Result, params};
use std::path::PathBuf;

/// Rebuilds look this far back. shortcut: nothing is pruned yet; old rows just stop mattering.
pub const HISTORY_DAYS: i64 = 60;

pub const SCHEMA: &str = r#"
-- Drainlog schema v1. Times are unix seconds (UTC). Booleans are 0/1. mAh are integers
-- unless noted. The raw tables (events, samples, assertions) only grow; the derived tables
-- (intervals, nights, causes, baseline) are deleted and rebuilt on every run.
--
-- events      pmset -g log lines except Assertions and the noisy "* Client Acks" domains.
--             kind = the Domain column ("Sleep", "DarkWake", "Wake", "Wake Requests", ...).
--             kind 'Sched' = a planned wake (from pmset -g sched or a starred UserWake
--             request): ts is the planned time, reason who scheduled it.
--             reason = sleep reason / "due to" text; flags = [CDNPB] letters (P = Power Nap);
--             on_battery 1/0/NULL; charge_pct = "Charge:N%"; duration_s = trailing "N secs".
-- samples     IOKit AppleSmartBattery reads: mah = AppleRawCurrentCapacity,
--             max_mah = AppleRawMaxCapacity, pct = CurrentCapacity (%), on_ac = ExternalConnected,
--             source = sleep | wake | timer | once.
-- assertions  pmset assertion lines. action = Created/Released/TimedOut/... or 'Held' for a
--             pmset -g assertions snapshot row. type = assertion type.
-- intervals   the log cut into states: state = asleep | darkwake | awake. night = nights.start_ts
--             or NULL. cause = causes.cause. mah REAL, NULL outside nights; measured 1 = the
--             mAh came from IOKit samples on both sides, 0 = estimated.
-- nights      one row per on-battery night (lid/idle sleep -> user wake, >= 2 h).
--             measured 1 = drop_mah from IOKit samples, 0 = coarse (Charge % x max_mah).
--             estimated 1 = some part of the split is an estimate. unexplained_mah may be < 0.
--             timeline = JSON [{"ts": int or null, "text": str}].
-- causes      per-night buckets including 'baseline' and 'unexplained';
--             sum(mah) per night == nights.drop_mah exactly. estimated 1 = priced by a rate.
-- baseline    one row (id 1). mah_per_h = sleep drain, source clean | fit | default;
--             calibrated 1 once clean_nights >= 2. darkwake_mah_per_s source fit | default.
-- fixes       fixes the user applied (Stage 2; empty for now).
CREATE TABLE IF NOT EXISTS events (ts INTEGER NOT NULL, kind TEXT NOT NULL, reason TEXT NOT NULL,
  flags TEXT NOT NULL, on_battery INTEGER, charge_pct INTEGER, duration_s INTEGER,
  message TEXT NOT NULL, hash INTEGER NOT NULL, UNIQUE (ts, hash));
CREATE TABLE IF NOT EXISTS samples (ts INTEGER PRIMARY KEY, mah INTEGER NOT NULL,
  max_mah INTEGER NOT NULL, pct INTEGER NOT NULL, on_ac INTEGER NOT NULL, source TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS assertions (ts INTEGER NOT NULL, pid INTEGER NOT NULL,
  process TEXT NOT NULL, action TEXT NOT NULL, type TEXT NOT NULL, name TEXT NOT NULL,
  hash INTEGER NOT NULL, UNIQUE (ts, hash));
CREATE TABLE IF NOT EXISTS intervals (start_ts INTEGER NOT NULL, end_ts INTEGER NOT NULL,
  state TEXT NOT NULL, reason TEXT NOT NULL, flags TEXT NOT NULL, night INTEGER,
  cause TEXT NOT NULL, mah REAL, measured INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS nights (start_ts INTEGER PRIMARY KEY, end_ts INTEGER NOT NULL,
  start_pct INTEGER NOT NULL, end_pct INTEGER NOT NULL, drop_pct INTEGER NOT NULL,
  drop_mah INTEGER NOT NULL, max_mah INTEGER NOT NULL, measured INTEGER NOT NULL,
  baseline_mah INTEGER NOT NULL, attributed_mah INTEGER NOT NULL, unexplained_mah INTEGER NOT NULL,
  unexplained_pct REAL NOT NULL, top_cause TEXT NOT NULL, estimated INTEGER NOT NULL,
  baseline_calibrated INTEGER NOT NULL, headline TEXT NOT NULL, timeline TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS causes (night INTEGER NOT NULL, cause TEXT NOT NULL, label TEXT NOT NULL,
  mah INTEGER NOT NULL, seconds INTEGER NOT NULL, count INTEGER NOT NULL,
  estimated INTEGER NOT NULL, fix TEXT NOT NULL, undo TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS baseline (id INTEGER PRIMARY KEY CHECK (id = 1), mah_per_h REAL NOT NULL,
  source TEXT NOT NULL, clean_nights INTEGER NOT NULL, calibrated INTEGER NOT NULL,
  darkwake_mah_per_s REAL NOT NULL, darkwake_source TEXT NOT NULL, nights_fitted INTEGER NOT NULL,
  updated_ts INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS fixes (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, cause TEXT NOT NULL,
  command TEXT NOT NULL, undo TEXT NOT NULL);
"#;

/// `$DRAINLOG_DB`, else ~/Library/Application Support/Drainlog/drainlog.db (dir created).
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DRAINLOG_DB") {
        return p.into();
    }
    let dir = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/Drainlog");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("drainlog.db")
}

pub fn open(path: impl AsRef<std::path::Path>) -> Result<Connection> {
    let conn = Connection::open(path)?;
    // WAL lets the Swift app read while the agent writes; the timeout covers `--once`
    // running while the agent writes.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// Ingests `pmset -g log` text. Lines older than the newest stored event are skipped before
/// hashing; the (ts, hash) unique key makes re-ingesting anything a no-op.
/// shortcut: two byte-identical lines in the same second are stored once.
/// Returns (new events, new assertion rows).
pub fn ingest_log(conn: &Connection, text: &str) -> Result<(usize, usize)> {
    let cutoff: i64 = conn.query_row(
        "SELECT COALESCE(MAX(ts), 0) FROM events WHERE kind != 'Sched'",
        [],
        |r| r.get(0),
    )?;
    let (events, asserts) = parse::parse_log(text);
    let events: Vec<_> = events
        .into_iter()
        .filter(|e| e.ts >= cutoff && !e.kind.ends_with("Client Acks"))
        .collect();
    let asserts: Vec<_> = asserts.into_iter().filter(|a| a.ts >= cutoff).collect();
    Ok((
        insert_events(conn, &events)?,
        insert_assertions(conn, &asserts)?,
    ))
}

pub fn insert_events(conn: &Connection, events: &[Event]) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut n = 0;
    {
        let mut st = tx.prepare_cached(
            "INSERT OR IGNORE INTO events (ts, kind, reason, flags, on_battery, charge_pct, duration_s, message, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for e in events {
            n += st.execute(params![
                e.ts,
                e.kind,
                e.reason,
                e.flags,
                e.on_battery,
                e.charge_pct,
                e.duration_s,
                e.message,
                e.hash
            ])?;
        }
    }
    tx.commit()?;
    Ok(n)
}

pub fn insert_assertions(conn: &Connection, asserts: &[Assertion]) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut n = 0;
    {
        let mut st = tx.prepare_cached(
            "INSERT OR IGNORE INTO assertions (ts, pid, process, action, type, name, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for a in asserts {
            n += st.execute(params![
                a.ts, a.pid, a.process, a.action, a.kind, a.name, a.hash
            ])?;
        }
    }
    tx.commit()?;
    Ok(n)
}

pub fn insert_sample(conn: &Connection, s: &Sample, source: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO samples (ts, mah, max_mah, pct, on_ac, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![s.ts, s.mah, s.max_mah, s.pct, s.on_ac, source],
    )?;
    Ok(())
}

/// Loads the recent rows the ledger needs, rebuilds every derived table, returns the ledger.
pub fn rebuild(conn: &Connection, now: i64) -> Result<Ledger> {
    let since = now - HISTORY_DAYS * 86_400;
    let events = conn
        .prepare("SELECT ts, kind, reason, flags, on_battery, charge_pct, duration_s, message, hash FROM events
                  WHERE ts >= ?1 AND kind IN ('Sleep', 'DarkWake', 'Wake', 'Sched') ORDER BY ts, rowid")?
        .query_map([since], |r| {
            Ok(Event {
                ts: r.get(0)?,
                kind: r.get(1)?,
                reason: r.get(2)?,
                flags: r.get(3)?,
                on_battery: r.get(4)?,
                charge_pct: r.get(5)?,
                duration_s: r.get(6)?,
                message: r.get(7)?,
                hash: r.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    let samples = conn
        .prepare("SELECT ts, mah, max_mah, pct, on_ac FROM samples WHERE ts >= ?1 ORDER BY ts")?
        .query_map([since], |r| {
            Ok(Sample {
                ts: r.get(0)?,
                mah: r.get(1)?,
                max_mah: r.get(2)?,
                pct: r.get(3)?,
                on_ac: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    let asserts = conn
        .prepare("SELECT ts, pid, process, action, type, name, hash FROM assertions WHERE ts >= ?1
                  AND type IN ('PreventUserIdleSystemSleep', 'PreventSystemSleep', 'NoIdleSleepAssertion') ORDER BY ts, rowid")?
        .query_map([since], |r| {
            Ok(Assertion {
                ts: r.get(0)?,
                pid: r.get(1)?,
                process: r.get(2)?,
                action: r.get(3)?,
                kind: r.get(4)?,
                name: r.get(5)?,
                hash: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    let ledger = ledger::build(&events, &samples, &asserts, now);
    save(conn, &ledger, now)?;
    Ok(ledger)
}

fn save(conn: &Connection, l: &Ledger, now: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "DELETE FROM intervals; DELETE FROM nights; DELETE FROM causes; DELETE FROM baseline;",
    )?;
    let b = &l.baseline;
    tx.execute(
        "INSERT INTO baseline VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            b.mah_per_h,
            b.source,
            b.clean_nights as i64,
            b.calibrated,
            b.dark_mah_per_s,
            b.dark_source,
            b.nights_fitted as i64,
            now
        ],
    )?;
    {
        let mut st =
            tx.prepare("INSERT INTO intervals VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)")?;
        for i in &l.intervals {
            st.execute(params![
                i.start,
                i.end,
                i.state.as_str(),
                i.reason,
                i.flags,
                i.night,
                i.cause,
                i.mah,
                i.measured
            ])?;
        }
        let mut st = tx.prepare("INSERT INTO nights VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)")?;
        let mut sc =
            tx.prepare("INSERT INTO causes VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)")?;
        for n in &l.nights {
            let timeline: Vec<_> = n
                .timeline
                .iter()
                .map(|(ts, text)| serde_json::json!({"ts": ts, "text": text}))
                .collect();
            st.execute(params![
                n.start,
                n.end,
                n.start_pct,
                n.end_pct,
                n.drop_pct,
                n.drop_mah,
                n.max_mah,
                n.measured,
                n.baseline_mah(),
                n.attributed_mah(),
                n.unexplained_mah(),
                n.unexplained_pct(),
                n.top_cause,
                n.causes.iter().any(|c| c.estimated),
                b.calibrated,
                n.headline,
                serde_json::Value::from(timeline).to_string(),
            ])?;
            for c in &n.causes {
                sc.execute(params![
                    n.start,
                    c.cause,
                    c.label,
                    c.mah,
                    c.seconds,
                    c.count,
                    c.estimated,
                    c.fix,
                    c.undo
                ])?;
            }
        }
    }
    tx.commit()
}

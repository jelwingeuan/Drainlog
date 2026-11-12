//! The morning report: one headline and a 3-5 line timeline (what happened, why, one fix).
//! Every number says whether it was measured or estimated. No raw log lines.

use crate::ledger::{Baseline, Night};
use chrono::{Local, TimeZone};
use rusqlite::{Connection, OptionalExtension, Result};

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

fn dur(s: i64) -> String {
    match s {
        ..60 => format!("{s} s"),
        60..3600 => format!("{} min", (s + 30) / 60),
        _ => format!("{:.1} h", s as f64 / 3600.0),
    }
}

fn est(estimated: bool) -> &'static str {
    if estimated { "est." } else { "measured" }
}

fn who(reason: &str) -> &'static str {
    let has = |t: &str| reason.split_whitespace().any(|w| w == t);
    if has("lid") {
        "opening the lid"
    } else if has("pwrbtn") {
        "the power button"
    } else if reason.contains("UserActivity") || reason.contains("HID") {
        "keyboard or trackpad"
    } else {
        "a long full wake"
    }
}

fn base_label(b: &Baseline) -> &'static str {
    match (b.source, b.calibrated) {
        ("clean", true) => "calibrated",
        ("clean", false) => "1 clean night, not calibrated yet",
        ("fit", _) => "fitted, not calibrated yet",
        _ => "default guess, not calibrated yet",
    }
}

pub fn clock(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map_or_else(|| ts.to_string(), |d| d.format("%H:%M").to_string())
}

/// Sets `headline` and `timeline` from the priced night.
pub fn fill(n: &mut Night, base: &Baseline) {
    let drop = n.drop_mah;
    let up = n.unexplained_pct().round() as i64;
    let top = n.bucket(&n.top_cause).cloned();
    let lost = match (drop > 0, n.measured) {
        (false, _) => format!("No battery lost overnight ({drop} mAh)."),
        (true, true) => format!("You lost {}% ({drop} mAh) overnight.", n.drop_pct),
        (true, false) => format!(
            "You lost about {}% (~{drop} mAh, from Charge %) overnight.",
            n.drop_pct
        ),
    };
    let verdict = match &top {
        None => "Normal sleep drain.".to_string(),
        Some(t) if n.unexplained_mah() > t.mah && up > 15 => {
            format!("Largest known cause: {}.", t.label)
        }
        Some(t) => format!("Mostly {}.", t.label),
    };
    let tail = if up.abs() > 15 {
        format!(
            " {up}% is unexplained{}.",
            if base.calibrated {
                ""
            } else {
                " (baseline not calibrated yet)"
            }
        )
    } else {
        String::new()
    };
    n.headline = format!("{lost} {verdict}{tail}");

    let base_b = &n.causes[0];
    let mut t = vec![(
        Some(n.start),
        format!(
            "{} at {}%; asleep {:.1} of {:.1} h.",
            if n.clamshell {
                "Lid closed"
            } else {
                "Went to sleep (idle)"
            },
            n.start_pct,
            base_b.seconds as f64 / 3600.0,
            (n.end - n.start) as f64 / 3600.0
        ),
    )];
    t.push((
        None,
        match &top {
            Some(c) => {
                let times = if c.count > 0 {
                    format!("{}x over ", c.count)
                } else {
                    String::new()
                };
                format!(
                    "{}: {times}{}, {} mAh ({}).",
                    cap(&c.label),
                    dur(c.seconds),
                    c.mah,
                    est(c.estimated)
                )
            }
            None => {
                let wakes: i64 = n.attributed().map(|c| c.count).sum();
                let mah: i64 = n.attributed().map(|c| c.mah).sum();
                let any_est = n.attributed().any(|c| c.estimated);
                if wakes == 0 {
                    "No wakes during the night.".to_string()
                } else {
                    format!(
                        "No wake stood out: {wakes} wakes, {mah} mAh in all ({}).",
                        est(any_est)
                    )
                }
            }
        },
    ));
    t.push((
        None,
        format!(
            "Baseline sleep {} mAh at {:.1} mAh/h ({}, est.); unexplained {} mAh ({up}%).",
            base_b.mah,
            base.mah_per_h,
            base_label(base),
            n.unexplained_mah()
        ),
    ));
    t.push((
        Some(n.end),
        format!(
            "Woken by {} at {}% ({}).",
            who(&n.wake_reason),
            n.end_pct,
            if n.measured {
                "measured"
            } else {
                "from Charge %, coarse"
            }
        ),
    ));
    match top.filter(|c| !c.fix.is_empty()) {
        Some(c) => t.push((None, format!("Fix: {} (undo: {}).", c.fix, c.undo))),
        None => t.extend(n.notes.first().map(|note| (None, note.clone()))),
    }
    n.timeline = t;
}

/// The latest night as plain text, for `drainlogd --report`.
pub fn latest(conn: &Connection) -> Result<Option<String>> {
    let row = conn
        .query_row("SELECT start_ts, end_ts, headline, timeline, drop_mah, measured FROM nights ORDER BY start_ts DESC LIMIT 1", [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, bool>(5)?))
        })
        .optional()?;
    let Some((start, end, headline, timeline, drop, measured)) = row else {
        return Ok(None);
    };
    let day = Local
        .timestamp_opt(start, 0)
        .single()
        .map_or(String::new(), |d| d.format("%a %-d %b").to_string());
    let mut out = format!(
        "Night of {day}, {} -> {} ({:.1} h, on battery)\n{headline}\n",
        clock(start),
        clock(end),
        (end - start) as f64 / 3600.0
    );
    let items: Vec<serde_json::Value> = serde_json::from_str(&timeline).unwrap_or_default();
    for it in &items {
        let when = it["ts"].as_i64().map_or("     ".to_string(), clock);
        out += &format!("  {when}  {}\n", it["text"].as_str().unwrap_or(""));
    }
    let parts: Vec<String> = conn
        .prepare("SELECT cause, label, mah, estimated FROM causes WHERE night = ?1 ORDER BY rowid")?
        .query_map([start], |r| {
            let (cause, label, mah, e): (String, String, i64, bool) =
                (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
            let how = if cause == "unexplained" {
                "remainder"
            } else {
                est(e)
            };
            Ok(format!("{label} {mah} ({how})"))
        })?
        .collect::<Result<_>>()?;
    out += &format!(
        "  Split (mAh): {} = {drop} {}\n",
        parts.join(" + "),
        if measured { "measured" } else { "coarse" }
    );
    Ok(Some(out))
}

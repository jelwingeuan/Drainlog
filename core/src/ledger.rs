//! Events -> intervals -> nights, and the pricing that splits each night's measured drop into
//! baseline + attributed causes + unexplained (which may be negative and is never hidden).

use crate::battery::Sample;
use crate::causes::{self, Hold};
use crate::parse::{Assertion, Event};
use crate::report;

pub const MIN_NIGHT_S: i64 = 2 * 3600;
pub const MIN_CLEAN_S: i64 = 3600;
/// Conservative (low) defaults used until there is data: a low guess pushes drain into
/// "unexplained" instead of blaming a cause. shortcut: picked for Apple silicon, not measured.
pub const DEFAULT_SLEEP_MAH_H: f64 = 4.0;
pub const DEFAULT_DARK_MAH_S: f64 = 0.03;
/// How far a sample may be from the moment it bounds.
const NIGHT_TOL: i64 = 600;
const STRETCH_TOL: i64 = 120;
const WAKE_TOL: i64 = 60;
/// shortcut: a full wake this long with no user-wake line (e.g. the lid opened during a
/// notification wake) is treated as the user being back; it ends the night.
const LONG_AWAKE_S: i64 = 1800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Asleep,
    DarkWake,
    Awake,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Asleep => "asleep",
            State::DarkWake => "darkwake",
            State::Awake => "awake",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Interval {
    pub start: i64,
    pub end: i64,
    pub state: State,
    pub reason: String,
    pub flags: String,
    /// Started by a "Sleep" line: asleep, or a DarkWake entered while trying to sleep
    /// (lid closed, something kept it up) rather than a wake from sleep.
    pub via_sleep: bool,
    pub on_battery: Option<bool>,
    pub charge: Option<i64>,
    pub night: Option<i64>,
    pub cause: String,
    pub mah: Option<f64>,
    pub measured: bool,
}

#[derive(Debug, Clone)]
pub struct Bucket {
    pub cause: String,
    pub label: String,
    pub mah: i64,
    pub seconds: i64,
    /// Wakes counted (DarkWake/Wake lines), not intervals entered while trying to sleep.
    pub count: i64,
    pub estimated: bool,
    pub fix: String,
    pub undo: String,
}

#[derive(Debug, Clone)]
pub struct Night {
    pub start: i64,
    pub end: i64,
    pub start_pct: i64,
    pub end_pct: i64,
    pub drop_pct: i64,
    pub drop_mah: i64,
    pub max_mah: i64,
    /// true: drop from IOKit samples. false: coarse, Charge % x max_mah.
    pub measured: bool,
    pub clamshell: bool,
    pub wake_reason: String,
    /// "baseline" first, attributed buckets largest first, "unexplained" last.
    pub causes: Vec<Bucket>,
    /// A cause key, or "normal".
    pub top_cause: String,
    pub notes: Vec<String>,
    pub headline: String,
    pub timeline: Vec<(Option<i64>, String)>,
}

impl Night {
    pub fn bucket(&self, key: &str) -> Option<&Bucket> {
        self.causes.iter().find(|c| c.cause == key)
    }
    pub fn baseline_mah(&self) -> i64 {
        self.bucket("baseline").map_or(0, |c| c.mah)
    }
    pub fn unexplained_mah(&self) -> i64 {
        self.bucket("unexplained").map_or(0, |c| c.mah)
    }
    pub fn attributed(&self) -> impl Iterator<Item = &Bucket> {
        self.causes
            .iter()
            .filter(|c| c.cause != "baseline" && c.cause != "unexplained")
    }
    pub fn attributed_mah(&self) -> i64 {
        self.attributed().map(|c| c.mah).sum()
    }
    pub fn unexplained_pct(&self) -> f64 {
        if self.drop_mah == 0 {
            0.0
        } else {
            self.unexplained_mah() as f64 * 100.0 / self.drop_mah as f64
        }
    }
}

#[derive(Debug, Clone)]
pub struct Baseline {
    pub mah_per_h: f64,
    /// "clean" (median of measured clean sleep stretches), "fit", or "default".
    pub source: &'static str,
    pub clean_nights: usize,
    pub calibrated: bool,
    pub dark_mah_per_s: f64,
    /// "fit" or "default".
    pub dark_source: &'static str,
    pub nights_fitted: usize,
}

pub struct Ledger {
    pub intervals: Vec<Interval>,
    pub nights: Vec<Night>,
    pub baseline: Baseline,
    /// Night-shaped spans skipped because some of it was on AC power.
    pub skipped_ac: usize,
}

/// Cuts the state-change events (sorted by ts) into back-to-back intervals.
/// The last interval runs to `now`.
pub fn intervals(events: &[Event], now: i64) -> Vec<Interval> {
    let mut out: Vec<Interval> = Vec::new();
    for e in events {
        let state = match e.kind.as_str() {
            "Sleep" if e.message.starts_with("Entering DarkWake") => State::DarkWake,
            "Sleep" => State::Asleep,
            "DarkWake" => State::DarkWake,
            "Wake" => State::Awake,
            _ => continue,
        };
        if let Some(last) = out.last_mut() {
            last.end = e.ts;
        }
        out.push(Interval {
            start: e.ts,
            end: now.max(e.ts),
            state,
            reason: e.reason.clone(),
            flags: e.flags.clone(),
            via_sleep: e.kind == "Sleep",
            on_battery: e.on_battery,
            charge: e.charge_pct,
            night: None,
            cause: String::new(),
            mah: None,
            measured: false,
        });
    }
    out
}

/// A user FullWake: lid, power button, keyboard/trackpad.
pub fn is_user_wake(reason: &str) -> bool {
    reason.contains("UserActivity")
        || reason.contains("HID Activity")
        || reason
            .split_whitespace()
            .any(|t| t == "lid" || t == "pwrbtn")
}

/// (first interval, index of the waking interval) for each lid/idle sleep -> user wake span.
/// shortcut: a Mac that never slept at all (lid open, sleep blocked) has no night here.
fn night_spans(ivs: &[Interval]) -> Vec<(usize, usize)> {
    let (mut out, mut start) = (Vec::new(), None);
    for (i, iv) in ivs.iter().enumerate() {
        if iv.state == State::Awake {
            if (is_user_wake(&iv.reason) || iv.end - iv.start >= LONG_AWAKE_S)
                && let Some(s) = start.take()
            {
                out.push((s, i));
            }
        } else if start.is_none()
            && iv.via_sleep
            && matches!(iv.reason.as_str(), "Clamshell Sleep" | "Idle Sleep")
        {
            start = Some(i);
        }
    }
    out
}

/// The sample closest to `t` within `tol`, on battery.
fn near(samples: &[Sample], t: i64, tol: i64) -> Option<&Sample> {
    let i = samples.partition_point(|x| x.ts < t - tol);
    samples[i..]
        .iter()
        .take_while(|x| x.ts <= t + tol)
        .filter(|x| !x.on_ac)
        .min_by_key(|x| (x.ts - t).abs())
}

/// Two distinct samples bounding [a, b].
fn bounds(samples: &[Sample], a: i64, b: i64, tol: i64) -> Option<(&Sample, &Sample)> {
    let (x, y) = (near(samples, a, tol)?, near(samples, b, tol)?);
    (x.ts < y.ts).then_some((x, y))
}

struct NightDrop {
    mah: i64,
    start_pct: i64,
    end_pct: i64,
    max_mah: i64,
    measured: bool,
}

/// Measured from the samples bounding the night; else coarse from the log's Charge %.
fn night_drop(ivs: &[Interval], s: usize, e: usize, samples: &[Sample]) -> Option<NightDrop> {
    let (start, end) = (ivs[s].start, ivs[e].start);
    if let Some((a, b)) = bounds(samples, start, end, NIGHT_TOL) {
        return Some(NightDrop {
            mah: a.mah - b.mah,
            start_pct: a.pct,
            end_pct: b.pct,
            max_mah: b.max_mah,
            measured: true,
        });
    }
    // shortcut: with no sample at all there is no mAh scale, so the night is skipped.
    let (sp, ep) = (ivs[s].charge?, ivs[e].charge?);
    let max_mah = samples.iter().min_by_key(|x| (x.ts - end).abs())?.max_mah;
    let mah = ((sp - ep) as f64 * max_mah as f64 / 100.0).round() as i64;
    Some(NightDrop {
        mah,
        start_pct: sp,
        end_pct: ep,
        max_mah,
        measured: false,
    })
}

/// Seconds (asleep, dark-or-awake) in a night's intervals.
fn split(ivs: &[Interval]) -> (i64, i64) {
    ivs.iter().fold((0, 0), |(a, o), iv| {
        let d = iv.end - iv.start;
        if iv.state == State::Asleep {
            (a + d, o)
        } else {
            (a, o + d)
        }
    })
}

/// Least squares, no intercept: drop = a * hours_asleep + b * seconds_dark_or_awake.
/// Needs 3+ nights and a sane answer (a >= 0, b > 0). shortcut: no outlier rejection yet.
fn fit(rows: &[(f64, f64, f64)]) -> Option<(f64, f64)> {
    if rows.len() < 3 {
        return None;
    }
    let (mut hh, mut hs, mut ss, mut hd, mut sd) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for &(h, s, d) in rows {
        hh += h * h;
        hs += h * s;
        ss += s * s;
        hd += h * d;
        sd += s * d;
    }
    let det = hh * ss - hs * hs;
    if det <= 1e-9 * hh * ss {
        return None;
    }
    let (a, b) = ((hd * ss - sd * hs) / det, (hh * sd - hs * hd) / det);
    (a >= 0.0 && b > 0.0).then_some((a, b))
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    if v.len() % 2 == 1 {
        v[m]
    } else {
        (v[m - 1] + v[m]) / 2.0
    }
}

/// `events` and `asserts` sorted by ts, `samples` sorted by ts.
pub fn build(events: &[Event], samples: &[Sample], asserts: &[Assertion], now: i64) -> Ledger {
    let mut ivs = intervals(events, now);
    let sched: Vec<i64> = events
        .iter()
        .filter(|e| e.kind == "Sched")
        .map(|e| e.ts)
        .collect();
    let holds = causes::holds(asserts, now);

    let mut spans = Vec::new();
    let mut skipped_ac = 0;
    for (s, e) in night_spans(&ivs) {
        if ivs[e].start - ivs[s].start < MIN_NIGHT_S {
            continue;
        }
        if ivs[s..=e].iter().any(|i| i.on_battery != Some(true)) {
            skipped_ac += 1;
            continue;
        }
        if let Some(d) = night_drop(&ivs, s, e, samples) {
            spans.push((s, e, d));
        }
    }

    // Baseline: measured mAh/h of clean sleep stretches (one asleep interval >= 1 h, so no
    // DarkWake inside, with a sample at each end).
    let (mut rates, mut clean_nights) = (Vec::new(), 0);
    for (s, e, _) in &spans {
        let before = rates.len();
        for iv in ivs[*s..*e]
            .iter()
            .filter(|iv| iv.state == State::Asleep && iv.end - iv.start >= MIN_CLEAN_S)
        {
            if let Some((a, b)) = bounds(samples, iv.start, iv.end, STRETCH_TOL) {
                rates.push((a.mah - b.mah) as f64 * 3600.0 / (b.ts - a.ts) as f64);
            }
        }
        clean_nights += usize::from(rates.len() > before);
    }
    let rows: Vec<_> = spans
        .iter()
        .map(|(s, e, d)| {
            let (asleep, other) = split(&ivs[*s..*e]);
            (asleep as f64 / 3600.0, other as f64, d.mah as f64)
        })
        .collect();
    let fitted = fit(&rows);
    let (mah_per_h, source) = match fitted {
        _ if !rates.is_empty() => (median(rates), "clean"),
        Some((a, _)) => (a, "fit"),
        None => (DEFAULT_SLEEP_MAH_H, "default"),
    };
    let (dark_mah_per_s, dark_source) =
        fitted.map_or((DEFAULT_DARK_MAH_S, "default"), |(_, b)| (b, "fit"));
    let baseline = Baseline {
        mah_per_h,
        source,
        clean_nights,
        calibrated: clean_nights >= 2,
        dark_mah_per_s,
        dark_source,
        nights_fitted: if fitted.is_some() { rows.len() } else { 0 },
    };

    let nights = spans
        .into_iter()
        .map(|(s, e, d)| price(&mut ivs, s, e, d, &baseline, &sched, &holds, samples))
        .collect();
    Ledger {
        intervals: ivs,
        nights,
        baseline,
        skipped_ac,
    }
}

#[allow(clippy::too_many_arguments)]
fn price(
    ivs: &mut [Interval],
    s: usize,
    e: usize,
    d: NightDrop,
    base: &Baseline,
    sched: &[i64],
    holds: &[Hold],
    samples: &[Sample],
) -> Night {
    let (start, end) = (ivs[s].start, ivs[e].start);
    let clamshell = ivs[s].reason == "Clamshell Sleep";
    let wake_reason = ivs[e].reason.clone();
    let mut asleep_s = 0;
    // (cause, mAh, seconds, wakes, estimated)
    let mut acc: Vec<(String, f64, i64, i64, bool)> = Vec::new();
    for iv in &mut ivs[s..e] {
        let dur = iv.end - iv.start;
        iv.night = Some(start);
        if iv.state == State::Asleep {
            asleep_s += dur;
            iv.cause = "baseline".into();
            iv.mah = Some(dur as f64 / 3600.0 * base.mah_per_h);
            continue;
        }
        iv.cause = causes::classify(iv, sched, holds);
        let measured =
            bounds(samples, iv.start, iv.end, WAKE_TOL).map(|(a, b)| (a.mah - b.mah) as f64);
        iv.measured = measured.is_some();
        let mah = measured.unwrap_or(dur as f64 * base.dark_mah_per_s);
        iv.mah = Some(mah);
        let i = match acc.iter().position(|a| a.0 == iv.cause) {
            Some(i) => i,
            None => {
                acc.push((iv.cause.clone(), 0.0, 0, 0, false));
                acc.len() - 1
            }
        };
        let a = &mut acc[i];
        a.1 += mah;
        a.2 += dur;
        a.3 += i64::from(!iv.via_sleep);
        a.4 |= !iv.measured;
    }

    let bucket = |cause: &str, mah: i64, seconds: i64, count: i64, estimated: bool| {
        let (fix, undo) = causes::fix(cause);
        Bucket {
            cause: cause.into(),
            label: causes::label(cause, clamshell),
            mah,
            seconds,
            count,
            estimated,
            fix,
            undo,
        }
    };
    // Round each bucket once; unexplained takes the remainder, so the parts sum exactly.
    let mut attributed: Vec<Bucket> = acc
        .iter()
        .map(|a| bucket(&a.0, a.1.round() as i64, a.2, a.3, a.4))
        .collect();
    attributed.sort_by_key(|b| -b.mah);
    let base_mah = (asleep_s as f64 / 3600.0 * base.mah_per_h).round() as i64;
    let unexplained = d.mah - base_mah - attributed.iter().map(|b| b.mah).sum::<i64>();
    let top_cause = attributed
        .first()
        .filter(|b| d.mah > 0 && b.mah * 10 >= d.mah)
        .map_or("normal".to_string(), |b| b.cause.clone());

    // A holder that overlapped the night by 15+ min but kept it up < 5 min (e.g. only the
    // 1 s DarkWake at lid close): the OS slept anyway.
    let mut notes: Vec<String> = Vec::new();
    for h in holds
        .iter()
        .filter(|h| causes::overlap(h, start, end) >= 900)
    {
        let note = format!(
            "{} held {}, but {}.",
            h.process,
            h.kind,
            if clamshell {
                "lid-close sleep won"
            } else {
                "the Mac slept anyway"
            }
        );
        let kept_up = attributed
            .iter()
            .any(|b| b.cause == format!("assertion:{}", h.process) && b.seconds >= 300);
        if !kept_up && !notes.contains(&note) {
            notes.push(note);
        }
    }

    let mut causes = vec![bucket("baseline", base_mah, asleep_s, 0, true)];
    causes.extend(attributed);
    causes.push(bucket("unexplained", unexplained, 0, 0, !d.measured));
    let mut night = Night {
        start,
        end,
        start_pct: d.start_pct,
        end_pct: d.end_pct,
        drop_pct: d.start_pct - d.end_pct,
        drop_mah: d.mah,
        max_mah: d.max_mah,
        measured: d.measured,
        clamshell,
        wake_reason,
        causes,
        top_cause,
        notes,
        headline: String::new(),
        timeline: Vec::new(),
    };
    report::fill(&mut night, base);
    night
}

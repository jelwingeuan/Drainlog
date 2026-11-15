//! Shared by the ledger and replay tests: the "#SAMPLE <ts> mah= max= pct= ac=" lines that
//! stand in for IOKit battery samples, in the synthetic fixtures and in judge captures alike.

use drainlog::battery::Sample;
use drainlog::parse::parse_ts;

pub fn samples(text: &str) -> Vec<Sample> {
    text.lines()
        .filter_map(|l| l.strip_prefix("#SAMPLE "))
        .map(|rest| {
            let kv = |k: &str| -> i64 {
                rest[26..]
                    .split_whitespace()
                    .find_map(|p| p.strip_prefix(&format!("{k}=")))
                    .unwrap()
                    .parse()
                    .unwrap()
            };
            Sample {
                ts: parse_ts(&rest[..25]).unwrap(),
                mah: kv("mah"),
                max_mah: kv("max"),
                pct: kv("pct"),
                on_ac: kv("ac") != 0,
            }
        })
        .collect()
}

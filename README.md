# Drainlog

Answers one question: **why did my Mac's battery die overnight?**

Drainlog reads macOS's own power records (`pmset -g log`, `pmset -g assertions`, the `com.apple.powerd` log, and battery counters from IOKit). It builds a ledger of each night: time asleep, in DarkWake and awake, and how much charge each part cost. The morning report is a short incident timeline: what happened, why, and one fix.

## Honest numbers

macOS reports no per-app energy use while the lid is closed. Drainlog therefore labels every number:

- **Measured**: battery charge in mAh, read at sleep and at wake.
- **Estimated**: how that loss splits between baseline sleep drain and each wake, priced against your Mac's own measured baseline.
- **Unexplained**: whatever is left over. It is always shown, never hidden.

No root required. `powermetrics` is an optional extra.

## Status

Stage 1 in progress: morning report.

Tested on Apple silicon (M1). Intel builds compile but are **untested**. Intel Macs name battery keys and wake reasons differently, and handle standby and hibernate differently.

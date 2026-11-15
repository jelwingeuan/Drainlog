# What the logs on the dev Mac show (Nov 1–15, 2026)

Machine: MacBook Air M1, macOS 27.2. Sources: the 15 daily files in `/var/log/powermanagement` (read with `syslog -f`) and `pmset -g log`, which keeps only the last 7 days. The brief asked for 30 days, but macOS keeps only these 15 days of power history, so that is all there is to read. Drainlog's own database starts keeping history from now on.

## Counts (measured from the logs)

| What | Value |
|---|---|
| DarkWakes, 15 days | 1,222 (15–159 per day, median 63) |
| Woken by the Wi-Fi/Bluetooth chip (`smc … wifibt`) | 726 (59%) |
| Woken by the RTC for maintenance (`rtc/Maintenance`) | 494 (40%) |
| Other reasons (lid, SleepService) | 2 |
| DarkWakes with the Power Nap flag `P` | 1,222 (all of them) |
| DarkWakes promoted to a full wake by a notification | 549 |
| Time spent in DarkWake, last 7 days | 7.8 h (454 wakes, 62 s on average) |
| Lid-close sleeps | 624 |
| Sleep or wake events on battery | **0** |

Only one line in the whole period was recorded on battery: a status summary on Nov 13 at 21:06, while the Mac was awake.

## Does "wakes are what drain the battery" hold on this Mac?

**It can't be tested yet.** Every sleep in these 15 days was on AC power. While plugged in, the battery doesn't drain, so there is no capacity loss to divide between baseline sleep and wakes. The baseline sleep drain can't be measured for the same reason. That needs real nights on battery, with `drainlogd` running so it can read the mAh counter at sleep and at wake.

What the logs do show:

- If wakes matter at all, they matter a lot on this Mac. It spends about 1.1 h a day in DarkWake, and every wake has Power Nap on.
- Almost 6 in 10 wakes come from the Wi-Fi/Bluetooth chip, not from Power Nap's scheduled maintenance. Turning Power Nap off may therefore remove fewer wakes than you'd expect. This is a hypothesis to check on battery nights, not a finding.
- 549 DarkWakes became full wakes because of a notification. A full wake usually costs more than a DarkWake, so these are worth pricing first.

These numbers get rechecked against measured drain once the first battery nights exist (see `judge/README.md`).

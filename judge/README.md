# Drainlog judge

Stage 1 is done when Drainlog names the right top cause on at least 9 of 10 controlled
closed-lid nights, reported with the real numbers, misses included.

## Each night

1. **Before the night**, add a row to `nights.csv`: a night name (it becomes the capture
   folder, e.g. `2026-11-16-powernap`), the expected cause from the table below, and any
   notes. Write the expected cause down before the night, never after.
2. Start from the clean setup, then add that night's one cause.
3. Run on battery all night. A night that touches AC power is skipped. Keep `drainlogd`
   running, because it takes the battery samples: `cd core && cargo run --release --bin drainlogd`.
4. Close the lid for the night. It must be at least 2 h, and the night ends when you open the
   lid.
5. **In the morning**, open the lid and run `judge/capture.sh <night-name>` within a few hours.
   It keeps the last 24 h, which has to reach back to the lid close.
6. Undo the setup.
7. Run `cd core && cargo test --test replay`. It replays every captured night on its own,
   prints each headline, every miss and the score, and passes at 90% or better.

## The 10 nights

| Nights | Setup, the evening before | Expected cause | Undo, the morning after |
|---|---|---|---|
| 2 clean | Note `pmset -g custom` first. Then `sudo pmset -b powernap 0`, Wi-Fi and Bluetooth off (they share the chip behind `wifibt` wakes), no caffeinate, nothing of yours in `pmset -g sched` | `normal` | Wi-Fi and Bluetooth back on, and Power Nap back to the value you noted |
| 2 Power Nap | Clean setup, then `sudo pmset -b powernap 1` | `powernap` | `sudo pmset -b powernap 0` |
| 2 scheduled wake | Clean setup, then a wake every 30 min from 00:00 to 06:00 (use tomorrow's date): `for t in 00:00 00:30 01:00 01:30 02:00 02:30 03:00 03:30 04:00 04:30 05:00 05:30; do sudo pmset schedule wake "MM/DD/YY $t:00"; done` | `scheduled` | Wakes that fired are gone. For any still in `pmset -g sched`: `sudo pmset schedule cancel wake "MM/DD/YY HH:MM:SS"` |
| 2 Bluetooth | Clean setup, but Bluetooth on, with a paired device switched on near the Mac (a mouse or keyboard, or headphones out of their case) | `bluetooth` | Switch the device off and restore Wi-Fi |
| 2 caffeinate | Clean setup, then `caffeinate -i -t 7200 &` right before closing the lid | `assertion:caffeinate`, or `normal` (see below) | `killall caffeinate` (it also stops by itself after 2 h) |

The setup commands need `sudo`. Drainlog itself never does.

### Caffeinate on Apple silicon

On Apple silicon, closing the lid may override `caffeinate`. Before the first caffeinate night,
decide which one you expect with a dry run:

1. On battery, run `caffeinate -i -t 1800 &`.
2. Close the lid for 20 min, then open it.
3. Run `pmset -g log | grep "due to 'Clamshell Sleep'" | tail -2`.

If the `Entering DarkWake state` line at the lid close lasted only a second or so, and
`Entering Sleep state` came right after, macOS overrode caffeinate. Both caffeinate nights then
expect `normal`, plus the note `caffeinate held PreventUserIdleSystemSleep, but lid-close sleep
won.`, which replay prints after the headline. If the DarkWake lasted about as long as the
hold, expect `assertion:caffeinate`. Either way, write the expected cause into `nights.csv`
before the night.

## Captures

Each capture lands in `core/tests/fixtures/nights/<night-name>/` and holds about 4 MB:

- `pmset_log.txt`, the last 24 h of `pmset -g log`
- `assertions.txt` and `sched.txt`. The sched file also holds the planned wakes `drainlogd`
  recorded overnight.
- `powerd_log.txt`, the `com.apple.powerd` unified log. It is kept for reading and not replayed
  yet.
- `samples.txt`, the battery samples as `#SAMPLE` lines

Host and computer names, the hardware UUID and serial number, the home folder path, and pmset's
`UUID:` lines are scrubbed. App and process names stay, so read a capture before you push it
anywhere public.

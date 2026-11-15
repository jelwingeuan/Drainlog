#!/bin/bash
# judge/capture.sh <night-name>: run it the morning after a judge night, soon after waking.
# It keeps the last 24 h, which has to reach back to the lid close. No root needed; it reads
# the Drainlog DB and never writes it. Saves into core/tests/fixtures/nights/<night-name>/:
#   pmset_log.txt   pmset -g log, the lines from the last 24 h
#   assertions.txt  pmset -g assertions (its first line, the capture time, is replay's "now")
#   sched.txt       pmset -g sched, then the planned wakes drainlogd recorded in the window
#                   (a wake that already fired is gone from pmset -g sched by morning)
#   powerd_log.txt  log show for com.apple.powerd, last 24 h (for reading; not replayed yet)
#   samples.txt     drainlogd's battery samples from the window, as #SAMPLE lines
# Scrubbed: host and computer names, the Mac's hardware UUID and serial number, the home
# folder path, and the UUID header lines of pmset -g log.
set -euo pipefail

name=${1:-}
if [[ ! $name =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
    echo "usage: judge/capture.sh <night-name>  (letters, digits, . _ -)" >&2
    exit 2
fi
out="$(cd "$(dirname "$0")/.." && pwd)/core/tests/fixtures/nights/$name"
if [[ -e $out ]]; then
    echo "capture.sh: $out already exists; delete it first to capture again" >&2
    exit 1
fi
db=${DRAINLOG_DB:-$HOME/Library/Application Support/Drainlog/drainlog.db}
if [[ ! -f $db ]]; then
    echo "capture.sh: no Drainlog DB at $db; was drainlogd running?" >&2
    exit 1
fi

# Build the capture in a temp folder and move it into place at the end: all or nothing.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
now=$(date +%s)
since=$((now - 86400))
cut=$(date -r "$since" '+%Y-%m-%d %H:%M:%S')

# Timestamped lines from the cutoff on, plus the untimed lines that follow them (headers,
# DriverReason continuations, the trailing assertion dump).
/usr/bin/pmset -g log | awk -v cut="$cut" '
    /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9] / { keep = substr($0, 1, 19) >= cut }
    keep' > "$tmp/pmset_log.txt"
/usr/bin/pmset -g assertions > "$tmp/assertions.txt"
{
    /usr/bin/pmset -g sched
    echo "# Planned wakes drainlogd recorded in this window, from its DB:"
    sqlite3 "$db" "SELECT message FROM events WHERE kind = 'Sched' AND message LIKE '[%'
        AND ts BETWEEN $since AND $now ORDER BY ts"
} > "$tmp/sched.txt"
/usr/bin/log show --last 24h --predicate 'subsystem == "com.apple.powerd"' --style compact \
    > "$tmp/powerd_log.txt"
{
    echo "# REAL capture (judge/capture.sh), not synthetic: drainlogd's battery samples (the"
    echo "# samples table of drainlog.db) from the 24 h before $(date -r "$now" '+%Y-%m-%d %H:%M:%S %z')."
    # Local time with that moment's UTC offset, so the lines read like pmset's.
    sqlite3 "$db" "SELECT '#SAMPLE ' || datetime(ts, 'unixepoch', 'localtime')
        || printf(' %+03d%02d mah=%d max=%d pct=%d ac=%d', o / 3600, abs(o) % 3600 / 60,
                  mah, max_mah, pct, on_ac)
        FROM (SELECT *, CAST(round((julianday(ts, 'unixepoch', 'localtime')
                                    - julianday(ts, 'unixepoch')) * 86400) AS INTEGER) AS o
              FROM samples WHERE ts BETWEEN $since AND $now)
        ORDER BY ts"
} > "$tmp/samples.txt"

esc() { printf '%s' "$1" | sed 's/[][\/.*^$]/\\&/g'; }
ioreg_key() { ioreg -rd1 -c IOPlatformExpertDevice | awk -F'"' -v k="$1" '$2 == k { print $4 }'; }
script='s/^UUID: .*/UUID: 00000000-0000-0000-0000-000000000000/'
# The full hostname before the short one, which would leave its ".local" tail behind.
for v in "$(hostname)" "$(scutil --get ComputerName 2>/dev/null || true)" \
    "$(scutil --get LocalHostName 2>/dev/null || true)" "$(hostname -s)"; do
    if [[ -n $v ]]; then script+=$'\n'"s/$(esc "$v")/<host>/gI"; fi
done
for v in "$(ioreg_key IOPlatformUUID)" "$(ioreg_key IOPlatformSerialNumber)"; do
    if [[ -n $v ]]; then script+=$'\n'"s/$(esc "$v")/<machine-id>/g"; fi
done
script+=$'\n'"s/$(esc "$HOME")/~/g"
script+=$'\n'"s/$(esc "$(id -un)")/<user>/gI"
# Any remaining UUID (session, device, request IDs in the powerd log).
h='[0-9A-Fa-f]'
script+=$'\n'"s/$h\{8\}-$h\{4\}-$h\{4\}-$h\{4\}-$h\{12\}/<uuid>/g"
sed -i '' -e "$script" "$tmp"/*.txt

mkdir -p "$(dirname "$out")"
chmod 755 "$tmp"
mv "$tmp" "$out"
samples=$(grep -c '^#SAMPLE ' "$out/samples.txt" || true)
echo "Saved $name to $out ($samples battery samples)."
if [[ $samples -eq 0 ]]; then
    echo "capture.sh: no battery samples in the last 24 h, so replay finds no night." \
        "Was drainlogd running?" >&2
fi
echo "Next: undo tonight's setup (judge/README.md), then: cd core && cargo test --test replay"

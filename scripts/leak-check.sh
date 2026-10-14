#!/usr/bin/env bash
#
# Leak harness.
#
# Applies one wallpaper type repeatedly and reports how RSS and the total
# leaked-byte count move. Run against a freshly started daemon:
#
#   ./scripts/leak-check.sh
#
# The `leaks` count is only half the signal: an over-retain keeps an object
# reachable, so `leaks` reports nothing while RSS climbs anyway. Both numbers
# matter, which is why this reports both.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/release/livelywallpaper"
CYCLES="${1:-25}"

if [ ! -x "$BIN" ]; then
  echo "build first: cargo build --release"
  exit 1
fi

stop_daemon() { pkill -9 -f 'release/livelywallpaper' 2>/dev/null; sleep 1; }

rss_of()   { ps -o rss= -p "$1" 2>/dev/null | awk '{printf "%.1f", $1/1024}'; }
leaks_of() { leaks "$1" 2>/dev/null | grep 'total leaked bytes' | grep -oE '[0-9]+ total' | cut -d' ' -f1; }

# Pick one wallpaper id per renderer type from the daemon's own snapshot.
discover() {
  "$BIN" __ui '{"action":"state"}' 2>/dev/null | python3 -c "
import json, sys
try:
    d = json.load(sys.stdin)['state']['library']
except Exception:
    print('NONE|NONE|NONE|NONE'); raise SystemExit

def pick(t, builtin=None):
    for x in d:
        if x['wallpaperType'] == t and (x['isBuiltin'] if builtin is None else x['isBuiltin'] == builtin):
            return x['id']
    return 'NONE'

# Image: prefer a non-built-in, fall back to a built-in. The `or` compares
# against the literal, not truthiness -- 'NONE' is a non-empty string and would
# otherwise always win.
image = pick(5, False)
if image == 'NONE':
    image = pick(5, True)
print('|'.join([image, pick(1, False), pick(0, False), pick(6, True)]))
"
}

run_cycle() {
  local label="$1" id="$2"
  [ "$id" = "NONE" ] && { printf "  %-8s skipped (no wallpaper of this type)\n" "$label"; return; }

  stop_daemon
  "$BIN" daemon > "/tmp/leak-$label.log" 2>&1 &
  # Take the pid from the launch itself: pgrep can race with the previous
  # cycle's pkill and hand back the wrong (or a dying) process.
  local pid=$!
  sleep 3

  if ! kill -0 "$pid" 2>/dev/null; then
    printf "  %-8s daemon failed to start\n" "$label"
    return
  fi

  local before after before_leaks after_leaks
  before="$(rss_of "$pid")"
  before_leaks="$(leaks_of "$pid")"

  for _ in $(seq 1 "$CYCLES"); do
    "$BIN" __ui "{\"action\":\"apply\",\"id\":\"$id\",\"screen\":0}" >/dev/null 2>&1
    "$BIN" __ui '{"action":"closeWallpaper","monitor":0}' >/dev/null 2>&1
  done

  after="$(rss_of "$pid")"
  after_leaks="$(leaks_of "$pid")"

  printf "  %-8s RSS %6s -> %6s MB (%+.1f)   leaked %8s -> %8s bytes\n" \
    "$label" "$before" "$after" \
    "$(echo "$after - $before" | bc -l)" \
    "${before_leaks:-?}" "${after_leaks:-?}"

  stop_daemon
}

stop_daemon

# Deliberately does NOT clear the library: a diagnostic script must not destroy
# the user's wallpapers. Types with nothing installed are reported as skipped.
"$BIN" daemon > /tmp/leak-discover.log 2>&1 &
sleep 3
IFS='|' read -r IMG WEB VID SHD <<< "$(discover)"
stop_daemon

echo "Leak check — $CYCLES apply+close cycles per renderer"
echo
run_cycle IMAGE  "$IMG"
run_cycle WEB    "$WEB"
run_cycle VIDEO  "$VID"
run_cycle SHADER "$SHD"
echo
echo "A few MB of drift over tens of cycles is normal (caches, WebKit and"
echo "AVFoundation keep their own pools). Steady linear growth means a leak."
#!/usr/bin/env sh
# Runs wayinhibit against a real (headless) Wayland compositor and asserts
# its behavior end-to-end. Requires `sway` on PATH.
#
# Usage: scripts/e2e-test.sh [path-to-binary]

set -eu

bin="${1:-target/debug/wayinhibit}"
[ -x "$bin" ] || { echo "error: binary not found or not executable: $bin" >&2; exit 2; }

# True if the process exists and is not a zombie (kill -0 succeeds on zombies).
alive() { [ -r "/proc/$1/stat" ] && [ "$(awk '{print $3}' "/proc/$1/stat")" != Z ]; }

export XDG_RUNTIME_DIR=$(mktemp -d)
export WLR_BACKENDS="${WLR_BACKENDS:-headless}"
export WLR_RENDERER="${WLR_RENDERER:-pixman}"
export WLR_LIBINPUT_NO_DEVICES="${WLR_LIBINPUT_NO_DEVICES:-1}"

sway -c /dev/null 2>/dev/null &
SWAY_PID=$!
trap 'kill $SWAY_PID 2>/dev/null || true' EXIT

for i in $(seq 1 20); do
  SOCKET=$(find "$XDG_RUNTIME_DIR" -name 'wayland-*' ! -name '*.lock' 2>/dev/null | head -1)
  [ -n "$SOCKET" ] && break
  sleep 0.5
done
[ -n "${SOCKET:-}" ] || { echo "error: sway failed to start"; exit 1; }

export WAYLAND_DISPLAY=$(basename "$SOCKET")

"$bin" --quiet --timeout 1s
echo "ok: foreground mode with timeout"

"$bin" --quiet --timeout 5s -- true
echo "ok: child command mode"

"$bin" --quiet --timeout 5s -- sh -c 'exit 42' && STATUS=0 || STATUS=$?
[ $STATUS -eq 42 ] || { echo "error: expected exit code 42, got $STATUS"; exit 1; }
echo "ok: child exit code is propagated"

output=$("$bin" --timeout 1s 2>&1)
[ -n "$output" ] || { echo "error: expected output in non-quiet mode"; exit 1; }
output=$("$bin" --quiet --timeout 1s 2>&1)
[ -z "$output" ] || { echo "error: --quiet should suppress output, got: $output"; exit 1; }
echo "ok: --quiet suppresses all output"

"$bin" --timeout 30s &
WINH_PID=$!
sleep 0.3
kill -TERM $WINH_PID
wait $WINH_PID && STATUS=0 || STATUS=$?
[ $STATUS -eq 0 ] || { echo "error: expected exit 0 on SIGTERM, got $STATUS"; exit 1; }
echo "ok: SIGTERM stops cleanly with exit code 0"

WAYLAND_DISPLAY=nonexistent "$bin" --timeout 1s 2>/dev/null && STATUS=0 || STATUS=$?
[ $STATUS -eq 1 ] || { echo "error: expected exit 1 when no compositor, got $STATUS"; exit 1; }
echo "ok: exits with code 1 when compositor is unavailable"

PIDFILE=$(mktemp -u)
"$bin" --quiet --timeout 3s --pid-file "$PIDFILE" &
WINH_PID=$!
sleep 0.5
[ -f "$PIDFILE" ] || { echo "error: pid file was not created"; exit 1; }
grep -qE '^[0-9]+$' "$PIDFILE" || { echo "error: pid file does not contain a PID"; exit 1; }
wait $WINH_PID
[ ! -f "$PIDFILE" ] || { echo "error: pid file was not removed after exit"; exit 1; }
echo "ok: --pid-file created on start and removed on exit"

TOGGLE_PIDFILE=$(mktemp -u)
"$bin" --quiet --toggle --pid-file "$TOGGLE_PIDFILE" &
sleep 0.5
[ -f "$TOGGLE_PIDFILE" ] || { echo "error: --toggle did not start an instance"; exit 1; }
"$bin" --quiet --toggle --pid-file "$TOGGLE_PIDFILE"
sleep 0.3
[ ! -f "$TOGGLE_PIDFILE" ] || { echo "error: --toggle did not stop the running instance"; exit 1; }
echo "ok: --toggle starts and stops an instance"

# A stale pid file whose PID now belongs to an unrelated program must be left alone.
STALE_PIDFILE=$(mktemp -u)
sleep 30 &
OTHER_PID=$!
echo "$OTHER_PID" > "$STALE_PIDFILE"
"$bin" --quiet --toggle --pid-file "$STALE_PIDFILE" &
WINH_PID=$!
sleep 0.5
alive "$OTHER_PID" || { echo "error: --toggle killed an unrelated process"; exit 1; }
[ "$(cat "$STALE_PIDFILE")" = "$WINH_PID" ] || { echo "error: --toggle did not start a new instance over a stale pid file"; exit 1; }
kill -TERM $WINH_PID
wait $WINH_PID || true
kill "$OTHER_PID"
wait "$OTHER_PID" 2>/dev/null || true
echo "ok: --toggle ignores a stale pid file pointing at another program"

# An instance started through a symlink must still be recognised and stopped.
LINK_DIR=$(mktemp -d)
ln -s "$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")" "$LINK_DIR/wi"
LINK_PIDFILE=$(mktemp -u)
"$LINK_DIR/wi" --quiet --pid-file "$LINK_PIDFILE" &
LINK_PID=$!
sleep 0.5
[ -f "$LINK_PIDFILE" ] || { echo "error: symlinked instance did not start"; exit 1; }
# --timeout: if the instance is not recognised, toggle starts a new one that must not hang the test.
"$bin" --quiet --timeout 3s --toggle --pid-file "$LINK_PIDFILE"
sleep 0.3
if alive "$LINK_PID"; then
  kill -TERM "$LINK_PID"
  echo "error: --toggle did not stop a symlinked instance"
  exit 1
fi
wait "$LINK_PID" 2>/dev/null || true
rm -rf "$LINK_DIR"
echo "ok: --toggle stops an instance started through a symlink"

"$bin" --toggle 2>/dev/null && STATUS=0 || STATUS=$?
[ $STATUS -eq 2 ] || { echo "error: expected exit 2 for --toggle without --pid-file, got $STATUS"; exit 1; }
echo "ok: --toggle without --pid-file is rejected"

"$bin" --quiet -- sleep 30 &
WINH_PID=$!
sleep 0.3
kill -TERM $WINH_PID
wait $WINH_PID && STATUS=0 || STATUS=$?
[ $STATUS -eq 143 ] || { echo "error: expected exit 143 (child killed by SIGTERM), got $STATUS"; exit 1; }
echo "ok: SIGTERM in command mode terminates child and propagates exit code"

"$bin" --quiet -- sleep 30 &
WINH_PID=$!
sleep 0.3
kill -HUP $WINH_PID
wait $WINH_PID && STATUS=0 || STATUS=$?
[ $STATUS -eq 143 ] || { echo "error: expected exit 143 (child killed by SIGTERM) on SIGHUP, got $STATUS"; exit 1; }
echo "ok: SIGHUP triggers the same graceful shutdown as SIGTERM"

GRANDCHILD_PIDFILE=$(mktemp -u)
"$bin" --quiet -- sh -c "sleep 30 & echo \$! > $GRANDCHILD_PIDFILE; wait" &
WINH_PID=$!
sleep 0.5
GRANDCHILD_PID=$(cat "$GRANDCHILD_PIDFILE")
kill -0 "$GRANDCHILD_PID" || { echo "error: grandchild did not start"; exit 1; }
kill -TERM $WINH_PID
wait $WINH_PID 2>/dev/null || true
sleep 0.3
kill -0 "$GRANDCHILD_PID" 2>/dev/null && { echo "error: grandchild survived SIGTERM to wayinhibit"; exit 1; }
rm -f "$GRANDCHILD_PIDFILE"
echo "ok: SIGTERM reaches the wrapped command's own background children"

"$bin" --quiet -- sh -c 'kill -9 $$' && STATUS=0 || STATUS=$?
[ $STATUS -eq 137 ] || { echo "error: expected exit 137 (128+SIGKILL), got $STATUS"; exit 1; }
echo "ok: signal-killed child propagates 128+signal exit code"

# From here on the compositor is gone: this must be the last test that needs one.
"$bin" --quiet -- sh -c 'sleep 2; exit 7' &
WINH_PID=$!
sleep 0.3
kill -9 $SWAY_PID
wait $WINH_PID && STATUS=0 || STATUS=$?
[ $STATUS -eq 7 ] || { echo "error: expected exit 7 after compositor crash, got $STATUS"; exit 1; }
echo "ok: child exit code still propagates after the Wayland connection is lost mid-run"

"$bin" --help > /dev/null
echo "ok: --help exits 0"

"$bin" --version | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+'
echo "ok: --version prints version number"

"$bin" --invalid-flag 2>/dev/null && STATUS=0 || STATUS=$?
[ $STATUS -eq 2 ] || { echo "error: expected exit 2 for invalid args, got $STATUS"; exit 1; }
echo "ok: invalid args exits with code 2"

INHIBIT_FILE=$(mktemp)
RELEASE_FILE=$(mktemp)
rm "$INHIBIT_FILE" "$RELEASE_FILE"
"$bin" --dry-run --timeout 1s \
  --on-inhibit "touch $INHIBIT_FILE" \
  --on-release "touch $RELEASE_FILE"
[ -f "$INHIBIT_FILE" ] || { echo "error: --dry-run did not fire --on-inhibit"; exit 1; }
[ -f "$RELEASE_FILE" ] || { echo "error: --dry-run did not fire --on-release"; exit 1; }
rm -f "$INHIBIT_FILE" "$RELEASE_FILE"
echo "ok: --dry-run fires hooks without a compositor"

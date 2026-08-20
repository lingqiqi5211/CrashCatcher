#!/system/bin/sh

MODDIR="${0%/*}"
. "$MODDIR/service.d/common.sh"
. "$MODDIR/service.d/manager_pin.sh"

mkdir -p "$STATE_DIR" "$RUNTIME_DIR" "$STATE_DIR/logs"
chmod 0700 "$STATE_DIR" "$RUNTIME_DIR" "$STATE_DIR/logs"

# Preserve the user's pre-module setting exactly once so uninstall can put it
# back instead of blindly forcing Android's default.
if [ ! -f "$STATE_DIR/takeover.previous" ]; then
  previous_takeover="$(settings get global hide_error_dialogs 2>/dev/null)"
  atomic_write "$STATE_DIR/takeover.previous" "$previous_takeover"
fi

pin="$MODDIR/config/manager_signing_cert.sha256"
if ! validate_manager_pin "$pin"; then
  set_module_status "❌ 已停止"
  service_log "invalid manager signing pin"
  exit 1
fi

case "$(getprop ro.product.cpu.abi)" in
  arm64-v8a) ABI="arm64-v8a" ;;
  armeabi-v7a|armeabi) ABI="armeabi-v7a" ;;
  x86_64) ABI="x86_64" ;;
  *)
    set_module_status "❌ 已停止"
    service_log "unsupported abi: $(getprop ro.product.cpu.abi)"
    exit 1
    ;;
esac

daemon="$MODDIR/bin/$ABI/catcherd"
if [ ! -x "$daemon" ]; then
  set_module_status "❌ 已停止"
  # A release build ships one ABI per package, so this is what a device that got the wrong
  # zip sees — and the only place that says so.
  service_log "no daemon binary for $ABI at $daemon"
  exit 1
fi

(
  while [ "$(getprop sys.boot_completed)" != "1" ]; do sleep 2; done
  boot_id="$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
  [ -n "$boot_id" ] && atomic_write "$STATE_DIR/.boot_ok" "$boot_id"
) &

# Keep the previous boot's logs. The crash being chased is often the one that made the device
# reboot, and its evidence is in the session that ended — which the daemon would otherwise
# append straight over.
#
# Keyed on boot_id so re-running this script inside one boot does not throw away the logs of the
# session it is part of.
boot_id="$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
logged_boot="$(cat "$STATE_DIR/logs/.boot_id" 2>/dev/null)"
if [ -n "$boot_id" ] && [ "$boot_id" != "$logged_boot" ]; then
  rm -rf "$STATE_DIR/logs/old"
  mkdir -p "$STATE_DIR/logs/old"
  for previous in "$STATE_DIR/logs"/*.log "$STATE_DIR/logs"/*.log.*; do
    [ -f "$previous" ] && mv -f "$previous" "$STATE_DIR/logs/old/"
  done
  atomic_write "$STATE_DIR/logs/.boot_id" "$boot_id"
fi

service_log "starting abi=$ABI daemon=$daemon"

# One launcher at a time. Two of them share `daemon.pid` and the ready file, so each takes the
# other's daemon for its own: one starts a daemon, the other sees the pid it did not write die,
# restarts, hits AddrInUse against the survivor, and both spend their attempts fighting. First
# one wins; a second exits rather than joining in.
#
# Checked against /proc rather than trusted from the file, since pids get reused.
service_pid_file="$RUNTIME_DIR/service.pid"
running="$(cat "$service_pid_file" 2>/dev/null)"
case "$running" in
  '' | *[!0-9]*) running='' ;;
esac
if [ -n "$running" ] && [ "$running" != "$$" ] &&
  grep -q service.sh "/proc/$running/cmdline" 2>/dev/null; then
  service_log "another launcher is running (pid=$running); exiting"
  exit 0
fi
atomic_write "$service_pid_file" "$$"

# A previous instance may still be alive: updating the module does not stop the running
# daemon, and the abstract socket belongs to the process holding it. The new one then fails
# with AddrInUse through every backoff step until the attempts run out — leaving the module
# saying "运行中" while nothing is listening, which is exactly what a user sees as
# "未连接守护进程" right after flashing.
#
# Matched against /proc/<pid>/cmdline rather than trusted from the file: pids are reused, and
# `pkill -f catcherd` is worse still — the pattern matches the very `su -c` that ran it.
stop_stale_daemon() {
  stale="$(cat "$RUNTIME_DIR/daemon.pid" 2>/dev/null)"
  case "$stale" in
    '' | *[!0-9]*) return 0 ;;
  esac
  grep -q catcherd "/proc/$stale/cmdline" 2>/dev/null || return 0

  service_log "stopping stale daemon pid=$stale"
  kill "$stale" 2>/dev/null
  waited=0
  while kill -0 "$stale" 2>/dev/null && [ "$waited" -lt 10 ]; do
    sleep 1
    waited=$((waited + 1))
  done
  if kill -0 "$stale" 2>/dev/null; then
    service_log "stale daemon pid=$stale ignored SIGTERM; killing"
    kill -9 "$stale" 2>/dev/null
    sleep 1
  fi
}

stop_stale_daemon

attempt=0
delay=1
while [ "$attempt" -lt 8 ]; do
  rm -f "$RUNTIME_DIR/ready"
  # stderr only. The daemon writes daemon.log itself so it can rotate it, and a redirect here
  # would be a second writer holding a descriptor across the rename. What is left for this file
  # is what never reaches tracing: a panic, or a start that failed before logging existed.
  "$daemon" \
    --state-dir "$STATE_DIR" \
    --module-dir "$MODDIR" \
    --manager-pin "$pin" \
    >/dev/null 2>> "$STATE_DIR/logs/daemon.stderr.log" &
  pid=$!
  atomic_write "$RUNTIME_DIR/daemon.pid" "$pid"
  service_log "daemon started pid=$pid attempt=$((attempt + 1))"

  waited=0
  while kill -0 "$pid" 2>/dev/null && [ ! -f "$RUNTIME_DIR/ready" ] && [ "$waited" -lt 20 ]; do
    sleep 1
    waited=$((waited + 1))
  done
  if [ -f "$RUNTIME_DIR/ready" ]; then
    set_module_status "✅ 运行中"
    service_log "daemon ready pid=$pid after ${waited}s"
  else
    service_log "daemon pid=$pid did not report ready within ${waited}s"
  fi

  wait "$pid"
  code=$?
  rm -f "$RUNTIME_DIR/ready" "$RUNTIME_DIR/daemon.pid"
  attempt=$((attempt + 1))
  service_log "daemon exited code=$code attempt=$attempt"
  [ "$attempt" -ge 8 ] && break
  # Said out loud while it is true. The status used to keep claiming 运行中 through every
  # restart, so a module list showing a healthy daemon and a manager showing "未连接" were
  # the same moment described two ways.
  set_module_status "⚠️ 重启中"
  sleep "$delay"
  delay=$((delay * 2))
  [ "$delay" -gt 60 ] && delay=60
done

set_module_status "❌ 已停止"
service_log "giving up after $attempt attempts"
exit 1

#!/system/bin/sh

MODDIR="${0%/*}"
. "$MODDIR/service.d/common.sh"

mkdir -p "$STATE_DIR" "$RUNTIME_DIR"
chmod 0700 "$STATE_DIR" "$RUNTIME_DIR"

boot_id="$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
[ -n "$boot_id" ] || exit 0

previous="$(cat "$STATE_DIR/.boot_pending" 2>/dev/null)"
completed="$(cat "$STATE_DIR/.boot_ok" 2>/dev/null)"

strikes="$(cat "$STATE_DIR/.boot_strikes" 2>/dev/null)"
case "$strikes" in
  '' | *[!0-9]*) strikes=0 ;;
esac

# The previous boot wrote a pending id and never reached sys.boot_completed. That is the
# signature of a module that hangs the device — but it is also what a user shutting down
# during the boot animation leaves behind, so one occurrence is not evidence. Two boots in a
# row are.
if [ -n "$previous" ] && [ "$previous" != "$boot_id" ] && [ "$completed" != "$previous" ]; then
  strikes=$((strikes + 1))
  if [ "$strikes" -ge 2 ]; then
    # Wipe the guard's own state on the way out. Leaving it behind is what turned a single
    # false positive into a module that could never be switched back on: `previous` froze at
    # the boot that tripped it, `completed` could never catch up to it again, and every later
    # boot re-derived the same verdict from the same stale pair — re-enabling by hand only
    # bought one boot before this script wrote `disable` straight back.
    rm -f "$STATE_DIR/.boot_pending" "$STATE_DIR/.boot_ok" "$STATE_DIR/.boot_strikes"
    touch "$MODDIR/disable"
    set_module_status "🚫 已禁用"
    service_log "boot guard tripped twice; disabling module"
    exit 0
  fi
  service_log "boot $previous never completed (strike $strikes)"
  atomic_write "$STATE_DIR/.boot_strikes" "$strikes"
else
  rm -f "$STATE_DIR/.boot_strikes"
fi

atomic_write "$STATE_DIR/.boot_pending" "$boot_id"
rm -f "$RUNTIME_DIR/ready" "$RUNTIME_DIR/daemon.pid"

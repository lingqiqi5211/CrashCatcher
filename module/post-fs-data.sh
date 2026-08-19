#!/system/bin/sh

MODDIR="${0%/*}"
. "$MODDIR/service.d/common.sh"

mkdir -p "$STATE_DIR" "$RUNTIME_DIR"
chmod 0700 "$STATE_DIR" "$RUNTIME_DIR"

boot_id="$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
[ -n "$boot_id" ] || exit 0

previous="$(cat "$STATE_DIR/.boot_pending" 2>/dev/null)"
completed="$(cat "$STATE_DIR/.boot_ok" 2>/dev/null)"

if [ -n "$previous" ] && [ "$previous" != "$boot_id" ] && [ "$completed" != "$previous" ]; then
  touch "$MODDIR/disable"
  set_module_status "🚫 已禁用"
  exit 0
fi

atomic_write "$STATE_DIR/.boot_pending" "$boot_id"
rm -f "$RUNTIME_DIR/ready" "$RUNTIME_DIR/daemon.pid"

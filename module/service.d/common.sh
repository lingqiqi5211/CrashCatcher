#!/system/bin/sh

# Entry scripts define MODDIR before sourcing this file. Keeping that value is
# important: while a file is sourced, $0 still points at the entry script rather
# than at service.d/common.sh.
[ -n "$MODDIR" ] || MODDIR="${0%/*}"
STATE_DIR="/data/adb/crash.catcher"
RUNTIME_DIR="$STATE_DIR/runtime"
MODULE_PROP="$MODDIR/module.prop"

set_module_status() {
  status="$1"
  [ -f "$MODULE_PROP" ] || return 0
  temp="$MODULE_PROP.tmp.$$"
  awk -v status="$status" '
    /^description=/ {
      body = substr($0, index($0, "]") + 2)
      print "description=[ " status " ] " body
      next
    }
    { print }
  ' "$MODULE_PROP" > "$temp" && chmod 0644 "$temp" && mv -f "$temp" "$MODULE_PROP"
}

# The launcher's own log, which the diagnostics page reads alongside the daemon's.
#
# It is the only record of what happened *around* the daemon rather than inside it: which ABI
# was chosen, a stale instance being cleared, an exit code, a restart that never became ready.
# A daemon that dies before it can log anything leaves nothing anywhere else.
service_log() {
  mkdir -p "$STATE_DIR/logs" 2>/dev/null
  printf '%s %s\n' "$(date '+%F %T')" "$1" >> "$STATE_DIR/logs/service.log"
}

atomic_write() {
  target="$1"
  value="$2"
  temp="$target.tmp.$$"
  printf '%s\n' "$value" > "$temp" && chmod 0600 "$temp" && mv -f "$temp" "$target"
}

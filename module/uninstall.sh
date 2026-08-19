#!/system/bin/sh

STATE_DIR="/data/adb/crash.catcher"
pid_file="$STATE_DIR/runtime/daemon.pid"
if [ -f "$pid_file" ]; then
  pid="$(cat "$pid_file" 2>/dev/null)"
  case "$pid" in
    ''|*[!0-9]*) ;;
    *) kill "$pid" 2>/dev/null ;;
  esac
fi

previous="$STATE_DIR/takeover.previous"
if [ -f "$previous" ]; then
  value="$(cat "$previous" 2>/dev/null)"
  case "$value" in
    ''|null) settings delete global hide_error_dialogs >/dev/null 2>&1 ;;
    *) settings put global hide_error_dialogs "$value" >/dev/null 2>&1 ;;
  esac
fi

# History/config are intentionally preserved across reinstall. The user can
# remove /data/adb/crash.catcher manually or from the manager's delete action.
exit 0

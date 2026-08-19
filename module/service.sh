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
  printf '%s invalid manager signing pin\n' "$(date '+%F %T')" >> "$STATE_DIR/logs/service.log"
  exit 1
fi

case "$(getprop ro.product.cpu.abi)" in
  arm64-v8a) ABI="arm64-v8a" ;;
  armeabi-v7a|armeabi) ABI="armeabi-v7a" ;;
  x86_64) ABI="x86_64" ;;
  *) set_module_status "❌ 已停止"; exit 1 ;;
esac

daemon="$MODDIR/bin/$ABI/catcherd"
if [ ! -x "$daemon" ]; then
  set_module_status "❌ 已停止"
  exit 1
fi

(
  while [ "$(getprop sys.boot_completed)" != "1" ]; do sleep 2; done
  boot_id="$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
  [ -n "$boot_id" ] && atomic_write "$STATE_DIR/.boot_ok" "$boot_id"
) &

attempt=0
delay=1
while [ "$attempt" -lt 8 ]; do
  rm -f "$RUNTIME_DIR/ready"
  "$daemon" \
    --state-dir "$STATE_DIR" \
    --module-dir "$MODDIR" \
    --manager-pin "$pin" \
    >> "$STATE_DIR/logs/daemon.log" 2>&1 &
  pid=$!
  atomic_write "$RUNTIME_DIR/daemon.pid" "$pid"

  waited=0
  while kill -0 "$pid" 2>/dev/null && [ ! -f "$RUNTIME_DIR/ready" ] && [ "$waited" -lt 20 ]; do
    sleep 1
    waited=$((waited + 1))
  done
  if [ -f "$RUNTIME_DIR/ready" ]; then
    set_module_status "✅ 运行中"
  fi

  wait "$pid"
  code=$?
  rm -f "$RUNTIME_DIR/ready" "$RUNTIME_DIR/daemon.pid"
  attempt=$((attempt + 1))
  printf '%s daemon exited code=%s attempt=%s\n' "$(date '+%F %T')" "$code" "$attempt" >> "$STATE_DIR/logs/service.log"
  [ "$attempt" -ge 8 ] && break
  sleep "$delay"
  delay=$((delay * 2))
  [ "$delay" -gt 60 ] && delay=60
done

set_module_status "❌ 已停止"
exit 1

#!/system/bin/sh

SKIPUNZIP=0

ui_print "- Installing CrashCatcher"

case "$ARCH" in
  arm64) ABI="arm64-v8a" ;;
  arm) ABI="armeabi-v7a" ;;
  x64) ABI="x86_64" ;;
  *) abort "Unsupported architecture: $ARCH" ;;
esac

DAEMON="$MODPATH/bin/$ABI/catcherd"
[ -f "$DAEMON" ] || abort "Missing daemon for $ABI"
[ -f "$MODPATH/dex/cch_bridge.dex" ] || abort "Missing cch_bridge.dex"
[ -f "$MODPATH/config/manager_signing_cert.sha256" ] || abort "Missing manager signing pin"

set_perm_recursive "$MODPATH" 0 0 0755 0644
set_perm "$DAEMON" 0 0 0755
set_perm "$MODPATH/customize.sh" 0 0 0755
set_perm "$MODPATH/post-fs-data.sh" 0 0 0755
set_perm "$MODPATH/service.sh" 0 0 0755
set_perm "$MODPATH/uninstall.sh" 0 0 0755
set_perm_recursive "$MODPATH/service.d" 0 0 0755 0755
set_perm "$MODPATH/config/manager_signing_cert.sha256" 0 0 0600

ui_print "- Installed zero-injection module for $ABI"

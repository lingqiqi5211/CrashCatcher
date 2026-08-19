#!/system/bin/sh

# Read-only diagnostic used by catcherd support bundles. Collection itself is
# native and does not depend on this script.
collector_diagnostics() {
  logcat -g -b events -b crash 2>&1
  for path in /data/system/dropbox /data/tombstones /data/anr; do
    if [ -r "$path" ] && [ -x "$path" ]; then
      printf '%s readable\n' "$path"
    else
      printf '%s unavailable\n' "$path"
    fi
  done
}

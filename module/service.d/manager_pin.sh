#!/system/bin/sh

validate_manager_pin() {
  pin_file="$1"
  [ -f "$pin_file" ] && [ ! -L "$pin_file" ] || return 1
  owner="$(stat -c '%u:%g' "$pin_file" 2>/dev/null)"
  [ "$owner" = "0:0" ] || return 1
  mode="$(stat -c '%a' "$pin_file" 2>/dev/null)"
  case "$mode" in
    600|400) ;;
    *) return 1 ;;
  esac
  value="$(tr -d '\r\n ' < "$pin_file")"
  [ "${#value}" -eq 64 ] || return 1
  case "$value" in
    *[!0-9a-fA-F]*|0000000000000000000000000000000000000000000000000000000000000000)
      return 1
      ;;
  esac
  return 0
}

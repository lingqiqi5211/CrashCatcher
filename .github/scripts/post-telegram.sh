#!/usr/bin/env bash
# Posts whatever this build produced to the release channel, with the changelog.
#
# Only the artifacts this build actually made are sent: a Kotlin-only commit ships an APK
# alone, a daemon-only commit ships a module alone. The channel therefore shows what changed
# rather than a fixed pair every time.
#
# The caption goes on the *last* document. Telegram renders a media group's caption under the
# final item, so attaching it to the first leaves the text stranded above the files.
set -euo pipefail

if [ -z "${TOKEN:-}" ] || [ -z "${CHAT:-}" ]; then
  echo "::warning::Telegram credentials are not set; skipping the channel post."
  exit 0
fi

# Module first, APK last: that is the install order, and it puts the changelog under the file
# a reader is most likely to tap.
mapfile -t files < <(
  { ls -1 dist/*.zip 2>/dev/null || true; ls -1 dist/*.apk 2>/dev/null || true; }
)
if [ "${#files[@]}" -eq 0 ]; then
  echo "::warning::nothing in dist/; skipping the channel post."
  exit 0
fi

escape() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }

# What to call this build. A tag is its own name; anything else is the version plus the commit
# count and short hash, which is what the module reports in the root manager — so a message in
# the channel and a line in the module list can be matched up.
if [ "${IS_TAG:-false}" = "true" ]; then
  label="${GITHUB_REF_NAME:-$SUFFIX}"
else
  label="$(sed -n 's/^version=//p' version.properties)-r$(git rev-list --count HEAD)-g$(git rev-parse --short=7 HEAD)"
fi

# What to summarise: a tag compares against the previous tag, a push against what it replaced.
# `before` is all zeroes for a new branch and can point at a commit that no longer exists, so
# both cases fall back to the tip commit alone rather than failing the build over a changelog.
range=""
if [ "${IS_TAG:-false}" = "true" ]; then
  previous=$(git describe --tags --abbrev=0 "${SHA}^" 2>/dev/null || true)
  [ -n "$previous" ] && range="$previous..$SHA"
elif [ -n "${BEFORE:-}" ] && [ "$BEFORE" != "0000000000000000000000000000000000000000" ] \
  && git cat-file -e "$BEFORE^{commit}" 2>/dev/null; then
  range="$BEFORE..$SHA"
fi

if [ -n "$range" ]; then
  log=$(git log --no-merges --format='%s' "$range" | head -20 | sed 's/^/• /')
  compare="https://github.com/${REPOSITORY}/compare/${range}"
else
  log=$(git log -1 --format='%s' | sed 's/^/• /')
  compare="https://github.com/${REPOSITORY}/commit/${SHA}"
fi

# A release says what changed in the words of whoever cut it: the annotated tag's body. Subject
# lines are for people reading the code, and this message goes to people installing it. Only a
# tag has one, so every other build keeps the commit list.
if [ "${IS_TAG:-false}" = "true" ]; then
  notes=$(git tag -l --format='%(contents:body)' "$GITHUB_REF_NAME" 2>/dev/null || true)
  [ -n "$notes" ] && log="$notes"
fi

caption=$(
  printf '<b>CrashCatcher %s</b>\n\n' "$(printf '%s' "$label" | escape)"
  printf '%s\n\n' "$(printf '%s' "$log" | escape)"
  printf '<a href="%s">%s</a>' "$compare" "$([ "${IS_TAG:-false}" = "true" ] && echo '更新日志' || echo '本次改动')"
)
# Telegram caps a caption at 1024 characters and rejects the whole request when it is longer.
#
# `head -c` counts bytes, so cutting Chinese mid-character leaves invalid UTF-8 that Telegram
# rejects outright. `iconv -c` drops the partial sequence at the end — and exits non-zero for
# having done so, which under `set -e` would fail the build instead of shortening a caption.
if [ "$(printf '%s' "$caption" | wc -c)" -gt 1000 ]; then
  caption=$(printf '%s' "$caption" | head -c 1000 | iconv -c -f utf-8 -t utf-8 2>/dev/null || true)
fi

api="https://api.telegram.org/bot${TOKEN}"

# `--form-string` for every text field, `-F` only where `@` has to mean "upload this file".
# `-F name=value` also treats a leading `<` as "read the value from this file", and the caption
# is HTML — so `<b>CrashCatcher …` was taken as a filename and the whole post died on
# `curl: (26) Failed to open/read local data`, forty milliseconds in, taking the release step
# with it.
if [ "${#files[@]}" -eq 1 ]; then
  curl -sS --fail-with-body -X POST "$api/sendDocument" \
    --form-string "chat_id=$CHAT" \
    --form-string "parse_mode=HTML" \
    --form-string "caption=$caption" \
    -F "document=@${files[0]}" > /dev/null
  echo "sent ${files[0]}"
  exit 0
fi

# Build the media array with jq so a filename or caption can never break the JSON.
media=$(
  for index in "${!files[@]}"; do
    if [ "$index" -eq $(( ${#files[@]} - 1 )) ]; then
      jq -n --arg attach "attach://file$index" --arg caption "$caption" \
        '{type: "document", media: $attach, caption: $caption, parse_mode: "HTML"}'
    else
      jq -n --arg attach "attach://file$index" '{type: "document", media: $attach}'
    fi
  done | jq -sc .
)

attachments=()
for index in "${!files[@]}"; do
  attachments+=(-F "file$index=@${files[$index]}")
done

curl -sS --fail-with-body -X POST "$api/sendMediaGroup" \
  --form-string "chat_id=$CHAT" \
  --form-string "media=$media" \
  "${attachments[@]}" > /dev/null
echo "sent ${files[*]}"

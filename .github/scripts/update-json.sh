#!/usr/bin/env bash
# Points the root manager's update check at the release that was just published.
#
# `updateJson` in module.prop is polled by Magisk / KernelSU / APatch, which compare its
# versionCode against the installed one and offer the zip. It therefore must describe a
# release people should install — moving it on every push would offer everyone whatever
# landed on main minutes ago, so this only runs for a tag.
#
# https://topjohnwu.github.io/Magisk/guides.html#moduleprop
set -euo pipefail

tag="${GITHUB_REF_NAME:?}"
version="${tag#v}"
code=$(git rev-list --count "$tag")
zip_name=$(basename "$(ls -1 dist/CrashCatcher-module-*.zip | head -1)")
zip_url="https://github.com/${GITHUB_REPOSITORY}/releases/download/${tag}/${zip_name}"

# Written on main, not on the tag. A tag build is detached, and the tag is not necessarily
# main's tip — pushing HEAD there would be rejected as a non-fast-forward the moment anything
# landed after the tag was cut. dist/ is untracked, so switching branches leaves it alone.
git fetch --quiet origin main
git checkout --quiet -B main origin/main

jq -n \
  --arg version "$version" \
  --argjson versionCode "$code" \
  --arg zipUrl "$zip_url" \
  --arg changelog "https://raw.githubusercontent.com/${GITHUB_REPOSITORY}/main/CHANGELOG.md" \
  '{version: $version, versionCode: $versionCode, zipUrl: $zipUrl, changelog: $changelog}' \
  > update.json

# The changelog that field points at has to exist and has to be plain text the module viewer
# can render — release notes live in the API, not at a URL.
previous=$(git describe --tags --abbrev=0 "${tag}^" 2>/dev/null || true)
{
  printf '# %s\n\n' "$version"
  if [ -n "$previous" ]; then
    git log --no-merges --format='- %s' "${previous}..${tag}"
  else
    git log --no-merges --format='- %s' "$tag" | head -50
  fi
  printf '\n'
  if [ -f CHANGELOG.md ]; then
    cat CHANGELOG.md
  fi
} > CHANGELOG.next
mv CHANGELOG.next CHANGELOG.md

git add update.json CHANGELOG.md
if git diff --cached --quiet; then
  echo "update.json and CHANGELOG.md already current"
  exit 0
fi

git config user.name '柒柒喵'
git config user.email 'lingqiqi233@gmail.com'
git commit --quiet -m "chore: update.json and changelog for ${tag}"
git push --quiet origin main
echo "update.json now points at $zip_url"

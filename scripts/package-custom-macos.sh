#!/usr/bin/env bash
# Run from the repository root after a signed Tauri macOS build.
set -euo pipefail

tag=${1:?Expected custom-vMAJOR.MINOR.PATCH}
bundle=${2:?Expected Tauri macOS bundle directory}
out=${3:?Expected new output directory}
[[ "$tag" =~ ^custom-v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]
version=${tag#custom-v}
repo=AyasukaRan/cc-switch
app="$bundle/CC Switch.app"
archive="$bundle/CC Switch.app.tar.gz"
base="CC-Switch-$tag-macOS-arm64"

codesign --verify --deep --strict "$app"
[[ "$(lipo -archs "$app/Contents/MacOS/cc-switch")" == arm64 ]]
[[ "$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")" == "$version" ]]
endpoint="https://github.com/$repo/releases/latest/download/latest.json"
jq -e --arg endpoint "$endpoint" '.plugins.updater.endpoints == [$endpoint]' src-tauri/tauri.conf.json >/dev/null
pubkey=$(jq -er '.plugins.updater.pubkey | @base64d | split("\n")[1]' src-tauri/tauri.conf.json)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
base64 -D -i "$archive.sig" -o "$tmp/updater.minisig"
minisign -Vm "$archive" -x "$tmp/updater.minisig" -P "$pubkey"
awk -F '\t' -v version="$version" '/^trusted comment: / {for (i=1; i<=NF; i++) if ($i=="version:" version) valid=1} END {exit !valid}' "$tmp/updater.minisig"

mkdir "$out"
ditto -c -k --sequesterRsrc --keepParent "$app" "$out/$base.zip"
cp "$archive" "$out/$base.app.tar.gz"
cp "$archive.sig" "$out/$base.app.tar.gz.sig"
jq -n --arg version "$version" --arg tag "$tag" --arg date "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" \
  --arg signature "$(cat "$archive.sig")" --arg url "https://github.com/$repo/releases/download/$tag/$base.app.tar.gz" \
  '{version: $version, notes: ("CC Switch Custom " + $tag), pub_date: $date,
    platforms: {"darwin-aarch64": {signature: $signature, url: $url}}}' > "$out/latest.json"

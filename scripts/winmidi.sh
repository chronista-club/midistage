#!/usr/bin/env bash
# Windows.Devices.Midi2（Windows MIDI Services の API）の取得と binding の生成。design 07。
#
#   scripts/winmidi.sh fetch      # NuGet パッケージを取って .cache/winmidi/ に winmd / dll / pri を置く
#   scripts/winmidi.sh bindings   # winmd から crates/midistaged/src/winmidi_bindings.rs を作り直す
#
# dll と pri は midistaged.exe の隣に要る。midistaged の build.rs が target に写す。
# API が Windows に入ったら（MidiApi::IsSystemProvided）同梱をやめる（go-live 条件）。
set -euo pipefail

VERSION="0.99.92-preview.11"
TAG="inbox-preview-11"
SHA256="13afc6e6da2adac309ab8c174352f46f1c0ebe2d79e65475bd6274defb73df74"

root="$(cd "$(dirname "$0")/.." && pwd)"
cache="$root/.cache/winmidi"

fetch() {
  local nupkg="$cache/Windows.Devices.Midi2.$VERSION.nupkg"
  mkdir -p "$cache"
  if [ ! -f "$nupkg" ]; then
    curl -fL --retry 3 -o "$nupkg.part" \
      "https://github.com/microsoft/MIDI/releases/download/$TAG/Windows.Devices.Midi2.$VERSION.nupkg"
    mv "$nupkg.part" "$nupkg"
  fi
  echo "$SHA256  $nupkg" | sha256sum -c - >/dev/null || {
    echo "sha256 mismatch: $nupkg" >&2
    rm -f "$nupkg"
    exit 1
  }
  # 中身は zip。使う 3 つだけ取り出す（pdb は 130 MB あるので取らない）
  local x="$cache/x"
  rm -rf "$x"
  mkdir -p "$x"
  unzip -q -o "$nupkg" \
    'ref/native/Windows.Devices.Midi2.winmd' \
    'runtimes/win-x64/native/Windows.Devices.Midi2.dll' \
    'runtimes/win-x64/native/Windows.Devices.Midi2.pri' \
    -d "$x"
  mv "$x/ref/native/Windows.Devices.Midi2.winmd" "$cache/"
  mv "$x/runtimes/win-x64/native/Windows.Devices.Midi2.dll" "$cache/"
  mv "$x/runtimes/win-x64/native/Windows.Devices.Midi2.pri" "$cache/"
  rm -rf "$x"
  echo "$cache: Windows.Devices.Midi2 $VERSION (winmd / dll / pri)"
}

bindings() {
  [ -f "$cache/Windows.Devices.Midi2.winmd" ] || fetch
  local out="$root/crates/midistaged/src/winmidi_bindings.rs"
  (cd "$root/tools/winmidi-bindgen" && cargo run -q --release -- "$cache/Windows.Devices.Midi2.winmd" "$out")
}

case "${1:-}" in
  fetch) fetch ;;
  bindings) bindings ;;
  *) echo "usage: $0 fetch|bindings" >&2; exit 2 ;;
esac

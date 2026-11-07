#!/bin/sh
# Zips the plugin folder into dist/com.avonderau.ts3deck.streamDeckPlugin.
# Run scripts/build.sh first (on each OS you want to include, copying the
# bin/ folders together). Set REQUIRE_ALL=1 to fail unless every platform's
# binary is present, as the release workflow does.
set -eu
cd "$(dirname "$0")/.."
plugin=com.avonderau.ts3deck.sdPlugin
missing=""
for path in \
  bin/x86_64-unknown-linux-gnu/ts3deck \
  bin/aarch64-unknown-linux-gnu/ts3deck \
  bin/macos/ts3deck \
  bin/windows/ts3deck.exe; do
  if [ -f "$plugin/$path" ]; then
    case "$path" in *.exe) ;; *) chmod 755 "$plugin/$path" ;; esac
  else
    missing="$missing $path"
  fi
done
if [ -n "$missing" ]; then
  if [ "${REQUIRE_ALL:-0}" = 1 ]; then
    echo "missing binaries:$missing" >&2
    exit 1
  fi
  echo "note: packing without:$missing" >&2
fi
mkdir -p dist
out="dist/com.avonderau.ts3deck.streamDeckPlugin"
rm -f "$out"
zip -qr -X "$out" "$plugin" -x '*.DS_Store'
echo "$out"

#!/bin/sh
# Builds dist/com.avonderau.ts3deck.streamDeckPlugin (a zip of the plugin
# folder) for the Deck editor's Plugins tab, OpenDeck or Stream Deck.
set -eu
cd "$(dirname "$0")/.."
mkdir -p dist
out="dist/com.avonderau.ts3deck.streamDeckPlugin"
rm -f "$out"
zip -qr -X "$out" com.avonderau.ts3deck.sdPlugin -x '*.DS_Store'
echo "$out"

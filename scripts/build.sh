#!/bin/sh
# Builds the plugin binary for the current OS into the plugin folder:
#   Linux    bin/<arch>-unknown-linux-gnu/ts3deck  (static musl, x86_64 + aarch64)
#   macOS    bin/macos/ts3deck                     (universal: Apple Silicon + Intel)
#   Windows  bin/windows/ts3deck.exe               (x86_64, static CRT; run from Git Bash)
# Each needs its Rust targets installed; see the README.
set -eu
cd "$(dirname "$0")/.."
bin=com.avonderau.ts3deck.sdPlugin/bin

build() {
  cargo build --release --locked --target "$1"
}

case "$(uname -s)" in
  Linux)
    for arch in x86_64 aarch64; do
      build "$arch-unknown-linux-musl"
      mkdir -p "$bin/$arch-unknown-linux-gnu"
      cp "target/$arch-unknown-linux-musl/release/ts3deck" "$bin/$arch-unknown-linux-gnu/ts3deck"
    done
    ;;
  Darwin)
    build aarch64-apple-darwin
    build x86_64-apple-darwin
    mkdir -p "$bin/macos"
    lipo -create -output "$bin/macos/ts3deck" \
      target/aarch64-apple-darwin/release/ts3deck target/x86_64-apple-darwin/release/ts3deck
    ;;
  MINGW* | MSYS* | CYGWIN* | Windows_NT)
    build x86_64-pc-windows-msvc
    mkdir -p "$bin/windows"
    cp target/x86_64-pc-windows-msvc/release/ts3deck.exe "$bin/windows/ts3deck.exe"
    ;;
  *)
    echo "unsupported OS: $(uname -s)" >&2
    exit 1
    ;;
esac

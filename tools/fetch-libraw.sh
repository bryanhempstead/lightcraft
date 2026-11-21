#!/bin/sh
# Fetch LibRaw 0.22.3 (the latest stable release) from libraw.org into vendor/ (gitignored) for
# crates/libraw-sys, LightCraft's fallback raw decoder for the cameras its own decoders don't read
# yet. LibRaw is dual-licensed LGPL-2.1 / CDDL-1.0: it is fetched at build time and linked
# statically for Bryan's personal build, never committed (this script + checksum are what the
# repository keeps). Only the sources LightCraft builds are unpacked (library sources, headers,
# the licence and copyright files).
set -eu
cd "$(dirname "$0")/.."
VERSION="0.22.3"
URL="https://www.libraw.org/data/LibRaw-$VERSION.tar.gz"
SHA256="789f3c33466210a1a0907f747a97c715d21bbf4af712f257fcbbd90111b227e4"
mkdir -p vendor/dl
TGZ="vendor/dl/LibRaw-$VERSION.tar.gz"
if [ ! -f "$TGZ" ] || [ "$(shasum -a 256 "$TGZ" | cut -d' ' -f1)" != "$SHA256" ]; then
  curl -fL -o "$TGZ.part" "$URL"
  /bin/mv -f "$TGZ.part" "$TGZ"
fi
GOT="$(shasum -a 256 "$TGZ" | cut -d' ' -f1)"
if [ "$GOT" != "$SHA256" ]; then
  echo "checksum mismatch for $TGZ: $GOT (expected $SHA256)" >&2
  exit 1
fi
tar -xzf "$TGZ" -C vendor "LibRaw-$VERSION/src" "LibRaw-$VERSION/libraw" "LibRaw-$VERSION/internal" \
  "LibRaw-$VERSION/LICENSE.LGPL" "LibRaw-$VERSION/LICENSE.CDDL" "LibRaw-$VERSION/COPYRIGHT" "LibRaw-$VERSION/README.md"
echo "LibRaw ready in vendor/LibRaw-$VERSION (cargo builds crates/libraw-sys with it)"

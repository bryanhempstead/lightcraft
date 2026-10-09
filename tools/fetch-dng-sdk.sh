#!/bin/sh
# Fetch Adobe's DNG SDK 1.7.1 from Adobe's own download server into vendor/ (gitignored) for
# crates/dng-sdk-sys. The download carries no licence text of its own ("Adobe permits you to use,
# modify, and distribute this file in accordance with the terms of the Adobe license agreement
# accompanying it" — none accompanies it), so the SDK is never committed: this script + checksum
# are what the repository keeps. Only the parts LightCraft builds are unpacked (SDK sources,
# libjpeg, libjxl's public headers, the read-me files).
set -eu
cd "$(dirname "$0")/.."
URL="https://download.adobe.com/pub/adobe/dng/dng_sdk_1_7_1.zip"
SHA256="5366d9abd0e623858573bfaf0291b5e352d704f51b322753a22f82af3ae6e0c2"
mkdir -p vendor/dl
ZIP=vendor/dl/dng_sdk_1_7_1.zip
if [ ! -f "$ZIP" ] || [ "$(shasum -a 256 "$ZIP" | cut -d' ' -f1)" != "$SHA256" ]; then
  curl -fL -A "Mozilla/5.0" -o "$ZIP.part" "$URL"
  /bin/mv -f "$ZIP.part" "$ZIP"
fi
GOT="$(shasum -a 256 "$ZIP" | cut -d' ' -f1)"
if [ "$GOT" != "$SHA256" ]; then
  echo "checksum mismatch for $ZIP: $GOT (expected $SHA256)" >&2
  exit 1
fi
unzip -q -o "$ZIP" 'dng_sdk_1_7_1/dng_sdk/source/*' 'dng_sdk_1_7_1/libjpeg/*' 'dng_sdk_1_7_1/libjxl/libjxl/lib/include/*' \
  'dng_sdk_1_7_1/libjxl/libjxl/LICENSE' 'dng_sdk_1_7_1/*.txt' -d vendor
echo "DNG SDK ready in vendor/dng_sdk_1_7_1 (cargo builds crates/dng-sdk-sys with it)"

#!/bin/bash
# ps.sh: render every line of manifest.txt ("in.dng<TAB>out.tif[<TAB>min]") through Camera Raw in
# Photoshop 2026 (AppleScript `do javascript`, no GUI input). Launches Photoshop if needed and
# refuses to touch it while it has documents open. Outputs: 16-bit ProPhoto TIFFs.
cd "$(dirname "$0")"
for i in $(seq 1 30); do
  n=$(osascript -e 'tell application "Adobe Photoshop 2026" to do javascript "app.documents.length"' 2>/dev/null)
  if [ "$n" = "0" ]; then break; fi
  if [ -n "$n" ] && [ "$n" != "0" ]; then echo "Photoshop has $n open documents: not touching it"; exit 3; fi
  sleep 5
done
sed "s|@ROOT@|$PWD/|" batch.jsx > .batch-run.jsx
cat > .run.applescript <<AS
set js to read (POSIX file "$PWD/.batch-run.jsx") as «class utf8»
with timeout of 3600 seconds
	tell application "Adobe Photoshop 2026" to do javascript js
end timeout
AS
osascript .run.applescript

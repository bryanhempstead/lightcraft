#!/bin/bash
# hdr_ps.sh DIR: render DIR/manifest.txt ("in<TAB>out.tif<TAB>[min]<TAB>[bits]") through Camera Raw in
# Photoshop 2026 (AppleScript `do javascript`, no GUI input; batch_hdr.jsx). HDR edits come back as
# 16-bit Rec. 2100 PQ documents, SDR ones as ProPhoto. Outputs that exist are skipped, so a rerun
# resumes.
#
# Several agents share one Photoshop: every run takes the lock PS_LOCK (default
# /private/tmp/lightcraft-photoshop.lock, a directory; a lock whose owner process is gone is taken over),
# waits while Photoshop has documents open, and is watched: when no new output appears for
# PS_STALL seconds (default 300; a hidden modal such as a stuck Progress dialog), the run is
# stopped, the lock released and the script exits 4 so the caller can ask for Photoshop to be
# restarted. At the end it quits Photoshop when no other run holds the lock (PS_KEEP=1 to keep it open).
D=$(cd "$1" && pwd); H=$(cd "$(dirname "$0")" && pwd)
LOCK="${PS_LOCK:-/private/tmp/lightcraft-photoshop.lock}"; STALL=${PS_STALL:-300}
got=
for i in $(seq 1 720); do
  mkdir "$LOCK" 2>/dev/null && { got=1; break; }
  owner=$(cat "$LOCK/pid" 2>/dev/null)
  if [ -n "$owner" ] && ! kill -0 "$owner" 2>/dev/null; then rm -rf "$LOCK"; continue; fi
  sleep 5
done
[ -n "$got" ] || { echo "Photoshop lock still held by $(cat "$LOCK/who" 2>/dev/null): try later"; exit 5; }
echo $$ > "$LOCK/pid"; echo "${PS_WHO:-hdrtone} $D $(date)" > "$LOCK/who"
# RAM: quit Photoshop (~3 GB idle) when the run ends and no other run took the lock; PS_KEEP=1 keeps it open.
quit_ps() { [ -n "$PS_KEEP" ] && return; sleep 2; [ -d "$LOCK" ] && return
  osascript -e 'with timeout of 30 seconds' -e 'tell application "Adobe Photoshop 2026" to quit saving no' -e 'end timeout' >/dev/null 2>&1; }
trap 'rm -rf "$LOCK"; quit_ps' EXIT
ask() { osascript -e 'with timeout of 60 seconds' -e 'tell application "Adobe Photoshop 2026" to do javascript "app.documents.length"' -e 'end timeout' 2>/dev/null; }
for i in $(seq 1 30); do
  n=$(ask)
  if [ "$n" = "0" ]; then break; fi
  if [ -n "$n" ]; then echo "Photoshop has $n open documents: not touching it"; exit 3; fi
  sleep 5
done
[ "$n" = "0" ] || { echo "Photoshop doesn't answer (a hidden modal?): restart it"; exit 4; }
sed "s|@ROOT@|$D/|" "$H/batch_hdr.jsx" > "$D/.batch-run.jsx"
cat > "$D/.run.applescript" <<AS
set js to read (POSIX file "$D/.batch-run.jsx") as «class utf8»
with timeout of 14400 seconds
	tell application "Adobe Photoshop 2026" to do javascript js
end timeout
AS
osascript "$D/.run.applescript" &
job=$!
count() { cut -f2 "$D/manifest.txt" | while read -r f; do [ -f "$f" ] && echo; done | wc -l; }
last=$(count); since=$(date +%s)
while kill -0 $job 2>/dev/null; do
  sleep 10
  c=$(count)
  if [ "$c" != "$last" ]; then last=$c; since=$(date +%s); fi
  if [ $(( $(date +%s) - since )) -gt "$STALL" ]; then
    kill $job 2>/dev/null
    echo "no new output for ${STALL}s ($c done): Photoshop looks stuck (hidden modal?); restart it and rerun"
    exit 4
  fi
done
wait $job

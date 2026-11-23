#!/bin/bash
# psq.sh MANIFEST: ps.sh behind a lock shared by every agent driving Photoshop, with a hang watchdog.
#
# - Lock: the directory $PS_LOCK (default /private/tmp/lightcraft-photoshop.lock, holding the owner's pid and
#   name); taken with mkdir, waited for while its owner lives, taken over when the owner is gone. Anyone else
#   driving Photoshop for the oracle should take the same lock (`mkdir`, write `pid`, remove when done).
# - Only the manifest's lines whose output TIFF is missing are rendered (a re-run resumes).
# - Watchdog: if no new output appears for $PS_STALL seconds (default 300) the batch counts as hung (Camera Raw
#   or Photoshop stuck on a modal): its osascript is killed, the lock released, exit 4. Photoshop itself is never
#   touched (no GUI input, no force-quit: it may hold someone's documents); a human clears the dialog.
cd "$(dirname "$0")"
M=${1:-manifest.txt}
LOCK=${PS_LOCK:-/private/tmp/lightcraft-photoshop.lock}
STALL=${PS_STALL:-300}
WHO=${PS_WHO:-$(basename "$(cd ../../.. && pwd)")}
todo=$(awk -F'\t' 'NF >= 2 { cmd = "test -e \"" $2 "\""; if (system(cmd) != 0) print }' "$M")
[ -z "$todo" ] && { echo "psq: nothing to render"; exit 0; }
n=$(printf '%s\n' "$todo" | wc -l | tr -d ' ')
while ! mkdir "$LOCK" 2>/dev/null; do
  owner=$(cat "$LOCK/pid" 2>/dev/null)
  if [ -n "$owner" ] && ! kill -0 "$owner" 2>/dev/null; then
    echo "psq: stale lock of pid $owner ($(cat "$LOCK/who" 2>/dev/null)): taking it over"
    rm -rf "$LOCK"
    continue
  fi
  sleep 10
done
echo $$ > "$LOCK/pid"
echo "$WHO" > "$LOCK/who"
trap 'rm -rf "$LOCK"' EXIT
printf '%s\n' "$todo" > manifest.txt
echo "psq: $n files ($WHO holds $LOCK)"
./ps.sh &
job=$!
count() { printf '%s\n' "$todo" | awk -F'\t' '{ cmd = "test -e \"" $2 "\""; if (system(cmd) == 0) c++ } END { print c + 0 }'; }
last=$(count); quiet=0
while kill -0 $job 2>/dev/null; do
  sleep 10
  c=$(count)
  if [ "$c" != "$last" ]; then last=$c; quiet=0; else quiet=$((quiet + 10)); fi
  if [ $quiet -ge "$STALL" ]; then
    echo "psq: no new render for ${STALL}s ($c / $n done): Photoshop looks hung (a modal dialog?); stopping"
    pkill -P $job 2>/dev/null
    pkill -f "$PWD/.run.applescript" 2>/dev/null
    kill $job 2>/dev/null
    exit 4
  fi
done
wait $job
rc=$?
echo "psq: $(count) / $n rendered (ps.sh exit $rc)"
exit $rc

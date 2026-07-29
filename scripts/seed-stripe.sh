#!/bin/bash
# Seeds registry/stripe/ from stripe/openapi tag history.
# Usage: scripts/seed-stripe.sh [tag_count] [step] [cache_dir]
# step samples every Nth tag to bound downloads over wide windows.
set -euo pipefail

count="${1:-12}"
step="${2:-1}"
cache="${3:-${TMPDIR:-/tmp}/firstwatch-stripe-cache}"
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$repo_root/target/release/firstwatch"
out="$repo_root/registry/stripe/changefeed.ndjson"

[ -x "$bin" ] || { echo "build first: cargo build --release" >&2; exit 1; }
mkdir -p "$cache"

tags=$(curl -s "https://api.github.com/repos/stripe/openapi/tags?per_page=$count" |
    python3 -c "
import json, sys
names = [t['name'] for t in json.load(sys.stdin)]
step = int('$step')
sampled = names[::step]
if names and sampled[-1] != names[-1]:
    sampled.append(names[-1])
print('\n'.join(sampled))")

for tag in $tags; do
    spec="$cache/$tag.json"
    if [ ! -s "$spec" ]; then
        echo "fetching $tag" >&2
        curl -sL "https://raw.githubusercontent.com/stripe/openapi/$tag/openapi/spec3.json" -o "$spec"
    fi
done

ordered=$(echo "$tags" | sort -t v -k 2 -n)
raw="$cache/raw.ndjson"
: > "$raw"
prev=""
for tag in $ordered; do
    if [ -n "$prev" ]; then
        echo "diff $prev -> $tag" >&2
        "$bin" watch --old "$cache/$prev.json" --new "$cache/$tag.json" >> "$raw"
    fi
    prev="$tag"
done

python3 - "$raw" "$out" << 'EOF'
import json, sys

seen = {}
for line in open(sys.argv[1]):
    e = json.loads(line)
    key = (e["surface"], e.get("field"), e["kind"])
    seen.setdefault(key, e)

events = sorted(seen.values(), key=lambda e: (e["surface"], e.get("field") or "", e["kind"]))
with open(sys.argv[2], "w") as f:
    for e in events:
        f.write(json.dumps(e, separators=(",", ":")) + "\n")
print(f"{len(events)} events -> {sys.argv[2]}", file=sys.stderr)
EOF

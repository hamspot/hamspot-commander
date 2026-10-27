#!/usr/bin/env bash
# List manual screenshot placeholders against the files in docs/images.
# Exit 1 if an image reference in the manual points at a missing file.
set -euo pipefail
cd "$(dirname "$0")/.."
guide=docs/UserGuide.md
status=0
mapfile -t ids < <(grep -o '<!-- screenshot: [a-z0-9]*-[0-9]*' "$guide" | sed 's/.*screenshot: //' | sort -u)
have=0; want=0
for id in "${ids[@]}"; do
    file=$(ls docs/images/"$id"-*.png 2>/dev/null | head -n1 || true)
    if [[ -n "$file" ]]; then
        have=$((have+1)); echo "have    $id  $file"
        grep -q "(images/$(basename "$file"))" "$guide" || echo "        ^ file exists but the manual does not reference it yet"
    else
        want=$((want+1)); echo "missing $id"
    fi
done
while IFS= read -r ref; do
    [[ -f "docs/$ref" ]] || { echo "BROKEN  $ref is referenced but missing"; status=1; }
done < <(grep -o '](images/[^)]*)' "$guide" | sed 's/^](//; s/)$//')
for f in docs/images/*.png; do
    [[ -e "$f" ]] || continue
    grep -q "images/$(basename "$f")" "$guide" || echo "unused  $f"
done
echo "placeholders: ${#ids[@]}  with file: $have  missing: $want"
exit $status

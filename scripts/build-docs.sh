#!/usr/bin/env bash
# Build the web manual (GitHub Pages) into build/site. Usage: scripts/build-docs.sh [serve]
set -euo pipefail
cd "$(dirname "$0")/.."
command -v mkdocs >/dev/null || { python3 -m venv build/venv && build/venv/bin/pip install -q mkdocs-material; PATH="$PWD/build/venv/bin:$PATH"; }
rm -rf build/docs-src && mkdir -p build/docs-src
cp docs/UserGuide.md build/docs-src/index.md
cp docs/UserGuide.pdf docs/patchbay.md docs/AudioGraph.md build/docs-src/
cp -r docs/rotator docs/images build/docs-src/
mkdir -p build/docs-src/design && cp -r docs/design/tv build/docs-src/design/
if [ "${1:-}" = serve ]; then exec mkdocs serve; fi
mkdocs build
echo "Built: build/site ($(du -sh build/site | cut -f1))"

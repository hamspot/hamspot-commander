#!/usr/bin/env python3
"""Check relative links and #anchors in the repo's Markdown docs.

Anchors are explicit <a id="..."> tags or GitHub-style heading slugs.
Exit 1 on any broken link. External http(s) links are not fetched.
"""
import pathlib, re, sys

root = pathlib.Path(__file__).resolve().parent.parent
files = [root / "README.md", root / "AGENTS.md"] + sorted((root / "docs").rglob("*.md"))
files = [f for f in files if f.exists()]

def anchors(text):
    out = set(re.findall(r'<a id="([^"]+)"', text))
    seen = {}
    in_code = False
    for line in text.splitlines():
        if line.startswith("```"):
            in_code = not in_code
            continue
        m = None if in_code else re.match(r"#{1,6} (.*)", line)
        if m:
            s = re.sub(r"[^\w\- ]", "", m.group(1).strip().lower()).replace(" ", "-")
            n = seen.get(s, 0); seen[s] = n + 1
            out.add(s if n == 0 else f"{s}-{n}")
    return out

bad = 0
cache = {}
for f in files:
    text = f.read_text()
    body = re.sub(r"```.*?```", "", text, flags=re.S)
    for target in re.findall(r"\]\(([^)\s]+)\)", body):
        if re.match(r"[a-z]+:", target):
            continue
        path, _, frag = target.partition("#")
        dest = (f.parent / path).resolve() if path else f
        if not dest.exists():
            print(f"{f.relative_to(root)}: missing file {target}"); bad += 1; continue
        if frag and dest.suffix == ".md":
            if dest not in cache:
                cache[dest] = anchors(dest.read_text())
            if frag not in cache[dest]:
                print(f"{f.relative_to(root)}: missing anchor {target}"); bad += 1
print(f"checked {len(files)} files, {bad} broken")
sys.exit(1 if bad else 0)

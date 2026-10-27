# Notes for AI and developer agents

Working rules and facts for anyone (human or agent) changing this repo. The user manual is `docs/UserGuide.md` and is for operators only: keep developer notes, design reasoning, issue numbers and TODOs out of it and put them here or in `docs/agents/`.

Last reviewed: 2026-10-27.

## Index

| File | What it holds |
| --- | --- |
| `docs/agents/decisions.md` | Design decisions and the reasons for them |
| `docs/agents/rec-internals.md` | REC architecture, formats and known gaps |
| `docs/agents/hardware-notes.md` | Hardware scope and device quirks |
| `docs/AGENT-HSCommander.md` | Turning a listing into a `.HSCommander` station and `.HSKeys` file |
| `docs/design/` | Design proposals (on their own branches until accepted) |
| `docs/images/README.md` | Screenshot naming; the wanted list is issue #149 |

## Build and test

```bash
cargo build --release -p commander          # the window; also builds commander-rec
cargo test --workspace
python3 scripts/docs-check-links.py         # manual links and anchors
scripts/docs-check-images.sh                # screenshot placeholders vs docs/images
```

A known test failure on the owner's machine (SAFE-T channel count) comes from the live station file, not the code.

**Tower5810 (ZFS) stalls under heavy I/O.** Before any cargo command:

- check `pgrep -a cargo` and run only one cargo at a time;
- run it as `nice -n 19 ionice -c3 cargo … -j2`;
- run `sync` after large builds.

## Rules

- **Never run the GUI binary** (`commander`) in tests or scripts. `commander-rec` is fine.
- **Don't touch the user's live config** (`~/.config/hamspot-commander`, their station files, recordings). Read-only if you must look; work on copies.
- **Don't push to `main`, merge, install, tag or bump versions** unless the owner asked for that step. Work on a branch and push the branch.
- **File issues with `gh` as the `webaugur` account.** Unsupported hardware or features are noted once in the overview issue, not in many places.
- **Warnings carry text, not colour alone.** Every lamp or colour state also has words on screen or in a tooltip.
- **Click and key conventions follow rustcars** (`github.com/webaugur/rustcars`): click = primary (green dot), double-click or Shift = first alternate (orange), triple-click or Ctrl = second alternate (purple). Before adding a shortcut or modifier anywhere, check rustcars' mode buttons and keys so one key never gets two meanings.
- **A USB numeric keypad must work in every number field.**
- **Prefer GNU Radio** for new demodulators, wrapped as a pipe file.
- **Recording policy:** raw IQ is saved and re-demodulated on playback; demodulated audio is never saved; encrypted traffic is never recorded. Nothing is uploaded.
- **No arbitrary commands from data.** Station files, recordings and imports may only name assets by kind-relative name (`modems/<file>`). See `docs/agents/decisions.md`.
- **Safety first for hardware designs** (patch bay, amplifier control): nothing keys, switches or goes to Operate without an explicit click.
- **Keep `docs/UserGuide.pdf` in mind:** it's an export made by `docs/export-user-guide.py` and is older than the Markdown. Regenerate only when asked.

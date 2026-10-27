# Manual screenshots

Images for `docs/UserGuide.md`. The list of shots wanted is GitHub issue #149.

- Name: `<chapter>-<nn>-<slug>.png`, for example `rec-09-playback-dialog.png`. `<chapter>-<nn>` is the id from the issue (`main`, `station`, `tune`, `plan`, `chan`, `rec`, `meter`, `keys`, `cfg`, `pipe`, `vna`, `smith`, `rfcalc`, `calc`, `hex`, `log`, `rot`, `timers`).
- PNG, window about 1280×800, default dark theme.
- No private data: mask serial numbers, callsigns, locations and real radio IDs.

The manual marks where each shot belongs with a comment such as
`<!-- screenshot: rec-09 — The Record / Playback dialog. -->`.
When you add the file, put the image right under that comment:

```markdown
<!-- screenshot: rec-09 — The Record / Playback dialog. -->
![The Record / Playback dialog.](images/rec-09-playback-dialog.png)
```

`scripts/docs-check-images.sh` lists which placeholders have a file, which don't, and any image the manual doesn't use.

# Design decisions

Brief record of decisions and why. Dated when recorded here (2026-10-27) unless noted.

- **Pipe files only (2026-10-27).** Station RX/TX pipe fields name `modems/<file>` only. No shell commands, no absolute paths. Reason: a station file or recording someone hands you must not be able to run an arbitrary program. Custom commands are wrapped into a pipe file in `modems.d` after the user confirms the exact contents. Load migrates known absolute paths in memory; anything else is disabled with a visible warning.
- **Asset-kind registry.** Allowed asset folders come from `hsrec::safepath::KINDS` (today only `MODEMS`: `modems/`, `.rxpipe`/`.txpipe`, user folder `modems.d`). A new kind (e.g. flowgraphs) is one more entry. Resolver rejects absolute paths, `..`, subfolders, symlinks, non-owner-executable files, and group/world-writable files or folders.
- **Release builds never search the CWD or source tree** for assets. Debug builds also search the source `modems/` and `HAMSPOT_DEV_MODEMS`.
- **Decoder settings are generic.** Recordings carry `decoder_params`, a free-form `HAMSPOT_[A-Z0-9_]+` map; replay passes them as env vars. Keys outside the pattern, and values that look like paths, are refused from data (`--param` on the CLI can still set them). Reason: future decoders can save what they need without schema changes.
- **No WACN/SYS/NAC in the station file.** Learned from the air. `dsd-fme -X` is the site word, not a key.
- **Keys live in `Name.HSKeys` (0600)**, never in the station JSON.
- **Commander starts and stops its own `rigctld`/`gqrx`/`rotctld`/`ampctld`**; it never attaches to ones it didn't start.
- **Amplifiers:** never sent Operate on load or restart; patch bay is visual only; boxes bind to the amplifier's fixed random id, never its name. Antenna switching, PTT/ALC and automatic operate are not planned without a separate decision.
- **Layout reference:** the Tune face loosely follows the Ham Radio Deluxe 5.21 rig screen (behaviour only; no icons, bitmaps or text copied).
- **Data layout (proposal, not built):** `Foo.HSCommander` plus a `Foo.HSData/` folder; see `docs/design/data-layout.md` on branch `design-data-layout` and epic #148.
- **Manual style (2026-10-27):** operator-only content, second person, tables for reference, screenshot placeholders as HTML comments with #149 ids.

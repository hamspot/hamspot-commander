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

## E-STOP button height (#200, 2026-10-27)

The E-STOP and RELEASE buttons use the standard egui button height
(`estop::button_height` = `max(interact_size.y, button text + 2 * button_padding.y)`,
the rule `egui::Button` uses) and the Button text style. Width follows the text.
The octagon radius is about half the text height, kept 2 px clear of the border.
The fixed 24 px `BUTTON_H` is gone, and the Config E-STOP group bars went back from
28 px (r_out 16) to the normal 16 px (r_out 12) `Metrics::default()`. The tan top bar
and the Tools tab row were not enlarged for it.

## HF data decode window (#217)

- Code lives in `crates/commander/src/hfdata/`. One **HF Data** page (not one window per decoder) with mode toggles; a tabbed or per-decoder split can come later.
- Plug-in point: `registry::DecodeMode` (turns one decoder stdout line into a `Decode`) plus a `Factory` registered with `Registry::register`. `Session` drains an `mpsc::Receiver<String>` of decoder lines into the shared `DecodeList`. The rxpipe/patch bay wiring that feeds that channel belongs to the first mode issue (#206) and #171/#172.
- `registry::parse_wsjt_line` parses `HHMMSS SNR DT FREQ ~ MSG`; time of day is resolved to the latest instant not in the future (midnight rollover). Rows stamped more than 2 s ahead are refused and counted on screen.
- Slots count from the Unix epoch; lengths must divide a UTC day (`SlotLen::new`).
- Clock check: `chronyc -c tracking`, then `timedatectl`, on a thread once a minute. Warn above 1.0 s, when unsynchronised, or when neither answers.
- Presets: `share/hfdata/presets.json` (embedded, `deny_unknown_fields`, no paths). All `checked: false`. Several common dials sit in the ARRL plan's CW segments (e.g. FT8 7.074, FT4 7.0475, WSPR 7.0386); the page states the segment rather than hiding it.

- **VNA device scans (#219, 2026-10-27).** Scans of patch-panel devices go in `Foo.HSData/VNA/<device uid>/<UTC stamp>[-n].HSVNAScan`, derived from the station file name (first use of `Foo.HSData/`, #148; the planned `vna/` for calibration is separate). Devices gain a random `uid` (assigned once, also to older devices at load). Scan files hold no paths. Kinds beyond the original seven are data: `crates/commander/src/vna_devices/rf_kinds.json` (ports, left/right side, scan plan `s11|all_pairs|all_ordered|from_first`, TX through-paths, `tx` flag); a new row needs a rebuild but no code. All ports are 50 Ω. The wizard reuses the calibration wizard's step style and wiring painter (`paint_device_wiring`), not the calibration `Wizard` state itself.
- **Format change: `HSVNAScan` v1 (#219, for #203).** JSON `{format:"HSVNAScan", version:1, device_uid, device_name, device_kind, ports, taken_at, start_hz, stop_hz, points, vna_id, cal_note, by, note?, length_m?, vf?, steps:[{step:{ch0,ch1?,loads?}, raw: StoredScan, corrected?: StoredScan}]}`. Newer/unknown versions are refused and left untouched. Written temp + hard link (never overwrites). `serde_json` now has `float_roundtrip` so sweep values survive a load/save exactly. Station file: patch `Device` gains `uid`, `rf`, `nports`, `notes`, `length_m`, `vf` (all optional; older Commanders refuse these files because `Device` is `deny_unknown_fields`). Marker `Foo.HSData/VNA/tx-lockout.json` `{format:"HSVNALock",version:1,device,since}` exists only while a lockout-holding scan runs.
- **TX lockout (#219).** `tx_lockout::TxLockout` is a set of named holds; transmit is allowed only when none remain. Refusal sites: `App::ptt` (PTT/MOX, with text), `App::send` backstop for `SetPtt(true)`, `App::send_amp` for `SetPower(Operate)`. The VNA hold is engaged before the sweep is requested, released on finish or cancel, made manual (Release button only) on disconnect, and restored as manual at station load when the marker is left over. **E-Stop (#198) convergence:** E-Stop should add its own hold key at the same three sites (and its rotator stop path); it stays separate in behaviour (latched, manual release) and must not be cleared by the VNA release.
- **E-Stop + TX lockout converged (merge to main, 2026-10-27).** Implemented as `tx_lockout::refuse_transmit(lock, estop::engaged(), action)` at the three App sites (`App::ptt` PTT/MOX, `App::send` for `SetPtt(true)`, `App::send_amp` for OPERATE/On). E-Stop is checked first and gives "… refused. E-STOP is engaged; press RELEASE …"; otherwise any lockout hold gives "… refused. TX LOCKOUT: …". E-Stop was not turned into a lockout hold key: it keeps its own latched state, persisted file and device-hub choke points (`estop::allows_rig/allows_amp/allows_rot`, TX pipe stop, rotator stop), so the rotator is covered by E-Stop at the hub only (the lockout never held the rotator). VNA "Release TX lockout" cannot clear E-Stop and RELEASE cannot clear a VNA hold. Test: `tx_lockout::tests::transmit_is_refused_under_estop_or_lockout_with_its_own_text`.

## E-STOP button shape, second fix (#200, 2026-10-27)

David rejected the first #200 fix on screen (E-STOP ~30 px with big text, RELEASE ~24 px, both rounded). Real cause: E-STOP drew its text with `TextStyle::Button` (rustcars `apply_theme` sets it to Antonio 22 pt, galley 28 px → 36.5 px button), while RELEASE was an `egui::Button`, which in egui 0.37 draws a `&str`/`RichText` label with `FontSelection::Default` = **Body** (15 pt, galley 19 px → 19 + 2 × 4 = 27 px), so the two never matched. Rounding came from E-STOP's own `rect_filled(rect, 4.0, …)` and, for RELEASE, egui's widget visuals (`corner_radius = 4` from rustcars `apply_theme`, which every other app button also uses).
Now: `estop::std_button_font` / `std_button_height` reproduce what `ui.button` really uses; one `square_button` paints both buttons (corner radius 0 `estop::CORNER`, 1.5 px border in both states, same font); `estop::buttons` lays them out with `horizontal_centered` so they sit centred in whatever bar slot they get. Headless tests (`estop::tests`) run a themed `egui::Context` frame and assert E-STOP, RELEASE and a plain `ui.button` are all 27 px and that the buttons are centred in a 44 px bar. The other app buttons were left rounded.

## Tools detach is the page pill (#221, 2026-10-30)

The square tear-off tab at the right of the page row and the deferred `hamspot-tools` viewport are gone. Tools now detach like Config, REC, Timers, HF Data, Levels and Keys: the top-right page pill (`DETACHABLE_PAGES`, `detached_window`, an immediate viewport). The detached window shows E-STOP / RELEASE at the right of its top row and then the same body as the page (tool sub-tabs, status line, tool screen); it polls the E-STOP chord itself.
**Migration:** the eframe storage keys `tool_detach` ("1"/"0"), `tool_win_pos` and `tool_win_size` ("x,y") are read once at startup by `migrate_tool_detach` into the Tools page `DetachState` (detached, pos, size). `Tools.detached` / `saved_pos` / `saved_size` mirror that state every frame and the same three keys keep being written, so the setting survives restarts (other page pills do not persist) and an older build still reads a sensible value. `Tools.dock` is gone (closing the old window set it). Test: `tool_detach_migrates_to_the_page_pill`.

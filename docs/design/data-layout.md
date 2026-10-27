# Station data layout: `Foo.HSCommander` + `Foo.HSData/`

Status: proposal (design only, nothing implemented). Based on main `6d5949c`.

## Goal

Keep a clear line between the **application** (the binaries, `modems/` scripts, BSPs, defaults and docs, all reinstallable) and the **data it produces** for each station (recordings, logbook, calibration, keys, caches). Each station file `Foo.HSCommander` gets one sibling folder `Foo.HSData/`. Nothing inside a station file or HSData stores an absolute path. Imported data can never name an executable.

## 1. Inventory (main 6d5949c)

Legend: **App** = application or reinstallable; **Data** = produced per station or user; **Host** = machine or user state, not per station.

| What | Path pattern today | Code | Created by | Class | Keyed by station | Absolute paths stored inside |
|---|---|---|---|---|---|---|
| Station dir | `$XDG_CONFIG_HOME/hamspot-commander/` (default `~/.config/...`) | `commander/src/config.rs:331` `station_dir()` | Commander | Data | – | – |
| Station file | `<dir>/Foo.HSCommander` (any path given on the command line) | `config.rs:19,357,375` | Commander, user | Data (config) | yes | **yes**: `radios[].rx_pipe` / `tx_pipe` hold full paths to `~/.local/share/hamspot-commander/modems/*.rxpipe` (SAFE-T: 9 rx, 9 tx, 1 trace). `cat.rs:168,173`. Also `conf`, `ptt_file` and `device` are path-shaped strings (`cat.rs:139,147,149`) |
| Corrupt backup | `Foo.HSCommander.corrupt-YYYYMMDD` | config load path | Commander | Data | yes | – |
| VNA calibration | `Foo.HSVNA` (+ `.HSVNA.tmp`, `.HSVNA.corrupt-<stamp>`) | `tools/nanovna/mod.rs:385,432,454` | Commander | Data | yes (`with_extension`) | no |
| Keys | `Foo.HSKeys` (mode 600) | `keys.rs:60` | Commander | Data (secret) | yes | no |
| Runtime key CSV | `$TMPDIR/hamspot-commander-keys.csv` | `keys.rs:309` | Commander | Host/tmp (secret) | **no, one shared name** | path passed to the decoder |
| Logbook / diary | `Foo.HSLog` (SQLite) | `tools/station_log/mod.rs:351` | Commander | Data | yes | no |
| REC database | `Foo.HSREC.sqlite` + `-wal`/`-shm` + `.lock` | `hsrec/src/db.rs:123`, `lock.rs:11` | Commander, commander-rec | Data | yes | `raw_path` is relative; the decoder script in `meta_json` was absolute (being fixed on `rec-dialog-order`) |
| Recordings | `Foo.recordings/<source>/<YYYY-MM-DD>/<HHMMSS>_<lane>_<MHz>.cs16|.cs8|.s16[.zst]` + `.json` sidecar | `hsrec/src/storage.rs:13,35` | writer | Data | yes | sidecar meta: same as DB |
| Archive temp | `….zst.tmp` beside the file | `archive.rs:57` | archive | Data (transient) | yes | – |
| Render cache | `Foo.recordings/.cache/<id>-<kind>-<ver>.bin` | `hsrec/src/cache.rs:24`, `storage.rs:21` | REC UI | Cache (rebuildable) | yes | – |
| Thumb-drive export | `<mount>/HamSpot-REC/<raw_path>` + `index.json` | `hsrec/src/export.rs:11,91` | export | Data (external) | **no**: one `HamSpot-REC` per drive, so two stations collide | `raw_path` relative, but **not validated on import** (see Surprises) |
| p25voice opt-in marker | `~/.config/hamspot-commander/with-p25voice` | `commander/build.rs:54` | user | Host (build flag) | no | – |
| Gqrx INI (current) | `~/.local/share/hamspot-commander/gqrx/<Radio>.conf`, `recentconfig.cfg`, `bandplan.csv` (Gqrx gets `XDG_CONFIG_HOME` pointed here) | `cat.rs:664,684` | Commander + Gqrx | Host per radio | no (by radio name) | Gqrx writes its own |
| Gqrx INI (legacy) | `~/.config/hamspot-commander/gqrx/*.conf` | `cat.rs:695` | old builds | Host, legacy | no | – |
| Gpredict hwconf | `~/.config/Gpredict/hwconf/hamspot_*` | `gpredict_hw.rs:65` | Commander (owned files only) | Host, another app's dir | no | – |
| eframe state | `~/.local/share/hamspotcommander/app.ron` | eframe, `main.rs:1911,2530` | eframe | Host | no | **yes**: `"config_dir": "/home/user/.config/hamspot-commander"` (last-used dir) |
| Instance or exit records | `$XDG_RUNTIME_DIR/hamspot-commander/` or `~/.cache/hamspot-commander/` | `exit_watch.rs:32` | Commander | Runtime | no | pids |
| WirePlumber reset | `~/.local/state/wireplumber` (deleted on request) | `audio.rs:726` | WirePlumber | Host, another app's | no | – |
| Modem scripts | `~/.local/share/hamspot-commander/modems/*.rxpipe|.txpipe`, symlinked into `~/.local/bin/` | `rx_pipe.rs:250` (search order: data home, exe dir, `../modems`, **`CARGO_MANIFEST_DIR/../../modems`**, **`./modems` (CWD)**); `install.sh:119` | installer | App | no | – |
| Install manifest + dsd-fme lists | `~/.local/share/hamspot-commander/{manifest, dsd-fme-files.txt, dsd-fme-rev, p25voice-files.txt}` | `install.sh:174`, `build-dsd-fme.sh` | installer | App | no | absolute (fine: describes the install) |
| dsd-fme trace log | `~/.local/share/hamspot-commander/dsd-fme-trace.log` (2.7 MB, grows without limit) | `modems/commander-demod-p25-trace.rxpipe:31` | trace script | Log | no | – |
| Source build cache | `~/.cache/hamspot-commander/{dsd-fme*,mbelib*,codec2*,DecDRM}` | `build-dsd-fme.sh`, installer | installer | App build cache | no | – |
| Unowned leftovers | `~/.local/share/hamspot-commander/QtProject.conf` (Gqrx/Qt), `gnuradio/prefs` (GNU Radio) | other apps, because of the `XDG_*` override | other apps | Host | no | – |
| Portable tree | `<prefix>/{bin,config,data,share,bsp}`; launcher sets `XDG_CONFIG_HOME` and `XDG_DATA_HOME` | `portable-install.sh:9`, `hamspot-commander.sh` | portable installer | both | – | – |
| BSP and defaults | repo `bsp/`, `share/` | installer | App | – | – |
| Test/tmp | `$TMPDIR/hamspot-*`, `hsrec-player-out.raw`, `hs-helvum-*` | tests only | tests | tmp | – | – |

About 16 code sites build these paths: `config.rs`, `keys.rs`, `nanovna/mod.rs`, `station_log/mod.rs`, `hsrec/{db,storage,cache,lock,export,archive}.rs`, `commander-rec.rs`, `rec_ui.rs:348`, `cat.rs`, `rx_pipe.rs`, `exit_watch.rs` and `main.rs`. Add 3 scripts (`install.sh`, `portable-install.sh`, `uninstall.sh`), 1 modem script and the docs (`UserGuide.md`, `AGENT-HSCommander.md`).

## 2. Proposed layout

```
~/.config/hamspot-commander/            (any folder works; this is just the default)
  SAFE-T.HSCommander                    user-facing config: radios, channels, REC marks, TV presets
  SAFE-T.HSData/
    HSData.json                         {"layout":1,"station":"SAFE-T","created":...,"migrated_from":...}
    keys/SAFE-T.HSKeys                  mode 600, folder 700
    vna/calibration.HSVNA               (+ .corrupt-* aside copies)
    log/logbook.HSLog
    rec/
      rec.sqlite (+wal/shm/lock)
      files/<source>/<YYYY-MM-DD>/...cs16[.zst] + .json
    tv/                                 future TV baseband lives under rec/files/tv_*; tv/ holds TV-only state
    cache/                              safe to delete any time (render cache, waveforms)
    tmp/                                runtime key CSV (600), archive .tmp files; cleared at start
    logs/                               per-station decoder trace logs, rotated
    exports/                            optional local export target
    migration/                          manifests and rollback for layout upgrades
  ...
~/.local/share/hamspot-commander/       APP ONLY: modems/, manifest, dsd-fme lists (reinstallable)
~/.cache/hamspot-commander/             APP build cache only
```

Rules:
- **Finding HSData:** it is always `station_path.with_file_name(stem + ".HSData")`. It is derived, never stored. One function, `hsdata::root(station)`, in a small shared crate used by `commander` and `hsrec`, replaces every `with_extension`/`*_beside` helper.
- **Inside the station file and HSData, every path is relative** to HSData (recordings) or names an app asset (`modems/commander-demod-p25.rxpipe`). Absolute paths are rejected on load (warn and ignore the field). The `rx_pipe`/`tx_pipe` change from `rec-dialog-order` extends to station files: store `modems/<name>` and resolve it against the app's modem dirs.
- **Rename or move:** Commander's own *Rename/Move station* moves both. A move done outside Commander that leaves the folder behind is detected: on open, if `Foo.HSData` is missing but an HSData whose `HSData.json` says `station:"Foo"` sits nearby, or there are old flat files, offer to re-link it. It is never auto-created over that.
- **Save As / copy:** *Save As* writes the new station file and asks: "Copy station data (N GB) / Start empty / Share nothing". The default is **start empty except keys and VNA cal**, which are small. Recordings are never silently duplicated.
- **Two stations** are fully independent. Two Commanders on the *same* station are blocked by the existing REC lock, extended to an `HSData/.lock`.
- **Portability:** copy `Foo.HSCommander` + `Foo.HSData/` to a thumb drive or another machine and it opens as is. The app resolves `modems/…` against its own install.

### Storage cap, other disks, ZFS
- The cap stays per station and is measured over `rec/files` + `cache`.
- **Putting recordings on another disk:** support exactly one indirection, `rec/files` itself being a **symlink** (or the whole `Foo.HSData` being a symlink) chosen with a "Move recordings to…" button that copies, verifies and then links. That is safe because the *user* creates it once and Commander records it in `HSData.json` (`"files_link": true`). No other symlinks inside HSData are followed. While walking, resolve every path with `canonicalize` and require it to stay under `canonicalize(rec/files)`. Do not add a "recordings dir" field to the station file: that would put an absolute path back in.
- Free space is measured on the target filesystem (`statvfs` of the resolved `rec/files`). On ZFS, df-style numbers include snapshots and quota. Show "free (filesystem)" and use the cap as the authority. Recommend a dedicated dataset (`recordsize=1M`, `compression=off`, since IQ doesn't compress and archive uses zstd anyway).

### Migration from the flat layout (layout 0 → 1)
1. On open, if `Foo.HSData/` is missing and any of `Foo.HSVNA`, `.HSKeys`, `.HSLog`, `.HSREC.sqlite`, `.recordings/` exists, show a one-time dialog: "Move station data into SAFE-T.HSData (recommended) / Later". Commander never migrates silently, and `commander-rec` never migrates.
2. Requires Commander to hold the lock, with no writer running and the DB checkpointed (`PRAGMA wal_checkpoint(TRUNCATE)`).
3. Same filesystem: `rename` each item (atomic and instant). Different filesystems are impossible in the default layout. Otherwise copy, sha256 and fsync.
4. Write `migration/0001-flat-to-v1.json` listing every source → destination with size and sha256, plus a rollback script (`rollback.sh`) and a "Restore old layout" button.
5. Rewrite stored absolute modem paths in the station file to `modems/<name>`, keeping a `Foo.HSCommander.pre-v1` backup.
6. With rename there are no old files to keep. If copying was needed, the originals are kept and listed as "old copies, delete when you're happy", with a button. **Never auto-delete.**
7. Until migrated, the old flat paths keep working read and write (a compatibility shim in `hsdata::root`), so nothing breaks if the user clicks Later.

### commander-rec, installers, docs
- `commander-rec --station PATH` derives HSData the same way. Add `--hsdata DIR` for a moved data folder. With old flat files and no HSData it works read-only and prints "run Commander once to migrate". Fix `--help` at the same time.
- `install.sh`/`portable-install.sh` install `commander-rec`. The portable tree's `config/` holds `Foo.HSCommander` + `Foo.HSData/`, unchanged otherwise. `uninstall.sh` must never touch `*.HSData` (it should say so).
- Move the trace log to `Foo.HSData/logs/` by passing `HAMSPOT_LOG_DIR` from Commander, with size-based rotation.
- Update the UserGuide "Files" section and `AGENT-HSCommander.md`.

### Security rules
1. Every stored or imported path inside HSData (DB `raw_path`, sidecars, `index.json`, archive names) must be relative, with no `..`, no root, no prefix and no NUL. After joining, its canonical parent must be under the canonical HSData root. One helper, `hsdata::resolve(rel) -> Result<PathBuf>`, does this, with tests.
2. Executables come **only** from the app's modem dirs. Stored values are bare `modems/<name>` and must match a script in those dirs. Drop the `./modems` (CWD) entry from `modem_dirs()` in release builds, and the `CARGO_MANIFEST_DIR` entry as well (debug only).
3. Decoder parameters stay `HAMSPOT_`-only (already done).
4. `keys/` is 700 and the runtime CSV moves to `HSData/tmp/` (600), unique per station, and is removed at exit.
5. Imports go into a staging folder under `HSData/tmp/import-*`, are verified, then renamed into place.

### Pipe files are an asset kind (added 2026-10-27)

Since `pipe-files-only`, RX/TX pipe files are an *asset kind*, not station data. They live in the installed `modems/` folder (read-only) and the user's `modems.d/` folder, both app-level under `~/.local/share/hamspot-commander/` (or beside the portable binary), and stay **outside** `Foo.HSData/`. Station files and recordings hold only `modems/<name>`; the resolver finds the file. Any new kind (for example flowgraphs) is one more entry in the registry `hsrec::safepath::KINDS`, with the same name, symlink and permission checks.

## 3. Effect on open issues
- **REC epic #119 (#120–#133):** paths in storage/DB/export/CLI change to `HSData/rec/…`. The thumb drive becomes `HamSpot-REC/<station>/…` with `index.json` per station, so two stations don't collide. The old `HamSpot-REC/index.json` is still importable.
- **TV epic #134 (#135–#147):** TV baseband goes into `rec/files/tv_<std>/…` (it shows as a "TV" source). The GNU Radio flowgraphs are app assets (`modems/` or `flowgraphs/`), never stored by path. Presets and AUX stay in the station file (#147), as decided. Any per-station GNU Radio prefs go to `HSData/tv/`, not to the shared `gnuradio/prefs`.
- **#145 demodulator dev kit:** user-written demodulators need a clear "user modems" app dir (`~/.local/share/hamspot-commander/modems.d/`), not HSData, so imported data still can't add executables.

## 4. Suggested breakdown (PR-sized, in order)
1. **Security fix now:** validate `raw_path` in `export::import`/`reattach`/`locate` and in DB rows (`hsdata::resolve`). Small and independent. *Risk: low.*
2. `hsdata` module: `root()`, `resolve()`, layout version, compatibility shim that returns the flat paths for layout 0. All call sites use it, with no behaviour change. *Risk: low, mechanical (~16 sites).*
3. Station-file modem paths to `modems/<name>` + reject absolute paths on load; drop CWD/manifest modem dirs in release. (Follows `rec-dialog-order`.) *Risk: medium: an old station whose script was outside the modem dirs stops working; warn clearly.*
4. Migration dialog + manifest + rollback (layout 1). *Risk: highest; test on a copy of SAFE-T first.*
5. REC on layout 1: DB/files/cache/tmp, lock, cap on the resolved dir, "Move recordings to…" symlink flow. *Risk: medium.*
6. Per-station runtime key CSV, trace log to `HSData/logs` with rotation. *Risk: low.*
7. Rename / Move / Save As station flows. *Risk: medium (UX).*
8. Per-station export folder on the thumb drive + old-format import. *Risk: low.*
9. `commander-rec` `--hsdata`, `--help` fix, installers install it, uninstall leaves HSData; docs. *Risk: low.*

## 5. Open questions for David
1. **Folder name:** `Foo.HSData`? *Recommend yes.*
2. **Subfolder names** (`rec/ vna/ log/ keys/ cache/ tmp/ logs/`)? *Recommend as shown.*
3. **Migration trigger:** a prompt on first open, or automatic? *Recommend a prompt, with Later allowed and the old layout still working.*
4. **Save As default:** *recommend copying keys + VNA cal only, and asking about recordings.*
5. **Recordings on another disk:** symlink of `rec/files` (or the whole HSData) only, no path field? *Recommend yes.*
6. **Thumb-drive layout:** per station `HamSpot-REC/<station>/`? *Recommend yes, with old exports still importable.*
7. **Gqrx INI, eframe `app.ron`, the p25voice marker:** keep them per machine (not in HSData)? *Recommend yes. They're machine or radio state, not station data. Just stop `app.ron` storing an absolute `config_dir` (store the last station only, and let a missing one fall back to the default).*
8. **The dsd-fme trace log:** per station under `HSData/logs`, rotated at 10 MB? *Recommend yes.*
9. **Rename on disk outside Commander:** offer a re-link, never auto-create an empty HSData over found data? *Recommend yes.*

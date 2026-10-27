# REC internals

Facts as of 2026-10-27. Code: `crates/hsrec`, `crates/commander/src/rec_ui.rs`, `rec_live.rs`.

- **Tap.** HackRF channel IQ after DC removal, shifted to the call frequency and averaged to **32 ksps cs16** (half the 500-sample block of the live voice path), so replay rebuilds the exact 16 kHz discriminator stream. ~128 KB/s, ~460 MB/h. The full 8 Msps capture (~58 GB/h) is not saved.
- **Files.** `Name.HSREC.sqlite` index; `Name.recordings/<source>/<YYYY-MM-DD>/<HHMMSS>_<lane>_<MHz>.cs16` + `.json` sidecar (frequency, centre, offset, rate, chain, TG, radio, `decoder_params`, Commander version). Sidecars are the source of truth; Rescan rebuilds rows.
- **Writer.** Idle CPU/IO priority, 128 KiB sequential chunks, no per-chunk fsync; drops and counts samples instead of stalling live decode. Crash-truncated recordings repaired on start. 250 ms pre/post roll, 5-minute split, re-keys inside voice hang merge.
- **Encrypted discard.** Marking refused for encrypted TG/channels; a call whose decoder reports a non-clear algorithm is discarded even with a key.
- **Caches** (`Name.recordings/.cache/`, waveforms keyed by recording + decoder version) are disposable.
- **Archive.** zstd level 9 in background: temp file → decompress and compare sha256 → rename → remove original. Any failure keeps the original. No live compression.
- **Export/drive.** `HamSpot-REC/` + `index.json`; move deletes local only after sha256 verify; import skips index entries with absolute paths, `..` or symlinks. Lock file `Name.HSREC.sqlite.lock` shared by UI jobs and `commander-rec` (exit 3 when held).
- **Replay.** Raw IQ → shared discriminator → the recording's decoder pipe (resolved by name through `safepath`) → `pw-cat` raw to default output. `pw-cat` needs the raw flag (missing flag caused "Broken pipe"/2.0 s replays, fixed 2026-10-26). The decoder UDP port is taken from the live pool and held for the whole replay.
- **decoder_params.** P25: `HAMSPOT_P25_X`, WACN, SYS, NAC, RFSS, SITE; also `HAMSPOT_MODULATION`, HackRF LNA/VGA gains. Old recordings: P25 replay borrows the session's learned site word for the same radio.
- **Out of band.** Blocks tagged when the channel was >3 MHz from the HackRF centre (outside the usable 6 MHz); new recordings wait for the follower's retune.
- **Known gaps.** A recording that starts before the control channel reports its site word has no Phase 2 value. The P25 key-file path isn't saved (encrypted calls aren't recorded anyway). Hamlib/GQRX demod-audio recording (`source_kind = demod_audio`) is supported by the DB/player but not wired to REC keys.
- **Tests.** Automated coverage includes channeliser, live-vs-replay, segment states, encrypted discard, cap/overwrite, crash recovery, migrations, rescan, cache, archive, export/import, timeline layout and `commander-rec`. On-air checks are manual (P25 follow, all-traffic, scanner hold, encrypted call, retune mid-call, restart armed, decoder upgrade, full cap, drive move/re-attach).

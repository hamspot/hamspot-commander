# Hamspot Commander

A station console for amateur radio and scanner listening on Linux. One window drives Hamlib radios, GQRX and HackRF receivers; scans channel lists; follows trunked systems (P25 and others) with talkgroup and radio-ID lists; decodes digital voice through pipe files; records HackRF traffic as raw IQ and re-decodes it on playback; controls rotators and amplifiers; and includes a NanoVNA V2 screen, Smith chart, RF calculators, a logbook and station timers.

- **User guide:** [docs/UserGuide.md](docs/UserGuide.md)
- **For developers and agents:** [AGENTS.md](AGENTS.md)

## Install

On Ubuntu 26.04, from a checkout:

```bash
scripts/install.sh
```

See the user guide for the portable folder and a persistent live USB.

## Build from source

```bash
cargo build --release -p commander
```

Radio I/O goes through Hamlib's `rigctld` protocol; Commander starts its own `rigctld`, `gqrx`, `rotctld` and `ampctld` and doesn't link `libhamlib`.

## Layout

```
crates/rigctl/      rigctld client
crates/commander/   the egui window
crates/hsrec/       REC library and the commander-rec tool
modems/             shipped RX/TX pipe files
bsp/                band plans
scripts/            install, portable build and helper scripts
docs/               user guide, agent notes, design docs
```

## License

GPL-3.0-or-later. See `LICENSE`. The face uses [rustcars](https://github.com/webaugur/rustcars) (MIT, `licenses/rustcars-MIT.txt`); the bundled Antonio typeface is OFL 1.1 (`licenses/Antonio-OFL.txt`). The Tune layout loosely follows the Ham Radio Deluxe rig screen; Ham Radio Deluxe is a trademark of its owner and none of its icons, bitmaps or text are copied.

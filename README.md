# Hamspot Commander

A station console for amateur radio. Phase 1 is the rig-control face: frequency, S-meter, VFO and RIT keys, the ham-band scales, and the level sliders.

The layout follows the rig screen in the Ham Radio Deluxe 5.21 preliminary user guide (11 May 2012). That guide is the behavior reference for this screen. Logbook, Digital Master, satellites, and rotator control are later work and are not in this tree. Ham Radio Deluxe is a trademark of its owner. This project does not copy its icons, bitmaps, or manual text.

The window is a Rust [`egui`](https://github.com/emilk/egui) app in the same spirit as the RT-950 CPS: dark groups, a large frequency, and a click-to-tune bandspread. Radio I/O goes through Hamlib's `rigctld` extended response protocol (`+\` commands, `RPRT` replies). The program does not link `libhamlib`.

## Run

```bash
cargo run -p commander
```

The window opens on an in-process demo rig at 14.153.608 MHz USB. **Demo** returns to that rig. **Connect** talks to a `rigctld` that is already running. This program does not start `rigctld`.

```bash
rigctld -m 1 -t 4532
```

Model `1` is Hamlib's dummy rig. Use your radio's model number and serial port when you connect a real rig. Levels the backend lists with an empty range stay disabled.

## Layout

```
crates/rigctl/      rigctld client and the demo rig
crates/commander/   the egui face
```

## License

GPL-3.0-or-later. See `LICENSE`.

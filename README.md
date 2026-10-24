# Hamspot Commander

A station console for amateur radio. Phase 1 is the rig-control face: frequency, S-meter, VFO and RIT keys, the ham-band scales, and the level sliders.

The layout kinda follows the rig screen in the Ham Radio Deluxe 5.21 preliminary user guide (11 May 2012). That guide is the behavior reference for this screen. Ham Radio Deluxe is a trademark of its owner. This project does not copy its icons, bitmaps, or manual text, and it does not include that program's logbook.

The window is a Rust [`egui`](https://github.com/emilk/egui) app in the same spirit as the RT-950 CPS: dark groups, a large frequency, and a click-to-tune bandspread. Radio I/O goes through Hamlib's `rigctld` extended response protocol (`+\` commands, `RPRT` replies). GQRX uses the short rigctl commands on the same client. The program does not link `libhamlib`. Digital voice is an RX pipe around `dsd-fme`, not a mode in the menu.

The operator manual is `docs/UserGuide.md`. `docs/AGENT-HSCommander.md` is the field map for turning a listing into a `.HSCommander` station and the `.HSKeys` file beside it.

## Run

```bash
cargo run -p commander
```

The window starts Hamlib's dummy itself: `rigctld -m 1` on `127.0.0.1`, port 4532 when that port is free. VFO A is 14.153608 MHz USB, VFO B is 146.520 MHz FM, and VFO C is 446.000 MHz FM. **Demo** switches to a small in-process stand-in and does not spawn `rigctld`. The Config tab saves the station to one JSON file and adds radios. A hamlib row starts `rigctld`. A GQRX row starts GQRX. The program does not attach to a `rigctld` or GQRX it did not start.

`scripts/dummy-rig.sh` still starts that dummy by hand for a manual run. Levels the backend lists with an empty range stay disabled.

## Layout

```
crates/rigctl/      rigctld client and the demo rig
crates/commander/   the egui face
```

## License

GPL-3.0-or-later. See `LICENSE`.

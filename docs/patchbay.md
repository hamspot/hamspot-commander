# RF patch bay data model (#98)

Code: `crates/commander/src/patchbay/` — `model.rs` (devices, ports, cables, tracing; pure), `geom.rs` (layout and hit testing; pure), `view.rs` (egui painter and pointer).

Station file section `patchbay` (omitted when empty, `deny_unknown_fields`):

```json
"patchbay": {
  "devices": [
    {"id": 1, "kind": "radio", "name": "IC-9700", "radio": "IC-9700", "tx": true, "ways": 4, "sel": 1, "pos": [12.0, 12.0]},
    {"id": 2, "kind": "amplifier", "name": "Elecraft KPA1500", "model": 201, "tx": false, "ways": 4, "sel": 1, "pos": [220, 20]},
    {"id": 3, "kind": "antenna", "name": "Yagi", "bands": "2m", "tx": false, "ways": 4, "sel": 1, "pos": [420, 20]}
  ],
  "cables": [
    {"from": {"device": 1, "port": "ANT1"}, "to": {"device": 2, "port": "INPUT"}},
    {"from": {"device": 2, "port": "OUTPUT"}, "to": {"device": 3, "port": "FEED"}}
  ]
}
```

Kinds: `radio`, `amplifier`, `switch`, `tuner`, `meter`, `load`, `antenna`. A cable is always stored output → input (transmit sense). Radio devices are matched to station radio rows by `radio` (the row name) and follow that list.

Hooks for later hardware work:
- `model` on an amplifier is the Hamlib amp model number (`ampctl -l`), so an ampctld can be started per device (#96).
- `trace()` returns each radio's chains as device ids and cable indexes; a future TX interlock can refuse PTT when a TX path ends `Open`, loops, or runs into a receive-only radio (#97).
- Radio `ANT1`/`ANT2` map to Hamlib antenna selection; a switch's `sel` maps to an external switch driver (#97).

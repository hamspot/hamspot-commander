# Transcribing a listing into an HSCommander station

This is for an agent that has been given a listing, usually a RadioReference system page or an export the operator is allowed to read, and has been asked to produce a Hamspot Commander station. The operator manual is `docs/UserGuide.md`. The types that reject a bad file are `ConfigFile` in `crates/commander/src/config.rs`, `Channel` in `channels.rs`, `Talkgroup` in `talkgroups.rs`, and `KeyFile` in `keys.rs`. Every one of those structs uses `deny_unknown_fields`.

Write two files, and only the second when there is something to put in it:

- `Name.HSCommander`, the station. Version 1 JSON.
- `Name.HSKeys`, beside it, only when the operator gave you key ids or a dsd-fme hex CSV. Same base name, extension `HSKeys`. `City.HSCommander` pairs with `City.HSKeys`. Mode `0600`.

Do not put key hex in the station. Do not invent key bytes. Do not add a JSON field the structs do not have. Do not commit either file unless the operator asked, and do not commit a key file at all. A real system the operator keeps private belongs in their own checkout, not in an example in this tree.

## What you may read

Use a page, PDF, or CSV the operator already has the right to use. RadioReference's database and its prose are theirs. Copy the facts the station needs: system name, site name, frequency, control-channel mark, talkgroup number, talkgroup name, and whether the row is encrypted. Do not paste their descriptions, narrative, or a bulk dump into the repo. Do not log in as them, do not work around a subscription wall, and do not script their site. If the page is not in front of you, ask for an export or for the rows. If a column legend on that page disagrees with the usual letters below, follow the page.

A fictional system is used at the end of this file so the shape is concrete. Do not replace it with a real city's channels.

## Leave these alone

- Do not store WACN, system id, or NAC anywhere in the JSON. The control decoder learns them from the air. There is no field for them. `dsd-fme -X` is that site word, and it is not a key. Do not pass `-T`.
- Do not store `SCAN` or `DWELL` being on. Marked scan zones may be stored. Leave `scan_zones` empty unless the operator named the zones to limit.
- Do not fill `units`. Those rows are radios heard on a control channel. The window writes them.
- Do not fill `audio.devices` or `audio.links`. The window writes the PipeWire graph it sees.
- Do not fill `levels` with guesses. An empty object is valid.
- Do not set Gpredict `follow` on a radio. That flag means Gpredict is moving the dial. Talkgroup Follow is `control_follow`, and a new file should leave it empty so Follow starts off.
- Do not put the HackRF serial, a sound-card name, or a speaker node in the file unless the operator handed you that exact string from this machine. An empty `device` means the only HackRF. An empty `audio.output` lets the window use the default sink.
- Do not create the key file by hand with real key material you were not given. A placeholder is a row with no `text`.

## Station skeleton

Required top-level fields:

| Field | What to write |
| --- | --- |
| `version` | `1` |
| `bsp_name` | `publicsafety.json` for a public-safety listing, `default.json` for amateur. One file name, no slash. |
| `bsp` | Copy that file from `bsp/` in this repo into the station. Do not author a new plan. Opening the station will not overwrite a `bsp/` file that is already there. |
| `channels` | Version 1, at least one named zone. |
| `talkgroups` | Version 1, at least one named zone. Use one empty zone named `Talkgroups` when the listing has no talkgroups. |
| `tune` | See below. |
| `levels` | `{}` unless the operator gave you values the radio already uses. The field is required. |
| `radios` | `[]` unless the operator asked for a row. An empty list starts the Hamlib dummy. |
| `audio` | `{ "devices": [], "links": [] }` |
| `rotators` | `[]` |

Omit a field that the struct skips when empty, or include it empty. Both load. Do not invent names. Optional fields you may set: `selected` (radio index, omit for 0), `page` (`"Channels"` is a useful landing), `channel_zone` and `channel_sel` (which site opens), `talkgroup_zone`, `talkgroup_sel`. Leave `control_follow`, `trunk_receivers`, `scan_zones`, `monitor_zones`, `monitor_talkgroups`, `tabs`, `pulse`, `pulse_secs`, and `nanovna` out unless the operator asked. `pulse` defaults on. `pulse_secs` defaults to 2. `nanovna` is the NanoVNA V2 port and sweep. A missing key is the built-in sweep (50 MHz to 900 MHz, 101 points). Do not put calibration standards or zone scans in the station file. Those records live in `Name.HSVNA` beside the station, in the same JSON shape: `cals` entries may include `zone`, and `scans` entries are the most recent sweep of a zone. The NanoVNA screen keeps one calibration set for each plan zone, from that zone's lowest frequency to its highest, and leaves `band` empty. Do not invent calibration standards. `tabs` is the page-tab order: `Tune`, `Plan`, `Channels`, `Meters`, `Timers`, `Levels`, `Keys`, `Tools`, `Config`, plus `Talkgroups:<radio id>` and `RadioIds:<radio id>`. The on-screen labels for those last two are TG and ID. `Both` is the trunk role when `trunk_receivers` has no row for that radio. `control` and `voice` are the stored role strings when they did ask for a dedicated receiver. One dongle cannot be both a dedicated control receiver and a dedicated voice receiver.

`tune` must include `show_spread`, `step_hz` (greater than 0, `100` or `1000` is fine), `zone_sel`, `band_sel`, `band_mem` (object, may be `{}`), `host` (`"127.0.0.1"`), `port` (`"4532"`), `freq_hz` (hertz, greater than 0), `mode`, `passband_hz` (0 or a positive width), `sub_freq_hz` (0 is fine), `sub_mode`, `rit_hz`, and `split`. For a trunked FM system set `mode` to `"FM"` and `freq_hz` to the primary control channel in hertz. `favorite` and `mem_hz` may be omitted.

## Frequencies

Listings are in megahertz. The file is in hertz. Multiply with decimal arithmetic and round to the nearest hertz. `859.9625` MHz is `859962500`. Do not leave a binary-float remainder. A match within 500 Hz is the same memory for the encrypted-channel check. Two memories closer than that are still two rows. Prefer one row.

A 12.5 kHz or 6.25 kHz listing ends in `.x000`, `.x125`, `.x250`, `.x375`, `.x500`, `.x625`, `.x750`, or `.x875` when read in megahertz to four places. If the source prints fewer places, keep what it printed. Do not invent the dropped digits.

## One zone per site

The follower's control list is the open channel zone and no other zone. Make one channel zone per site. Put that site's control channels and its voice channels in that zone. Set `channel_zone` to the site the operator named. If they did not name one, use the site whose control channels sit closest together, and say which site you opened.

Zone name: the site number and the site name from the listing, for example `1.037 Post`. The name is not empty and it is not matched against the band plan.

Sort channels by frequency inside the zone. The program does not require that order. It makes the list readable.

## Channel rows

Allowed fields: `name`, `hz`, `mode`, `control`, `encrypted`, `vocoder`, `invert`.

- `mode` is `"FM"` for a conventional FM channel and for a trunked P25 or DMR frequency. The discriminator is FM. The vocoder is separate.
- `control` is true only for a control channel. Listings mark these `c`, `cc`, bold, or a control-channel column. The name used in this project puts `c` on the end of the megahertz, as in `859.9625c`. The boolean is what the program reads. The suffix is for the person looking at the list.
- `encrypted` is true when that frequency is encrypted-only. An encrypted control is dropped from the control list, so do not mark a clear control channel encrypted because some talkgroups on the system are encrypted. Talkgroup encryption belongs on the talkgroup row.
- `vocoder` is omitted on trunked voice and control channels. Follow uses the radio's RX pipe, not the per-channel vocoder. Set `vocoder` on a conventional digital memory that is not part of the trunk: `p25`, `p25p2`, `dmr`, `nxdn48`, `nxdn96`, `dstar`, `ysf`, `dpmr`, `m17`, and the other names in `VOCODER_CHOICES`. `p25p2` is the conventional Phase 2 vocoder. It is not the trunk pipe.
- `invert` is only for `dmr`, `nxdn48`, `nxdn96`, and `dpmr`, and only when the listing says the polarity is inverted. Otherwise omit it.

Voice channels still belong in the zone. They are the memory list and the scanner. The P25 channel map that the follower hops with is learned from the control channel, not from these rows.

## System type and the RX pipe

Set `rx_pipe` only on a radio row you were asked to write. The value is the script's file name.

| Listing says | Trunk pipe on the radio | Channel `vocoder` for a conventional memory |
| --- | --- | --- |
| Project 25, Phase 1, Phase 2, or both | `commander-demod-p25.rxpipe` | `p25` or `p25p2` |
| DMR, including Tier III | `commander-demod-dmr.rxpipe` | `dmr` |
| NXDN trunk, 6.25 kHz | `commander-demod-nxdn48-trunk.rxpipe` | `nxdn48` |
| NXDN trunk, 12.5 kHz | `commander-demod-nxdn96-trunk.rxpipe` | `nxdn96` |
| Analog conventional, or a mixed channel in FM | none | omit |
| EDACS, ProVoice, OpenSky, or anything else | none, unless a script in `modems/` is named for it and the operator asked | the matching `VOCODER_CHOICES` value, or omit |

`commander-demod-p25.rxpipe` is the trunk pipe. It follows Phase 1 control and Phase 2 voice in one decoder. Do not point a trunked radio at `commander-demod-p25p2.rxpipe`. That script is the conventional Phase 2 vocoder.

A HackRF template, when the operator asked for one, is a radio object:

```json
{
  "driver": "hackrf",
  "name": "HackRF",
  "model": 0,
  "device": "",
  "park": false,
  "follow": false,
  "audio": { "input": "", "output": "", "rx": true, "tx": false },
  "rx_pipe": "commander-demod-p25.rxpipe"
}
```

`model` is ignored for HackRF. Leave `lna_db` and `vga_db` out so the window keeps the gain file it already has. Do not invent gain numbers.

An audio tap is `"driver": "tap"`. It has no rig and no tuning. Use it only when the operator said the audio is a discriminator from a radio they already programmed. `audio.input` is that capture node. `audio.output` is the speaker. `rx_pipe` is the demodulator script. Leave `tx_pipe` empty.

## HackRF window

The capture is 8 Msps with a 6 MHz filter. The follower keeps the widest run of clear control channels that fits in 6 MHz, at most 8 of them, and centers the LO on the midpoint of that run. A control more than 3 MHz from that center is outside the window. Say so in your report when a site has controls that far apart. Do not drop those rows from the file. The scanner can still visit a memory outside the window. Do not retune the file to hide the gap.

A voice grant inside the window stays on that center. A voice grant outside it, with Follow on and scan off, can leave the control channel until the call ends. That behavior is the program's. You do not encode it in JSON.

## Talkgroups

Allowed fields: `name`, `tg`, `encrypted`.

- `tg` is the decimal talkgroup. RadioReference prints decimal and hex. Store decimal. `10006` and `0x2716` are the same number. `0` is not a talkgroup. The field is a JSON number, up to 16777215.
- `name` is the alpha tag. When the tag is empty, use the description's short name, then the decimal digits. Do not paste a paragraph.
- `encrypted` is true when that row is encrypted. Usual mode letters, when the page's own legend matches them: a trailing `E`, or `DE` / `TE`, means encrypted. `D` and `T` alone do not. `A` is analog and not encrypted. A separate Encrypted column wins over the mode letter.
- One zone per category the listing already groups (law, fire, public works), or one zone for the whole system when the export is flat. Zone names are not empty. Do not create a zone named `Discovered`. The window adds that zone for talkgroups it hears that are not in the file.
- The same decimal in two zones is one talkgroup. It is encrypted if any of those rows says so. Prefer one row.
- Leave `monitor_zones` and `monitor_talkgroups` empty. Empty means every clear talkgroup in the window is heard. A filled key also allows an encrypted talkgroup. A double-click in the window is how the operator limits the list later.

Color code, time slot, RAN, and NAC are not talkgroup fields. Record them in your report if the operator will need them on a radio you are not programming here. Do not add them to the JSON.

## Keys

Create `Name.HSKeys` only if at least one of these is true:

- The operator gave you a key id the decoder asked for, or a key id from the listing.
- The operator gave you key hex, or a dsd-fme hex CSV.

If they gave you neither, do not create the file. The window creates a placeholder later when a decoder prints a missing id.

The file:

```json
{
  "version": 1,
  "keys": [
    { "key_id": "1A2B", "alg": "81", "name": "local test" }
  ]
}
```

- `key_id` is four uppercase hex digits. `0000` is refused. A DMR id `1` is stored as `0001`. One row per id.
- `alg` is two uppercase hex digits. P25 DES-OFB is `81`. P25 RC4 / ADP is `AA`. P25 AES-256 is `84`. Any other byte may be stored. Leave `text` off that row. Commander does not pass Hytera, TYT, or DMR privacy to `dsd-fme`.
- `name` is a short label. Omit it when you have none.
- `text` is the key hex, uppercase, no spaces, no `0x`. Omit the field when you do not have the key. That row is a placeholder. It is saved. It is not sent to the decoder.
- DES and RC4 text is 16 hex digits. AES-256 text is 64 hex digits. Any other length is refused if you include `text`. A wrong length must not be written.
- Do not generate a key unless the operator asked for a local debug key. The Keys page Generate button does that from `/dev/urandom`. A generated value is not a system key.

Write the file mode `0600`. On Linux, create it with that mode and then set the mode again, because umask can widen the create. Pretty JSON and a trailing newline match the other files. Do not log the hex.

A dsd-fme hex CSV is the Import and Export format. The first line is the header and is skipped:

```
key,word0,word1,word2,word3
0004,0011223344556677
0003,1111111111111111,1111111111111111,1111111111111111,1111111111111111
1A2B
```

AES-256 is four 16-digit words. DES and RC4 are one word. A key id with no words is a placeholder. A word of all zeros is empty text. The CSV has no algorithm and no name. A new 16-digit row imports as DES. A 64-digit row imports as AES-256. Re-import onto a row that already exists keeps that row's algorithm and its name. If the operator's keys are RC4, write the HSKeys row with `"alg": "AA"` yourself. Do not rely on a fresh CSV import to remember RC4.

The example digits above are not a key. Do not copy them into a station as if they were.

## Check before you hand it over

1. The station parses. Unknown fields, an empty zone name, a frequency that is not a finite number above zero, a talkgroup file with no zones, or `version` other than 1 will fail the load and the window will keep whatever it already had.
2. Every `control: true` row is a control channel, and no clear control is marked `encrypted`.
3. Hertz values are rounded integers. The primary control matches `tune.freq_hz`.
4. `bsp_name` is `publicsafety.json` or `default.json`, and `bsp` is that file's object, not a plan you wrote.
5. The key file, if you wrote one, is mode `0600`, has no `0000` id, and has no `text` unless the operator supplied those digits.
6. You did not start Commander, and you did not commit.

In the report, name the site you selected, how many zones and channels and talkgroups you wrote, how many controls fall outside a single 6 MHz window, and whether a key file exists. Do not print key hex in the report.

## A fictional station

This listing is invented. North County is not a real system. The plan in `bsp` is a one-band stand-in so the example fits on a page. A file you deliver copies `bsp/publicsafety.json` or `bsp/default.json` instead.

Sites:

| Site | MHz | Role |
| --- | --- | --- |
| 1.001 Courthouse | 855.2125 | control |
| 1.001 Courthouse | 856.4625 | voice |
| 1.001 Courthouse | 857.9625 | voice |

Talkgroups: `101` Sheriff (digital), `202` Jail (digital, encrypted). No key was provided, so there is no `.HSKeys` file.

`North-County.HSCommander`:

```json
{
  "version": 1,
  "bsp_name": "publicsafety.json",
  "bsp": {
    "version": 1,
    "zones": [
      {
        "name": "UHF",
        "color": [130, 136, 150],
        "bands": [
          {
            "name": "800",
            "chip": "800",
            "lo_hz": 851000000,
            "hi_hz": 862000000,
            "home_hz": 855212500,
            "mode": "FM",
            "segments": [
              {
                "lo_hz": 851000000,
                "hi_hz": 862000000,
                "use": "voice",
                "service": "government"
              }
            ]
          }
        ]
      }
    ]
  },
  "channels": {
    "version": 1,
    "zones": [
      {
        "name": "1.001 Courthouse",
        "channels": [
          {
            "name": "855.2125c",
            "hz": 855212500,
            "mode": "FM",
            "control": true,
            "encrypted": false
          },
          {
            "name": "856.4625",
            "hz": 856462500,
            "mode": "FM",
            "control": false,
            "encrypted": false
          },
          {
            "name": "857.9625",
            "hz": 857962500,
            "mode": "FM",
            "control": false,
            "encrypted": false
          }
        ]
      }
    ]
  },
  "talkgroups": {
    "version": 1,
    "zones": [
      {
        "name": "Law",
        "talkgroups": [
          { "name": "Sheriff", "tg": 101, "encrypted": false },
          { "name": "Jail", "tg": 202, "encrypted": true }
        ]
      }
    ]
  },
  "tune": {
    "show_spread": true,
    "step_hz": 1000,
    "zone_sel": 0,
    "band_sel": 0,
    "band_mem": {},
    "host": "127.0.0.1",
    "port": "4532",
    "freq_hz": 855212500,
    "mode": "FM",
    "passband_hz": 15000,
    "sub_freq_hz": 0,
    "sub_mode": "",
    "rit_hz": 0,
    "split": false
  },
  "levels": {},
  "radios": [],
  "audio": { "devices": [], "links": [] },
  "rotators": []
}
```

If the operator later says key id `0007` is AES-256 and does not give you the hex, the whole key file is:

```json
{
  "version": 1,
  "keys": [
    { "key_id": "0007", "alg": "84", "name": "Jail" }
  ]
}
```

That placeholder does not play the Jail talkgroup. A filled `text` of 64 hex digits, supplied by the operator, would.

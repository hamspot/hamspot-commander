# Hamspot Commander User Guide

Hamspot Commander is a station console for amateur radio and scanner listening on Linux. This guide covers the program as it is today: how to install it, how to find your way round the window, and how to do the everyday jobs: tune, scan, follow a trunked system, record and replay traffic, sweep an antenna with a NanoVNA, and write your own pipe files.

The program is licensed under the GNU General Public License, version 3 or later. See `LICENSE` in the source tree.

## Contents

1. [Introduction](#ch1)
2. [Quick start](#ch2)
3. [Installing](#ch3)
4. [The window](#ch4)
5. [Tune](#ch5)
6. [Plan: band plans](#ch6)
7. [Channels and scanning](#ch7)
8. [Trunked systems: Talkgroups, Radio IDs and Keys](#ch8)
9. [REC: recording and playback](#ch9)
10. [Meters, Levels and Timers](#ch10)
11. [Config: radios, audio and automation](#ch11)
12. [Tools](#ch12)
13. [Pipe files](#ch13)
14. [How-to recipes](#ch14)
15. [Files and folders](#ch15)
16. [Troubleshooting](#ch16)
17. [Glossary](#ch17)
18. [Recording, the law and your privacy](#ch18)
19. [Roadmap](#ch19)

Screenshots are being collected. Where one belongs, the Markdown source has a `<!-- screenshot: … -->` marker.

---

<a id="ch1"></a>

# 1. Introduction

## 1.1 What Commander does

Commander puts the controls of one or more radios in one dark, LCARS-style window:

- **Rig control** for any radio Hamlib can drive, through `rigctld`, which Commander starts for you.
- **SDR receive** with GQRX (started by Commander) or a HackRF read directly.
- **Scanning** a local channel file, and **following a trunked system** (P25 and others) from its control channel, with talkgroup lists and the radio IDs heard on the air.
- **Digital voice** through RX pipes, small scripts that run decoders such as `dsd-fme`.
- **REC**, which records HackRF traffic as raw radio data and re-decodes it every time you play it back.
- **Automation**: Hamlib rotators and amplifiers, and an RF patch drawing of your station.
- **Tools**: a NanoVNA V2 vector network analyzer screen, a Smith chart, RF and general calculators, a hex viewer, a logbook and a rotator display. The **Timers** page has station clocks, an ID timer, a countdown and a stopwatch.

Everything about a station (radios, band plan, channels, talkgroups, levels, layout) is kept in one **station file**, `Name.HSCommander`. Key material, VNA calibration, the logbook and recordings are kept in separate files beside it ([chapter 15](#ch15)).

## 1.2 What it does not do

- It is not a contest logger, ADIF log or DX cluster client. The Logbook is a time-stamped station diary.
- It is not a satellite tracker. A radio row can follow Gpredict instead.
- The Channels page is a file on your computer, not the memory bank inside the radio.
- It does not attach to a `rigctld` or GQRX that something else started. It starts its own and stops only what it started.

## 1.3 What you need

| Item | Notes |
| --- | --- |
| Computer | 64-bit Linux with a graphical desktop and PipeWire. The installer supports Ubuntu 26.04 only. |
| Groups | Your user in `dialout` (serial radios) and `plugdev` (some USB devices). Log out and in after a change. |
| `rigctld`, `rotctld`, `ampctld` | Ubuntu package `libhamlib-utils`. |
| `gqrx` | Package `gqrx-sdr`, for GQRX rows. |
| `pw-loopback`, `pw-cat`, `pw-dump` | Package `pipewire-bin`. |
| `helvum` | Optional PipeWire patchbay, opened from the Audio frame. |
| `dsd-fme`, `sox`, `socat` | Digital voice pipes. The installer builds `dsd-fme`. |
| `hackrf_transfer` | Package `hackrf`, for HackRF rows. |

Supported test equipment is the **NanoVNA V2** (V2.2, USB id `04b4:0008`). Supported SDR hardware for raw capture and REC is the **HackRF**.

## 1.4 Conventions in this guide

- Button and field names are printed as they appear, for example **Add hamlib** or `Band +`.
- **Click modes.** Some keys do different things on a single, double or triple click. They show three small dots on their right edge: green for click, orange for double-click and purple for triple-click. On those keys **Shift+click** is the same as a double-click and **Ctrl+click** the same as a triple-click. The tooltip lists all three.
- Warnings are always spelled out in words. Colour is only a second cue.

---

<a id="ch2"></a>

# 2. Quick start

1. **Install.** On Ubuntu 26.04, from a source checkout, run `scripts/install.sh` ([3.1](#s3-1)). For a USB stick, use the portable folder ([3.3](#s3-3)).
2. **Start it** from the desktop menu (**Hamspot Commander**) or with `~/.local/bin/commander`. The window opens on **Tune**. With no station loaded, Commander runs Hamlib's dummy radio (model 1), so the controls work straight away.
3. **Save a station.** Open **Config** and press **Save**. Choose a folder and a name such as `Home.HSCommander`. Pages such as Keys, Logbook and REC need a saved station, because their files go beside it.
4. **Add your radio.** On **Config**, press **Add hamlib** for a CAT radio, **Add GQRX** for an SDR through GQRX, or **Add HackRF** for a HackRF. Fill in the window and press **Save** ([chapter 11](#ch11)). Click the row to make it the radio that Tune drives. Wait for its **Status** to stop saying `starting`.
5. **Listen.** On **Tune**, click a band chip, or type a frequency under the digits and press Enter ([chapter 5](#ch5)). Wire the radio's RX card to your speakers in the **Audio Patch** group on Config ([11.6](#s11-6)).
6. **Scan.** On **Channels**, press **LOAD** to open a channel file or **Add** to store the current frequency, then press **SCAN** ([chapter 7](#ch7)).
7. **Save again** on Config when you have changed things you want to keep. When you close the window with unsaved changes, Commander asks **Save**, **Don't save** or **Cancel**.

<!-- screenshot: main-01 — Commander's main window: top bar, page row and the Tune page. -->
<!-- screenshot: station-03 — A new station with no radios yet. -->

---

<a id="ch3"></a>

# 3. Installing

<a id="s3-1"></a>

## 3.1 Installed copy (Ubuntu 26.04)

From a source checkout:

```bash
scripts/install.sh
```

The script checks `/etc/os-release` and stops on anything other than Ubuntu 26.04. It uses `sudo` for apt, so run it from a terminal that can authenticate. It then:

- installs the packages in [1.3](#ch1) plus the build tools;
- builds the release program and the upstream `mbelib` and `dsd-fme`;
- installs the shipped pipe files ([chapter 13](#ch13)).

| Installed path | What it is |
| --- | --- |
| `~/.local/bin/commander` | The program |
| `~/.local/bin/dsd-fme` | Digital voice decoder |
| `~/.local/share/hamspot-commander/modems/` | Shipped pipe files (also linked into `~/.local/bin`) |
| `~/.local/share/applications/hamspotcommander.desktop` | Desktop menu entry |
| `~/.local/share/icons/hicolor/128x128/apps/hamspot-commander.png` | Icon |
| `~/.local/share/hamspot-commander/installed.manifest` | What this install added |

You are added to `dialout` or `plugdev` only if you were not already a member. Log out and back in if a group was added. The script does not start Commander.

The command-line tool `commander-rec` ([9.10](#s9-10)) is built with the program as `target/release/commander-rec`. Copy it onto your `PATH` if you want to use it from cron.

## 3.2 Removing an installed copy

```bash
scripts/uninstall.sh
```

This removes what the manifest lists under your home directory, and takes you out of a group only if the install added you. Apt packages, your station files and your recordings stay.

<a id="s3-3"></a>

## 3.3 Portable folder and a persistent live USB

The portable build is a folder, `HamSpotCommander`, that keeps its settings inside itself, so it can live on a USB stick. `scripts/portable-install.sh` builds it from a source checkout. Start it with `HamSpotCommander/hamspot-commander.sh`. The script finds its own folder and points the config, data, cache and state directories into it.

To run it from a persistent Ubuntu live USB:

1. Make the stick with **mkusb**, choosing its persistent-live operation. The desktop Startup Disk Creator does not add persistence. Use a stick of 32 GB or more.
   > **Important.** mkusb erases the drive you pick. Match the device name to `lsblk` before you accept.
2. Boot the **persistent** menu entry. Check with `findmnt -T /` (an overlay on `/`) and `lsblk -o NAME,LABEL,SIZE,FSTYPE,MOUNTPOINT` (a partition labelled `writable` or `casper-rw`).
3. Copy the folder into your home directory, for example `~/Applications/HamSpotCommander`. Don't run it from a FAT or NTFS partition, which drops the executable bit. Check with `test -x ~/Applications/HamSpotCommander/bin/commander && echo OK`.
4. Install the packages: `sudo apt-get install gqrx-sdr libhamlib-utils pipewire-bin helvum`.
5. Add yourself to the groups with `sudo usermod -aG dialout,plugdev "$USER"`, then log out and back in.
6. Start it with `~/Applications/HamSpotCommander/hamspot-commander.sh`. Don't set a private `XDG_RUNTIME_DIR` first, or audio will not reach PipeWire.

Reboot into the persistent entry again and check that your station is still there. A plain "Try Ubuntu" boot starts clean and shows neither the folder nor the packages.

| Inside `HamSpotCommander` | Role |
| --- | --- |
| `hamspot-commander.sh` | Launcher |
| `bin/commander` | The program |
| `bsp/*.json` | Band plans |
| `config/` | A convenient place for station files |
| `data/`, `cache/`, `state/` | Window state, GQRX settings and other program data |
| `hamspotcommander.desktop` | Desktop entry that follows the folder |

---

<a id="ch4"></a>

# 4. The window

<a id="s4-1"></a>

## 4.1 Page row

The top of the window is one row of page tabs. The default order is **Tune**, **Plan**, **Channels**, then any **TG** and **ID** tabs, then **REC**, **Meters**, **Timers**, **Levels**, **Keys**, **Tools** and **Config**.

- A radio that follows a trunked system adds a **TG** tab (its talkgroups) and, once radios have been heard, an **ID** tab. The label is the radio's name plus TG or ID, for example `HackRF TG`.
- **Rearrange** tabs by dragging one off the row and dropping it on another tab. A small sideways nudge still counts as a click. The order is saved in the station file.
- **Voice activity.** The Channels tab and each TG tab show a speaker. It is dim when quiet, solid while voice is coming in, and fades over about two seconds of silence. The tab also pulses while voice is up. Set the pulse on the Config page (**Pulse busy tabs**, **Pulse speed**). Turning animation off in the desktop settings stops the pulse.
- **REC tab icon.** Black means no REC key is lit, dark red means armed, and bright red means a recording is being written. Hover it for the same words.

<!-- screenshot: main-02 — The top bar shows the active radio and its status. -->

## 4.2 Separate windows

- **Tools** can be torn off: on the Tools page, the square at the right end of the tab row opens the tools in their own window. Close that window to put them back.
- **Meters** has a **Detach** button that opens it in its own window, and **Dock** puts it back.

<!-- screenshot: main-03 — Any page can be torn off into its own window. -->

## 4.3 When the radio does not answer

Look at the **Status** column on the Config page.

| Status | Meaning | What to do |
| --- | --- | --- |
| `starting` | Commander has launched the radio program; its port isn't open yet. | Wait a moment. |
| `device missing` | The serial device named in the row isn't on the machine. | Plug the cable in. No restart is needed. |
| `choose I/O device` | GQRX is running and waiting for you to pick its input. | Pick the device in GQRX's window. |
| `retrying` | The radio program exited. Commander starts it again about 2 s later. | Check the cable, model and serial speed. |

A frequency the radio refuses doesn't move the readout.

<!-- screenshot: main-04 — What Commander shows when the radio is not responding. -->

---

<a id="ch5"></a>

# 5. Tune

<!-- screenshot: tune-01 — The Tune page. -->

The Tune page has the frequency frame (digits, VFO keys, the paired VFO), a band row (`ALT`, `BSP`, `LOAD`, band chips, `Mode`, `Sub`, `Receiver`), the optional bandspread, and a control row of radio functions.

## 5.1 The frequency readout

The readout shows four MHz digits, then kHz and Hz groups: 14.153608 MHz is `0014.153.608`. The left readout is the current VFO, with the S-meter. The right readout, when the radio has one, is the paired VFO (A with B, Main with Sub). Tuning the right digits sets the split frequency.

| Action | Result |
| --- | --- |
| Click a digit | Select that place and add one to it |
| Wheel on a digit | Step that digit up or down, one step per notch |
| Wheel elsewhere on the VFO block | Step the selected digit |
| Drag the **Fine** scale under the digits | Fine-tune within the current window |

The line under the digits shows the step size, such as `step 1 kHz`.

<!-- screenshot: tune-02 — Scroll or click a digit to tune by that step. -->

## 5.2 Typing a frequency

Click the text under the digits, type, and press Enter.

| You type | Meaning |
| --- | --- |
| `0014.153.608` or `14.153.608` | 14.153608 MHz (three groups) |
| `14.153608` | 14.153608 MHz (two groups are MHz) |
| `14153608` | 14153608 Hz (a whole number is Hz) |

A frequency outside the radio's receive ranges is ignored.

<!-- screenshot: tune-03 — Type a frequency directly. -->

## 5.3 VFO keys, mode and receiver

A key lights only when the radio reports it can do that operation.

| Key | Action |
| --- | --- |
| `A <> B` (and `ALT`) | Exchange the VFOs |
| `A > B` | Copy A to B |
| `A > Mem`, `Mem > A` | Store and recall one frequency held by the window |
| `TX main`, `TX sub` | Split off or on |
| `VFO A`, `VFO B` | Select a VFO |
| `R-6kHz`, `R-3kHz`, `R-15kHz`, `R-Auto` | Set or clear the RIT offset |

`Mode` lists the modes the radio reports and keeps the passband. `Sub` sets the paired VFO's mode when split is possible. `Receiver` is one toggle (● on, ○ off) that powers the radio over CAT when it supports that.

<!-- screenshot: tune-04 — VFO, mode and receiver controls. -->

## 5.4 Band chips and the bandspread

- **Band chips** show the bands of the current plan zone that your radio can receive. Click one to go back to where you last were in that band, or to its home frequency.
- `Band +` and `Band -` step to the next receivable band, skipping bands outside the radio's range.
- `BSP` shows the **bandspread**: one coloured line per plan zone. On a ham plan, amber is CW, blue is digital, green is voice and grey is mixed.
  - **Left-click or drag** a segment to tune the main VFO and set a matching mode (CW for CW segments, RTTY or similar for digital, USB for voice unless the band's mode already suits).
  - **Right-click or drag** to set the split frequency and mode without moving the main VFO.
- `LOAD` on this row loads a band plan ([chapter 6](#ch6)).

<!-- screenshot: tune-05 — Band chips and the bandspread for the current band. -->

## 5.5 Control row

| Key | Function |
| --- | --- |
| `ATU` | Antenna tuner |
| `NR` | Noise reduction |
| `Pre 1`, `Pre 2` | Preamp |
| `Ant 1`, `Ant 2` | Antenna select |
| `Bk-in` | Semi break-in |
| `Fast` | Fast AGC |
| `Proc` | Speech processor |
| `Lock` | Dial lock |
| `Notch` | Automatic notch |
| `MOX` | Transmit |
| `Moni` | Monitor |

A key is dark when the radio is offline or doesn't support that function. `Dimmer`, `NR sub`, `CW Spot`, `Keyer`, `Nar`, `Ant Rx`, `Chan +`, `Chan -`, `QMR`, `BC`, `PWR`, `QMS`, `Contour`, `Scan` and `Scan Dn` are always dark in this version.

## 5.6 Transmit and the band plan

> **Important.** `MOX` keys the transmitter.

Before keying, `MOX` checks the loaded band plan: voice is refused on CW and digital segments, digital is refused on CW and voice segments, CW is allowed everywhere, and mixed segments allow anything. A refusal is shown as a note on the page and the transmitter isn't keyed. The plan never stops you from listening.

Tuning stays inside the receive ranges the radio reports. A step that would leave the current range stops at its edge.

<!-- screenshot: tune-06 — The band plan stops transmit outside your privileges. -->

---

<a id="ch6"></a>

# 6. Plan: band plans

A band plan is a JSON file of **zones**, each holding **bands**, each with edges, a home frequency and mode, and segments marked `cw`, `digital`, `voice` or `mixed`.

| Shipped plan (`bsp/`) | Contents |
| --- | --- |
| `default.json` | Amateur bands, loaded at startup |
| `arrl.json` | ARRL voluntary plan |
| `shortwave.json` | Listening map: broadcast, aero, marine, time signals |
| `publicsafety.json` | Public-safety allocations |
| `indiana-railroads.json`, `indy-business.json` | Local example listening maps |

The station file keeps a copy of its plan, so a station always opens with the plan it was saved with.

## 6.1 The Plan page

- **LOAD** and **SAVE** open and save plan files. **ADD** adds a zone with one band around the current frequency.
- **Right-click a zone name** to rename or delete it.
- **More zones than fit:** the ones that don't fit leave the row, and the rounded end of the tan bar above gets a thin outline and a small ▼ mark. Click it (or focus it and press Enter) for a list of the hidden zones, then pick one. The zone you have open always stays in the row.
- **Click a band row** to select it and tune its home frequency.
- The editor on the right edits the band's Name, Home, Low, High and Mode. Click the dial inside a band to set its home.

Nothing is written to disk until you save. A file that doesn't parse, or has overlapping bands, is refused and the plan already loaded stays; the page shows the error. Bands may touch at an edge.

<!-- screenshot: plan-01 — The Plan page. -->
<!-- screenshot: plan-02 — What a malformed band-plan file looks like. -->

---

<a id="ch7"></a>

# 7. Channels and scanning

<!-- screenshot: chan-01 — The Channels page and its memory file. -->

## 7.1 Channel files

The Channels page holds named frequencies grouped in zones. It's stored in the station file and can be loaded from or saved to a separate `channels.json` with **LOAD** and **SAVE**.

- **Click a row** to tune it (mode too, if the radio can set it).
- The editor has **Name**, **Frequency**, **Mode**, **Vocoder** (the digital decoder for this channel), **Control** (a trunking control channel), **Encrypted**, and **Invert** for DMR, NXDN and dPMR. **Add** stores the current frequency and mode, **Delete** removes the selected channel.
- The strip above the list draws the band plan across the zone's frequency span, so you can see where each channel sits.

## 7.2 Scanning

- **SCAN** steps through the channels and stops on activity. **DWELL** holds on the current channel.
- **Double-click a zone** to mark it for scanning. With zones marked, SCAN covers only those (**Selected zones.**); with none marked it covers the zone you have open. Marks are saved; whether SCAN or DWELL is on is not.
- Encrypted channels are skipped unless a matching key is filled on the Keys page ([8.4](#s8-4)).

<!-- screenshot: chan-02 — The strip shows which channel has voice right now. -->
<!-- screenshot: chan-03 — Scanning a channel list. -->

---

<a id="ch8"></a>

# 8. Trunked systems: Talkgroups, Radio IDs and Keys

## 8.1 Following a control channel

1. On **Channels**, mark the system's control channels with **Control** (all in one zone).
2. On the radio's row, choose a trunking pipe (for P25, `modems/commander-demod-p25.rxpipe`) in its Audio row ([11.6](#s11-6)).
3. On **Config**, tick **Follow** for that radio. The follower then reads the control channel and tunes to voice grants for talkgroups you monitor.

A HackRF can follow wideband: voice channels inside its capture window are decoded while the control decoders keep running.

<!-- screenshot: chan-04 — Following a P25 control channel and holding a call. -->

## 8.2 Holding a call

Once a receiver is on a voice channel it **holds** that call: other grants don't move it, and the readout keeps the talkgroup, radio ID and alias of the call.

When the call ends, the **voice hang** starts: the readout shows `· HANG` and the audio stays up, so a quick re-key picks the call back up. After the hang, the radio returns to the control channel. A channel that syncs but gives no voice for 6 seconds is released. Encrypted voice with no key is released at once.

| Config setting | Choices | Meaning |
| --- | --- | --- |
| **Trunk voice hang** | 500 ms (default), 1500 ms, 3000 ms | How long to wait for a re-key |
| **Emergency/priority override** | on (default) / off | When on, only an emergency call or a **Priority** talkgroup can take a receiver off a held call |

## 8.3 Talkgroups (TG tab) and Radio IDs (ID tab)

The **TG** tab is the talkgroup directory for one radio, in zones. Each row has a number and a name, and can be marked **Encrypted** or **Priority**. **FOLLOW** turns following on for that radio. **LOAD** and **SAVE** read and write talkgroup files, **Add** and **Delete** edit the list.

The **ID** tab lists the radio IDs heard on that control channel, with what was learned about each (talkgroups, site, emergency or encrypted use). Commander fills it in and stores it in the station file. Select an ID to add a **Description**.

<!-- screenshot: chan-05 — The talkgroup directory learned from the control channel. -->
<!-- screenshot: chan-06 — Radio IDs seen on the control channel. -->

<a id="s8-4"></a>

## 8.4 Keys

The **Keys** page holds the encryption keys you are entitled to use. They're kept in `Name.HSKeys` beside the station (mode 0600), never in the station file. The page needs a saved station.

- Columns are Key ID, Algorithm, Status and Name. A missing key is flagged under Status.
- **Add** adds the next free ID as a blank DES key. **Remove** deletes one. **Import** and **Export** read and write a dsd-fme hex CSV. **Generate** fills random key text for a radio you program yourself. **Save** writes the key file.
- Key IDs the decoder reports are added automatically with blank text.
- DES, RC4 and AES-256 can be filled. With no filled key, encrypted talkgroups and channels aren't scanned or played.

<!-- screenshot: keys-01 — Station keys are stored outside the station file. -->
<!-- screenshot: chan-07 — Encrypted calls are marked and not decoded. -->

---

<a id="ch9"></a>

# 9. REC: recording and playback

REC records HackRF talkgroup calls and channel traffic so you can play them back later. It saves the **raw radio signal** of the channel, not the audio, and decodes it again every time you play. A better decoder later gives better playback of old recordings.

- About 128 KB per second of traffic, roughly 460 MB per hour.
- **Encrypted traffic is never recorded.** Encrypted talkgroups and channels can't be marked, and a call found to be encrypted is discarded.
- Nothing is uploaded. See [chapter 18](#ch18).

REC needs a saved station. Recordings sit beside it ([chapter 15](#ch15)).

<!-- screenshot: rec-01 — Creating the recordings database on first use. -->

## 9.1 Marking what to record

**Right-click** a talkgroup row (TG tab) or a channel row (Channels) to mark or unmark it. A marked row shows a red **● REC**. Marks are saved with the station.

## 9.2 The REC key

The TG tabs and Channels have a **REC** key above LOAD and SAVE.

| Press | Lamp | Records |
| --- | --- | --- |
| Click | red | Marked talkgroups or channels only |
| Double-click or Shift+click | amber | All traffic |
| Click again when lit | dark | Off |

The REC state is saved and comes back on the next start. A talkgroup recording starts with the call, continues through re-keys in the voice hang, and has about 250 ms of padding at each end. A talkgroup recording only runs while the voice decoder is decoding voice: about 1 second after the last voice frame (plus the voice hang) the recording closes, even if the control channel still lists the call, and a call that shows no voice within 3 seconds isn't kept open. A known control channel is never recorded as a talkgroup. A channel recording runs while the scanner holds a voice channel. A call still going at the **Max segment** length (Data dialog, default 120 s) is split into a new recording straight away, so nothing is lost; the timeline shows two blocks.

<!-- screenshot: rec-02 — Arming a channel for recording. -->

## 9.3 The REC tab

While REC is open, the timeline controls sit inside the tan bar at the top of the page, at its right end just before the rounded end cap. They are plain rectangular buttons, the same height as the bar. Below the bar, from top to bottom: source tabs (**HackRF TG**, **HackRF Channels**, plus any other source found; if they don't all fit, click the outlined end of the tan bar above them for a list of the rest, as on the Plan page), a status line (REC state, TG, Channels, disk use), the search row, and the timeline.

| Control in the tan bar | What it does |
| --- | --- |
| **◀ DAY**, **TODAY**, **DAY ▶** | Jump by day. Every click counts, so quick clicks step several days. |
| **−**, **+** | Zoom out or in by 1.5× |
| **FIT** | Frame the most recent recordings |
| **● LIVE** / **○ LIVE** | Follow the present (the red NOW line). ● is on, ○ is off. |
| **● DATA** / **○ DATA** | Open or close the Data dialog ([9.6](#s9-6)) |
| **● ▶▶** / **○ ▶▶** | Play next: when a recording finishes playing, the following one plays. ● is on, ○ is off; hover to see ON or OFF. |

**LIVE**, **DATA** and **▶▶** are toggles. Each keeps its thin outline when off; off is a dark fill with ○, on is a full fill with ●.

The same rule holds everywhere in Commander: a button that switches something on or off (for example **NR**, **ATU**, **MOX**, **BSP**, **Receiver**, **Follow**, **Follow rig frequency**, **Limit test on**) has a thin border and starts with ● when on or ○ when off. Buttons that do something once have no border. A toggle the radio does not support stays visible, bordered and greyed out. Checkboxes in forms stay checkboxes.

When the window is too narrow for the full controls, they shorten to symbols: **◀ T ▶ − + FIT ● L ● D ● ▶▶**. If even that would leave too little of the tan bar, the controls move up to the right end of the page-tab row. Hover any control to see its name.

| Control in the search row | What it does |
| --- | --- |
| **Search** | Filter lanes by talkgroup, channel, radio ID or notes |
| **Undo delete** | Shown for 60 s after a delete |

<!-- screenshot: rec-03 — The REC tab, armed and waiting. -->
<!-- screenshot: rec-07 — Live follows the present moment when on. -->
<!-- screenshot: rec-08 — Search narrows the lanes. -->

## 9.4 The timeline

One **lane** per talkgroup or channel and one **block** per recording, like a security-camera viewer.

| Mouse | Action |
| --- | --- |
| Wheel over the timeline | Zoom about the pointer (about 18% per notch) |
| Wheel over the lane names | Scroll the lanes |
| Drag | Pan |
| Click a lane name | Highlight that lane |
| Click a block | Play from that point and open the Record / Playback dialog |

The span runs from 0.5 seconds to 30 days. A recording in progress is a red block that grows.

The lane names sit on a shelf over the timeline's left edge that fades out like a shadow. A long name runs on past the shelf into its lane at full brightness, drawn over the recordings, and fades out just before halfway across. Clicks, drags and the wheel on that faded text go to the timeline. When there are more lanes than fit, a thin scrollbar appears in a narrow strip right of the timeline.

The timeline never shows the future: NOW is its right edge. Zooming out, **FIT**, **TODAY** and **DAY ▶** all stop there, and zooming out while at NOW keeps NOW at the right. While the right edge is on NOW the view follows it (**LIVE** on); pan into the past and it stays put.

The red NOW line has a translucent red tint on its left, just wide enough for the **NOW** label. It is strongest at the line and fades to clear toward the past. Nothing right of NOW is tinted, and there is no tint when NOW is off screen. The lanes, recordings, NOW line and label all show through it. It is only paint: clicks, drags and the wheel work there as on the rest of the timeline.

A faint watermark along the frame's bottom bar reads: "Recording radio traffic may be regulated where you are; that is your responsibility. Encrypted traffic is never recorded. Nothing is uploaded." Hover over it to read it at full contrast.

| Block tag | Meaning |
| --- | --- |
| `EMERG`, `PRI` | Emergency or priority call |
| `zst` | Archived (compressed); still plays |
| `offline` | Moved to a drive that isn't attached |
| `no voice` | Playback decoded no voice |
| `OUT OF BAND` | The channel was outside the HackRF's capture when recorded, so the call isn't in the file |
| `MISSING` | The file is gone; **Rescan** flags these |

<!-- screenshot: rec-04 — A call being recorded live. -->
<!-- screenshot: rec-05 — The recordings timeline, one lane per channel. -->
<!-- screenshot: rec-06 — Zoomed in with the mouse wheel. -->
<!-- screenshot: rec-12 — The channel was outside the HackRF's capture, so the call wasn't recorded. -->
<!-- screenshot: rec-13 — A recording in which no voice was decoded. -->

## 9.5 The Record / Playback dialog

Clicking a block opens this dialog. It follows the selection; close it with ✕ and reopen it by clicking the block again. Clicks and the wheel over it don't reach the timeline.

From the top:

1. **Header**: lane, time range and length.
2. **Playback**: **▶ Play** / **■ Stop**, **★ Star**, **🔒 Protect** (never removed by overwrite-oldest), tags, and one status line. To play the following recording when one finishes, use the **▶▶** toggle in the REC bar. **Output** chooses where replay plays: **System default** (as before), or any audio output on the computer, such as wired speakers, a USB headset or a Bluetooth device. **Refresh** lists the outputs again. The choice is saved with the station. If the chosen output isn't there, replay plays to the system default and the dialog says so in words.
3. **Notes**, then **Export WAV…** (decoded now), **Export raw…** (raw IQ plus its sidecar), **Archive** / **Unarchive** and **Delete** (undo for 60 s).
4. **Metadata**: a label/value grid with a **📋 Copy** button.
5. **Decoder parameters**: the decoder settings saved with the recording, collapsed by default.

Playback re-runs the raw signal through the current demodulator and, for digital modes, through the decoder pipe the recording names. It plays to your default audio output and never touches the radios' own audio. If the decoder can't be found, the status line says `decoder script not found: modems/…` and the call plays without voice decoding.

Recordings made before decoder parameters were saved have none. For those, a P25 replay uses the site settings this session has already learned for the same radio, if any.

<!-- screenshot: rec-09 — The Record / Playback dialog. -->
<!-- screenshot: rec-10 — Decoder settings saved with the recording. -->
<!-- screenshot: rec-11 — Playing a recording back. -->
<!-- screenshot: rec-14 — Shown when the decoder script can't be found. -->
<!-- screenshot: rec-15 — Undo a delete within 60 seconds. -->
<!-- screenshot: rec-21 — Exporting a recording. -->

<a id="s9-6"></a>

## 9.6 The Data dialog

**DATA** in the top bar opens one dialog with three groups.

| Group | Controls |
| --- | --- |
| **Storage** | Disk use, **Cap GB**, **Overwrite oldest unprotected at cap**, **Max segment** (seconds, 10–3600, default 120) |
| **Archive** | **Archive older than** N days, **Rescan**, **Clear cache** |
| **Drive** | Pick a mounted drive, **Copy selected**, **Copy older than**, **Move selected**, **Move older than**, **Re-attach**, **Import** |

A running job shows a progress bar and **Cancel** in the tab and in the dialog.

<!-- screenshot: rec-16 — Storage cap and overwrite policy. -->
<!-- screenshot: rec-17 — Archiving older recordings. -->
<!-- screenshot: rec-18 — Copying recordings to a thumb drive. -->
<!-- screenshot: rec-19 — Importing another Commander's export. -->

## 9.7 Storage cap

The default cap is 20 GB. The disk meter warns in words at 80% and 95%. At the cap, recording **stops with a red warning and nothing is deleted**, unless you tick **Overwrite oldest unprotected at cap**. Archived files count at their compressed size; recordings moved to a drive don't count.

If the disk falls behind, REC drops samples rather than disturb live listening, and says so. A recording cut short by a crash is repaired on the next start.

<!-- screenshot: rec-20 — Recording stops at the cap and nothing is deleted. -->

## 9.8 Archiving

**Archive older than** N days (or **Archive** on one recording) compresses recordings in the background. The original is removed only after the compressed copy has been checked. Archived recordings play directly. **Unarchive** reverses it. **Clear cache** removes the waveform thumbnails, which are rebuilt as needed; no recording is lost.

## 9.9 Thumb drives

With a drive mounted under `/media` or `/run/media`, open **Data** and pick it under **Drive**.

- **Copy** puts recordings and their sidecars in `HamSpot-REC/` on the drive with an `index.json`.
- **Move** does the same and deletes the local copy only after the drive copy is verified.
- Moved recordings stay listed as `offline` and play from the drive when it's attached. **Re-attach** copies them back.
- **Import** reads another Commander's `HamSpot-REC/index.json`. Entries with unsafe paths are skipped.

Wait for the job to finish before removing the drive; it ends with a sync.

<a id="s9-10"></a>

## 9.10 commander-rec

`commander-rec` does the same storage jobs from a terminal or cron, without the window.

```
commander-rec archive   --station PATH [--older-than 7d] [--level 1-19] [--nice] [--dry-run]
commander-rec unarchive --station PATH [--dry-run]
commander-rec list      --station PATH [--source hackrf_tg]
commander-rec verify    --station PATH [--archived-only]
commander-rec stats     --station PATH [--cap-gb 20]
commander-rec export    --station PATH --to DIR [--label NAME] [--move] [--older-than AGE] [--dry-run]
commander-rec render-wav RAW OUT.wav [--decoder SCRIPT] [--param HAMSPOT_K=V]...
```

Ages are written `7d`, `2w`, `12h` or `30m`. Exit codes: 0 OK, 1 error, 2 partial or verify failure, 3 another job holds the lock. `render-wav` decodes one recording to a WAV file, which is handy for testing pipe files ([13.6](#s13-6)).

Example crontab line:

```
15 3 * * * nice -n 19 ionice -c3 commander-rec archive --older-than 7d --station ~/stations/Home.HSCommander
```

## 9.11 Other radios

Hamlib and GQRX radios give only demodulated audio, and REC does not record them yet. Recordings of that kind from elsewhere are labelled "demod audio (rig's own audio, not raw)" and play as they are.

---

<a id="ch10"></a>

# 10. Meters, Levels and Timers

## 10.1 Meters

The Meters page shows the S-meter for the current VFO, then ALC, SWR and power.

- The S-meter reads S0 to S9 in about 6 dB steps, then `S9+` dB.
- SWR reads 1.0 (empty) to 5.0 (full). **High SWR** lights at 3.0 or more.
- `Busy`, `Mic Eq` and `PLL unlock` are drawn but not yet connected to the radio.
- **Detach** opens the meters in their own window, and the lock button stops them being rearranged.

<!-- screenshot: meter-01 — The Meters page. -->

## 10.2 Levels

Sliders for AF gain, RF gain, AGC, RF power, noise reduction, IF shift, filter width (50–16000 Hz), mic gain, VOX delay and VOX gain. A slider works only when the radio lists that level with a range; otherwise its label ends in a dash. Levels are saved in the station file. The sub-receiver AF and RF gain, `AGC (sub)` and `Contour` are not connected yet.

<!-- screenshot: meter-02 — Audio levels. -->

## 10.3 Timers

Station clocks at fixed offsets (no daylight saving time), a 10-minute **ID timer** that warns you to identify, a **countdown** (click +1 min, double-click +5 min, triple-click +10 min), and a **stopwatch** with laps. **Copy time** copies a timestamp.

<!-- screenshot: timers-01 — Station clocks and timers. -->

---

<a id="ch11"></a>

# 11. Config: radios, audio and automation

<!-- screenshot: cfg-01 — The Config page. -->

The page is split into groups. Each group sits inside a thin elbow: a bar across the top, with the group's name printed in it, that turns down the left side of the group. From the top:

| Group | What's in it |
|---|---|
| **STATION** | **Load**, **Save**, the station file name, the band plan, and the channel and talkgroup zone counts ([11.1](#ch11)). |
| **DISPLAY** | **Pulse busy tabs** and **Pulse speed**. |
| **RADIO** | The CAT Control table of radios and its buttons ([11.2](#ch11)). |
| **ROTATOR** | The rotator table with **Add**, **Edit** and **Remove**. |
| **AMPLIFIER** | The amplifier table. |
| **TRUNKING** | **Trunk voice hang** and **Emergency/priority override**. |
| **AUDIO PATCH** | Audio wiring and pipes ([11.6](#s11-6)). |
| **AUTOMATION PATCH** | Gpredict, the Control view, **Follow** and the decoder log. |
| **RF PATCH** | The RF patch drawing ([11.8](#ch11)). |

The groups only separate the page visually. Every setting works as before.

## 11.1 Station file

**Load** and **Save** at the top of Config open and save the station file. The label beside them shows the file name, or `No file`.

- Opening a station replaces the plan, channels, talkgroups and Tune settings. If the radio is already online, Commander then sends the frequency, mode, split, RIT and levels. It never keys the transmitter or puts an amplifier in Operate.
- A file with a field Commander doesn't know is refused, and the window carries on unchanged.
- Key text, VNA calibration, the logbook and recordings are never in the station file; they're beside it.

<!-- screenshot: station-01 — Opening a station file (.HSCommander). -->
<!-- screenshot: station-02 — Commander asks before discarding unsaved station changes. -->

## 11.2 CAT Control

A table of radios: **Name**, **Driver**, **Device**, **Listen** and **Status**. **Click a row** to make it the radio Tune drives; the others keep running. **Add hamlib**, **Add GQRX**, **Add HackRF**, **Add audio tap**, **Edit** and **Remove** manage the list. Closing Commander stops everything it started.

Each radio listens on `127.0.0.1`, on the first free port from 4532. The USB ports line lists `/dev/serial/by-id` links, or `/dev/ttyUSB*` and `/dev/ttyACM*`.

Settings on this page include **Follow** (trunk following, [8.1](#ch8)), **Gpredict** (let Gpredict move this radio), **Trunk voice hang**, **Emergency/priority override**, **Pulse busy tabs** and **Pulse speed**.

<!-- screenshot: cfg-02 — Radios under CAT control. -->

## 11.3 Adding a Hamlib radio

1. Press **Add hamlib** and type a **Name**.
2. Pick a **Model** from the list (Hamlib's `rigctld -l`) or type its number. Model 1 is the dummy radio.
3. Pick a **Device**, choose `none`, or type a path.
4. Set **Serial speed**, **CI-V**, **PTT type** and **PTT device** if your model uses them. **Extra -C** passes Hamlib options through.
5. Press **Save**.

<!-- screenshot: cfg-03 — Adding a Hamlib radio. -->

## 11.4 Adding a GQRX receiver

1. Press **Add GQRX**, type a **Name** and press **Save**.
2. Commander writes a GQRX configuration and starts GQRX with remote control on the Listen port. A new row has no input device, so GQRX opens its I/O chooser. Pick your device there.
3. Status reads `choose I/O device` until GQRX answers. After that, the device you chose is kept with the station.

GQRX is receive-only here: one VFO, no transmit, RIT or split.

<!-- screenshot: cfg-04 — Adding a GQRX receiver. -->

## 11.5 Adding a HackRF or an audio tap

- **Add HackRF** adds a HackRF, read directly by Commander. Leave the device as **only HackRF** if you have one; otherwise pick its serial. **Refresh** rescans. HackRF rows are receive-only and feed the trunk follower and REC.
- **Add audio tap** takes a sound-card input (for example a scanner's discriminator output) and runs a decoder pipe on it. It doesn't tune anything.

<!-- screenshot: cfg-05 — Adding a HackRF. -->

<a id="s11-6"></a>

## 11.6 Audio Patch

Each radio gets two PipeWire devices while Commander runs: an RX sink `hamspot_<id>_rx` that the radio plays into and a TX source `hamspot_<id>_tx` that it transmits from. GQRX is started on them. Hamlib radios still get them so you can wire a speaker and a microphone.

The **Audio Patch** group shows each radio with a playback and a capture drop-down, and a three-column picture: microphones, the radio, speakers. **Drag a wire** between a device and a radio; it's the same as choosing in the drop-down. **Helvum** opens the Helvum patchbay if it's installed. Wiring is saved with the station once a station file is open.

Each row also has an **RX pipe** and a **TX pipe** chooser with a **Pipes…** button ([chapter 13](#ch13)).

<!-- screenshot: cfg-06 — One sound card per radio. -->

## 11.7 Automation: rotators and amplifiers

The **Rotator** and **Amplifier** groups each have a grid with **Add**, **Edit** and **Remove**. Each row runs Hamlib's `rotctld` or `ampctld` on `127.0.0.1` (amplifiers from port 4534) and restarts it if it exits.

- **Rotator**: name, model, device, serial speed, extra `-C`. A Yaesu G-5500 uses GS-232A (601) or GS-232B (603). Control it from Tools → Rotator ([12.7](#s12-7)).
- **Amplifier**: name, model from `ampctld -l`, device, serial speed, extra `-C`. The row shows power state (OFF, ON, STBY, OPER), SWR and forward power. **Standby**, **Operate**, **Off** and **Reset** are sent only when you click them. **Freq MHz** with **Set** sends a frequency once; **Follow rig frequency** sends the active radio's frequency after it has settled for 0.4 s. Settable levels get a 1–10 slider.

Use model 1 (Dummy) to try either screen.

<!-- screenshot: cfg-07 — Configuring an amplifier. -->
<!-- screenshot: cfg-08 — Configuring a rotator. -->

## 11.8 RF patch

The RF patch group at the bottom of Config is a drawing of how coax runs in your station. It's **visual only** and switches nothing.

- Every radio appears automatically; receive-only ones are marked RX ONLY. Add amplifiers, antenna switches, tuners, meters, dummy loads and antennas from the Add row. Drag a box by its title bar; click it to edit.
- Inputs are on the left of a box and outputs on the right, in the transmit sense. Round jacks are coax; small square ones are PTT/ALC.
- Drag from jack to jack to add a cable. Impossible connections are refused with the reason shown below. Right-click a cable, or select it and press Delete, to remove it.
- Below the drawing, each path is listed (for example `TX path: IC-9700 → KPA1500 → ATU → Yagi`) with warnings for open outputs, loops and similar mistakes.
- An amplifier box can be bound to a configured amplifier. It then shows that amplifier's state, gets **Standby** and **Operate** keys (sent only when clicked), and warns when the amplifier's state doesn't suit the patched TX path.

<!-- screenshot: cfg-09 — The RF patch. -->

---

<a id="ch12"></a>

# 12. Tools

The Tools page has seven screens: **NanoVNA**, **RF Calc**, **Calc**, **Smith**, **Hex**, **Logbook** and **Rotator**. One is shown at a time. Tear them off into their own window with the square at the right of the tab row.

## 12.1 NanoVNA

For a **NanoVNA V2** (V2.2) over USB. When the screen opens and the station names a port, Commander connects once. The port list shows USB serial ports and marks a NanoVNA V2.

<!-- screenshot: vna-01 — NanoVNA sweep with markers. -->
<!-- screenshot: vna-03 — Choosing a NanoVNA. -->

**Zone tabs.** **Cal** is first; the rest are your band-plan zones (HF, VHF, UHF…). Click a zone to show its last scan, or measure it if there is none. Double-click to measure again. **Scan** measures the selected zone, **Run** keeps sweeping until **Stop**. A zone being measured is dark red.

**Chart.** Views are Smith (Y adds admittance), Polar, SWR, return loss, R and X, S21 dB, phase, unwrapped phase, group delay and TDR. **Layout·1/2/4** splits the chart into panes, **+Trace** adds up to four traces, each with its own format and pane. Double-click a pane header to fill the chart with it. Click the chart to drop the selected marker (M1–M4); hover a Smith chart to read impedance.

**Markers and search.** Max, Min and Next pk move the active marker; TARGET with ◀ Tgt / Tgt ▶ finds crossings; Track follows the max or min each sweep; Peaks→M places M1–M8 on the highest peaks. **Filter** on an S21 pane shows centre, loss, 3 dB and 6 dB widths and Q (double-click for notch analysis, triple-click turns both off). **SWR BW** on an S11 pane shows the 2:1 bandwidth.

**Setup.** Port, sweep limits, points (11 to 1023), velocity factor, port extension, **Smooth** and **Host avg** (1 = off). TX power and on-device averaging appear only on firmware 1.0.2 or newer with the sweep at or above 140 MHz.

<!-- screenshot: vna-04 — Sweep setup. -->
<!-- screenshot: vna-06 — Limit lines flag a failing sweep. -->

**Keyboard** (when no text box has focus): a letter is the click, Shift+letter the double-click, Ctrl+letter the triple-click. R Run, L Layout, T Trace, S Store, A Auto, K Limits, M Marker, X Max, I status line. Left and right arrows move the active marker one point.

**Export.** CSV, s1p and s2p save the trace as shown (with calibration). PNG saves the chart. **Screen** reads the device's own display.

### Calibration

Commander keeps one calibration set per zone, in `Name.HSVNA` beside the station. The NanoVNA itself doesn't store it.

1. Press **Scan** on an uncalibrated zone, or open the **Cal** tab. A dialog shows which jacks to fit.
2. **Open**: nothing on S11 or S21, then **Measure open**.
3. **Short** on S11, then **Measure short**.
4. **50 Ω load** on S11, then **Measure load**.
5. Optionally a cable from S11 to S21 (**Measure cable** or **Skip cable**) and a load on each jack (**Measure both** or **Skip loads**).

Each piece is fitted once and every zone that needs it is swept before you move on. Then fit your antenna or device and press **Scan**. The **Cal** tab lists what's stored per zone, with a reload button per step and **Rescan all**. **Backup** and **Restore** save and load the sets as JSON; **Clear cal** removes them for the connected device. **Graph** beside a stored standard plots it against the ideal (click Smith, double-click return loss, triple-click phase), and **Err terms** plots the error terms, which helps spot a badly fitted standard.

The calibration badge reads, for example, RESTORED with its date, OTHER ID if the set came from a different serial, or CAL INT when it's interpolated.

<!-- screenshot: vna-05 — Calibrating the VNA. -->

### Status line and lamps

The bar under the chart shows the sweep (start, stop, points, averaging, run mode, calibration) and the device and port. Click it, or a lamp, for full device details; double-click reconnects; triple-click copies the status line.

| Lamp | Green | Amber | Red | Grey |
| --- | --- | --- | --- | --- |
| LINK | connected | | lost (check USB) | not connected |
| ID | serial read and checked | | ID unreliable: power cycle the VNA | no ID |
| CAL | calibration applied | interpolated or correction off | | uncalibrated |
| SWEEP | sweeping | sweep errors | | held |

**ID UNRELIABLE: POWER CYCLE VNA** means the serial number reads didn't agree, usually because the NanoVNA restarted during the read (low battery, or a USB hub that can't power it). Charge it or use a powered port, power cycle it and reconnect.

<!-- screenshot: vna-02 — VNA status lamps. -->

Commander doesn't flash firmware. If ModemManager grabs the port, install `docs/nanovna-v2.rules` by hand and make sure you're in `dialout`.

## 12.2 Smith

A manual Smith chart calculator: type a load and read gamma, SWR, return loss, mismatch loss, normalized impedance, admittance and Zin.

<!-- screenshot: smith-01 — Smith chart. -->

## 12.3 RF Calc

Bench maths: wavelength, antennas, coax loss, SWR, power, Ohm's law, LC, matching, path loss and a US band list.

<!-- screenshot: rfcalc-01 — RF calculators. -->

## 12.4 Calc

A general calculator.

<!-- screenshot: calc-01 — The calculator. -->

## 12.5 Hex

A hex viewer. Press **Open** on the Hex Viewer row to load a file.

<!-- screenshot: hex-01 — Hex viewer. -->

## 12.6 Logbook

A station diary in `Name.HSLog` beside the station (it needs a saved station). **Log** stamps the selected radio's frequency, mode and name on a new entry; Enter saves it. Click a row to select, double-click to edit, triple-click to quote it into a new entry. Delete asks first, and **Undo delete** brings back the last one.

<!-- screenshot: log-01 — The station logbook. -->

<a id="s12-7"></a>

## 12.7 Rotator

Shows each configured rotator ([11.7](#ch11)) with an azimuth indicator and horizon, presets (**PRESET AZ / EL**), manual moves (LEFT/RIGHT, UP/DOWN), speed, **STOP ALL** and rate tracking. Limits come from Hamlib or the model preset, and overlap past 360° is shown.

<!-- screenshot: rot-01 — Rotator control. -->

---

<a id="ch13"></a>

# 13. Pipe files

<a id="s13-1"></a>

## 13.1 What a pipe file is

An RX pipe sits between a radio's audio and the speaker. A TX pipe sits between the microphone and the radio's transmit input. Each one is a small executable file, usually a shell script, in one of Commander's own pipe folders. RX pipe files end in `.rxpipe` and TX pipe files end in `.txpipe`.

A station file names a pipe only as `modems/<name>`, for example `modems/commander-demod-p25.rxpipe`. It never holds an absolute path, a shell command, arguments or anything else Commander would have to pass to a shell. Commander looks the name up in its own folders and runs the file it finds there, with nothing added except the arguments and environment described below. A station file or recording that someone hands you therefore can't make Commander run an arbitrary program. If you need a custom command, put it in a pipe file yourself. Creating that file is the explicit step that allows it to run.

The same rule covers REC decoders ([chapter 9](#ch9)) and the HackRF trunk decoder. Fixed tools that Commander runs itself, such as `pw-cat`, `rigctld`, `hackrf_transfer` and `gqrx`, aren't pipes.

<a id="s13-2"></a>

## 13.2 Where pipe files live

Commander searches these folders in order, and the first file with that name wins:

| Folder | Owner | Pipe editor |
|---|---|---|
| `~/.local/share/hamspot-commander/modems` | installed with Commander | read-only |
| `modems/` next to the `commander` executable (portable folder) | installed with Commander | read-only |
| `~/.local/share/hamspot-commander/modems.d` | you | edit, rename, delete |

Debug builds also search the source tree's `modems/`, plus one folder named by `HAMSPOT_DEV_MODEMS` if that variable is set, so a checkout works before it's installed. Release builds don't search the current directory or the source tree.

A user file can't reuse the name of an installed file. Use **Duplicate to user pipe…** and give the copy a new name.

Before Commander runs a pipe file, it checks:

- the name uses only `a-z 0-9 . _ -`, ends in `.rxpipe` or `.txpipe` and has no folder part;
- the file is a regular file, not a symlink;
- the file is executable by its owner;
- neither the file nor its folder is writable by group or others (fix with `chmod go-w`).

Files written by the Pipe editor are mode `0750`, and so is the `modems.d` folder.

If a check fails, the pipe doesn't start and the radio's Audio row shows the reason, for example `rx pipe disabled: … refused: file or folder is group/world-writable`.

**Older station files.** On load, a pipe field that holds an absolute path is shown as `modems/<file name>` when that file exists in the pipe folders. The station file on disk is updated only the next time you save the station. A field that holds a shell command, or a path to a file outside the pipe folders, is never run. Its radio shows `rx pipe disabled: custom command not allowed; create a pipe file in modems/ — see docs (Pipe files)` (or `tx pipe …`), the same text is written to the log, and the Audio row marks the field "not runnable". Open **Pipes…** on that row to wrap the command ([13.5](#s13-5)).

<!-- screenshot: pipe-05 — Custom commands no longer run directly. -->

<a id="s13-3"></a>

## 13.3 The RX pipe contract

Commander starts an RX pipe as `<pipe file> <port>`, in its own process group, and connects it like this:

- **stdin:** raw 48 kHz, mono, signed 16-bit little-endian PCM (s16le) with no header.
  - Hamlib and GQRX radios: the radio's receive audio, recorded with `pw-cat`.
  - Audio taps: the chosen sound card input.
  - HackRF rows: Commander's FM discriminator. It runs at 16 kHz, and each sample is repeated three times to make 48 kHz. 4 kHz of deviation is 12000 in sample units.
- **stdout:** raw 48 kHz mono s16le, played to the radio's speaker by `pw-cat`. The pipe must keep writing that format and rate. Resample inside the pipe (for example with `sox`) if your decoder works at 8 kHz.
- **`$1`:** a localhost UDP port Commander has reserved for this pipe. Each live pipe gets a different port, starting at 23469. Decoders that send voice over UDP, such as dsd-fme, should use it. A manual run with no argument should default to 23469, as the shipped scripts do.
- **stderr:** discarded, unless `HAMSPOT_CC_EVENTS=1` or `HAMSPOT_IDENTITY=1` is set. Then Commander reads it line by line, as trunk events or as scanner identity text.
- **Environment:** set only when it applies.

| Variable | Meaning |
|---|---|
| `HAMSPOT_CC_EVENTS=1` | The pipe is the trunk control decoder. Keep the decoder's stderr. |
| `HAMSPOT_IDENTITY=1` | A channel vocoder. Keep stderr for the scanner glass. |
| `HAMSPOT_INVERT=1` | The channel is set to inverted polarity. |
| `HAMSPOT_P25_K=<file>` | dsd-fme `-K` key CSV, from the station's key setting. |
| `HAMSPOT_P25_X=<hex>` | P25 Phase 2 WACN, system and NAC word (HackRF voice decoder, and REC replay). |
| other `HAMSPOT_*` | Decoder settings saved with a REC recording (below). |

- **Exit:** when a pipe exits for any reason, Commander stops its whole process group. It shows `rx pipe restarting (…)` and starts the pipe again after 2 seconds. Exit when your input ends or a tool you depend on fails, and don't loop forever inside the pipe.

**REC and decoder settings.** A recording stores the settings the live decoder got (P25 site word and IDs, modulation, HackRF gains) as `decoder_params`. Replay runs the same pipe file on the recorded IQ and feeds it the same 48 kHz discriminator stream as live. It passes each saved `HAMSPOT_*` key back as an environment variable. Keys outside `HAMSPOT_[A-Z0-9_]+`, and values that look like a path, aren't passed. A pipe must therefore be deterministic and work offline. It must not tune the radio, depend on the time of day or the live control channel, or expect a sound card. Replay may also run it faster than real time.

<a id="s13-4"></a>

## 13.4 The TX pipe contract

Commander starts a TX pipe as `<pipe file> <port>`:

- **stdin:** the microphone, as raw 48 kHz mono s16le.
- **stdout:** the radio's transmit audio input, also raw 48 kHz mono s16le.
- **stderr:** discarded.
- **`$1`:** a reserved UDP port. Most TX pipes ignore it.

A TX pipe never keys the radio. PTT is handled by Commander and the radio as before, and the pipe runs whenever the microphone link is up. As with RX, an exit is restarted after 2 seconds.

<a id="s13-5"></a>

## 13.5 The Pipe editor

<!-- screenshot: pipe-01 — Choosing an RX/TX pipe. -->
<!-- screenshot: pipe-02 — Editing a pipe. -->
<!-- screenshot: pipe-03 — Creating a new pipe. -->
<!-- screenshot: pipe-04 — Duplicate a built-in pipe to customise it. -->

Open it with **Pipes…** next to the RX pipe or TX pipe chooser in a radio's Audio row.

- **RX pipes / TX pipes** switch the list. Every file is marked `installed` or `user`. Selecting one shows its text in a monospace editor. Installed files are read-only.
- **New pipe…** starts a user file from a template that documents the contract above. Change the name (for example `my-fm.rxpipe`) and the text, then click **Save…**.
- **Duplicate to user pipe…** (installed files) copies the file under a new name, ready to edit and save.
- **Save…** (user files) saves your edits. If you change the name, it renames the file instead. **Delete…** removes the file. Radios that still name the file stop their pipe and show "not found".
- Every save, rename and delete opens a confirmation that shows the exact file contents or names. Nothing is written until you click **Confirm**. An existing file is replaced only when the confirmation says **Replace**.
- **Use for this radio** sets the radio's field to `modems/<name>` and saves the station. You can also pick the file from the RX pipe or TX pipe chooser, which lists user files too. The station field is always chosen from the list, never typed.

**Wrapping a custom command.** When you open **Pipes…** on a field that holds a refused command, the editor drafts `custom-<program>.rxpipe` (or `.txpipe`). The draft has a `#!/bin/sh` line, a "generated by Hamspot Commander on <date>" note and the old command, unchanged. The confirmation shows the whole file. After **Confirm**, the file is saved in `modems.d` with mode 0750, the radio's field is set to `modems/custom-<program>.rxpipe`, and the station is saved. Commander never creates a wrapper without that confirmation.

<!-- screenshot: pipe-06 — Wrap an old command into a pipe file. -->

<a id="s13-6"></a>

## 13.6 Examples

Every example below passes `sh -n` and `shellcheck`. Save each one with the Pipe editor, or by hand in `~/.local/share/hamspot-commander/modems.d` followed by `chmod 0750`. You can test any RX example without a radio against a copy of a recording:

```
cp <station folder>/<Station>.recordings/<radio>/<day>/<call>.cs16  /tmp/t.cs16
cp <station folder>/<Station>.recordings/<radio>/<day>/<call>.json  /tmp/t.json
commander-rec render-wav /tmp/t.cs16 /tmp/t.wav --decoder ~/.local/share/hamspot-commander/modems.d/my-nbfm.rxpipe
```

`render-wav` prints `disc_samples=… audio_samples=… voice=… decoder_status=…`. A status of `Some(0)` with non-zero audio means the pipe ran cleanly. `--decoder` on the command line is the one place a path is accepted, because you are typing it yourself.

**1. Wrapping an existing command.** This is what the Pipe editor makes from an old station field that held `dsdccx -i - -o -`:

```sh
#!/bin/sh
# custom-dsdccx.rxpipe: generated by Hamspot Commander on 2026-10-27 01:40
# from a station-file pipe command that is no longer allowed to run directly.
# $1 is the UDP port Commander reserved for this pipe (unused by the
# original command). stdin/stdout are 48 kHz mono s16le raw PCM.
# Original command, verbatim:
dsdccx -i - -o -
```

**2. Analog narrow FM voice.** This one works on a HackRF row or a discriminator tap:

```sh
#!/bin/sh
# my-nbfm.rxpipe: analog narrow FM voice from the 48 kHz discriminator.
set -eu
exec sox -q -t raw -r 48000 -e signed -b 16 -c 1 - \
         -t raw -r 48000 -e signed -b 16 -c 1 - \
         highpass 300 lowpass 3000 treble -12 2700 gain -l 6
```

`exec` makes `sox` the pipe process itself, so when Commander stops the pipe, `sox` stops with it.

**3. P25, read from `commander-demod-p25.rxpipe`.** These are the parts of the shipped script that matter. Open it in the Pipe editor to see all of it.

```bash
#!/bin/bash
set -euo pipefail                 # any failed tool ends the script -> Commander restarts it
port="${1:-23469}"                # the reserved UDP port; 23469 for a manual run
for program in dsd-fme socat sox; do   # fail fast, with a clear stderr line, if a tool is missing
    command -v "$program" >/dev/null 2>&1 || { echo "…: ${program} is not on PATH" >&2; exit 1; }
done
# dsd-fme reads the 48 kHz s16le stdin (-i -), decodes P25 (-ft: Phase 1
# control plus Phase 2 voice), and sends 8 kHz stereo voice to UDP $port.
# HAMSPOT_P25_X -> dsd-fme -X (Phase 2 WACN/SYS/NAC word, not a key).
# HAMSPOT_P25_K -> dsd-fme -K <key csv>; empty passes nothing.
# HAMSPOT_CC_EVENTS=1 keeps dsd-fme stderr for the trunk follower.
# socat listens on UDP $port; sox mixes the two slots to mono and
# resamples 8 kHz -> 48 kHz s16le on stdout for the speaker.
# A trap kills every child on exit, so a dead decoder or player ends the
# whole script and Commander restarts the pipe group.
```

**4. Reading settings from the environment.** A REC replay passes the same `HAMSPOT_*` values back. Test with `--param HAMSPOT_MY_GAIN_DB=3`.

```sh
#!/bin/sh
# my-params.rxpipe: shows how a pipe reads HAMSPOT_* settings.
set -eu
gain="${HAMSPOT_MY_GAIN_DB:-0}"
case "$gain" in
    ''|*[!0-9-]*) echo "my-params: HAMSPOT_MY_GAIN_DB must be a number" >&2; exit 1 ;;
esac
if [ "${HAMSPOT_INVERT:-0}" = 1 ]; then flip="vol -1"; else flip=""; fi
# shellcheck disable=SC2086 # $flip is intentionally split into sox words
exec sox -q -t raw -r 48000 -e signed -b 16 -c 1 - \
         -t raw -r 48000 -e signed -b 16 -c 1 - $flip gain "$gain"
```

Check every value you read. A recording from someone else can carry any `HAMSPOT_*` text.

**5. A TX pipe: microphone processing.**

```sh
#!/bin/sh
# my-voice-clip.txpipe: microphone processing before the transmitter.
set -eu
exec sox -q -t raw -r 48000 -e signed -b 16 -c 1 - \
         -t raw -r 48000 -e signed -b 16 -c 1 - \
         highpass 300 lowpass 2700 compand 0.02,0.2 -60,-60,-30,-15,0,-6 -6 gain -l 3
```

Test it without a radio with `head -c 96000 /dev/urandom | ./my-voice-clip.txpipe | wc -c`. It should print 96000.

**6. A GNU Radio flowgraph.** The pipe file starts a Python flowgraph that sits beside it, so nothing outside the pipe folder is named:

```sh
#!/bin/sh
# my-gr-fm.rxpipe: run a GNU Radio Python flowgraph as an RX pipe.
set -eu
here=$(dirname "$0")
command -v python3 >/dev/null 2>&1 || { echo "my-gr-fm: python3 missing" >&2; exit 1; }
exec python3 "$here/my-gr-fm.py" --port "${1:-23469}"
```

```python
#!/usr/bin/env python3
import argparse
from gnuradio import gr, blocks, filter as grfilter
from gnuradio.filter import firdes

class Pipe(gr.top_block):
    def __init__(self):
        gr.top_block.__init__(self, "commander-pipe")
        src = blocks.file_descriptor_source(gr.sizeof_short, 0, False)   # stdin s16le
        to_f = blocks.short_to_float(1, 32768.0)
        lpf = grfilter.fir_filter_fff(1, firdes.low_pass(1.0, 48000, 3000, 500))
        to_s = blocks.float_to_short(1, 32768.0)
        sink = blocks.file_descriptor_sink(gr.sizeof_short, 1)           # stdout s16le
        self.connect(src, to_f, lpf, to_s, sink)

if __name__ == "__main__":
    ap = argparse.ArgumentParser(); ap.add_argument("--port", type=int, default=23469); ap.parse_args()
    tb = Pipe(); tb.start(); tb.wait()
```

Save the `.py` file in `modems.d` next to the pipe file. The Pipe editor lists only `.rxpipe` and `.txpipe` files, so create the `.py` file in a text editor. The pipe-file checks apply to the pipe file. Keep the `.py` file `chmod go-w` as well. Python on this system follows the usual byte order (little-endian), so `sizeof_short` matches s16le.

<a id="s13-7"></a>

## 13.7 Troubleshooting

| What you see | Likely cause | Fix |
|---|---|---|
| `rx pipe disabled: custom command not allowed …` | The station field holds a command or a path outside the pipe folders. | Open **Pipes…** and wrap it ([13.5](#s13-5)). |
| `pipe file not found: modems/…` | The file was renamed or deleted, or is installed on another machine. | Pick another file, or copy the file into `modems.d`. |
| `… refused: file or folder is group/world-writable` | A permissive umask or a copy from a thumb drive. | Run `chmod go-w <file>` and `chmod go-w <folder>`. |
| `… refused: not executable` | The file was copied without its mode. | Run `chmod 0750 <file>`. |
| `… not a regular file (symlinks are refused)` | The file is a link. | Copy the real file in instead. |
| `rx pipe restarting (…)` every 2 s | The pipe exits straight away: a missing tool, a syntax error or a bad variable. | Run it by hand, `./x.rxpipe < /dev/zero > /dev/null`, and read stderr. |
| sox `Broken pipe` | The next stage died, for example a player or decoder that was given the wrong format. | Check that every stage uses raw s16le, 48 kHz, mono. |
| Silence, but the pipe runs | The output isn't 48 kHz, the decoder found no voice, or the gain is too low. | Run `commander-rec render-wav … --decoder` and check `audio_samples` and `audio_peak`. |
| Chipmunk or slow audio | The sample rate is wrong (8 or 16 kHz written as if it were 48 kHz). | Resample to 48000 before stdout. |
| Replay plays, live doesn't (or the other way round) | The pipe depends on something only one side has. | Use only stdin, `$1` and `HAMSPOT_*`. |

---

<a id="ch14"></a>

# 14. How-to recipes

## 14.1 Tune to a frequency and listen

1. Make sure the right radio row is selected on Config and its Status is up.
2. On Tune, type the frequency under the digits and press Enter, or click a band chip.
3. Pick a mode from **Mode**.
4. On Config → Audio, wire the radio's RX card to your speakers.

## 14.2 Scan a list of channels

1. On Channels, **LOAD** a channel file, or tune each frequency and press **Add**.
2. Double-click the zones you want to scan (or open one zone).
3. Press **SCAN**. Press **DWELL** to stay on a busy channel.

## 14.3 Follow a talkgroup

1. Store the system's control channels in one Channels zone and tick **Control** on each.
2. Set the radio's RX pipe to the right trunking pipe, for example `modems/commander-demod-p25.rxpipe`.
3. Tick **Follow** on Config, or press **FOLLOW** on the radio's TG tab.
4. On the TG tab, add or edit talkgroups, and tick **Priority** on any that should interrupt a held call.

## 14.4 Record and replay a call

1. Right-click the talkgroup (TG tab) or channel (Channels) to mark it: **● REC**.
2. Click the **REC** key (red). Double-click for all traffic (amber).
3. Open the **REC** tab. Calls appear as blocks; a red block is still recording.
4. Click a block to play it and open Record / Playback. Add notes, **★ Star** or **🔒 Protect** it, or **Export WAV…**.

## 14.5 Calibrate and sweep an antenna with a NanoVNA

1. Plug in the NanoVNA V2, open Tools → NanoVNA and pick its port in Setup.
2. Click the zone you want (for example HF). Follow the calibration dialog: open, short, load ([12.1](#ch12)).
3. Connect the antenna to S11 and press **Scan**.
4. Choose the SWR or Smith view, and press **SWR BW** for the 2:1 bandwidth, or **Max**/**Min** to place markers.
5. Save the sweep with **CSV** or **s1p**, or **PNG** for a picture.

## 14.6 Back up or move recordings to a thumb drive

1. Plug the drive in and let the desktop mount it.
2. On REC, open **Data** and pick the drive under **Drive**.
3. **Copy older than** N days for a backup, or **Move older than** N days to free space.
4. Wait for the job to finish, then eject the drive.

Moved recordings stay on the timeline as `offline`. Attach the drive to play them, or **Re-attach** to copy them back. From cron, use `commander-rec export --to DIR` ([9.10](#s9-10)).

## 14.7 Write your own pipe file

1. On Config → Audio, press **Pipes…** on the radio's RX pipe.
2. Press **New pipe…**, rename it (for example `my-nbfm.rxpipe`) and edit the text.
3. Press **Save…** and **Confirm**, then **Use for this radio**.
4. Test it offline with `commander-rec render-wav` on a recording.

[Chapter 13](#ch13) has the full rules and worked examples.

---

<a id="ch15"></a>

# 15. Files and folders

## 15.1 Station files

Everything a station produces sits in the same folder as its station file, named after it:

| File | Contents |
| --- | --- |
| `Name.HSCommander` | The station: plan, channels, talkgroups, radios, audio wiring, levels, layout, RF patch. JSON. |
| `Name.HSKeys` | Encryption keys (mode 0600) |
| `Name.HSVNA` | NanoVNA calibration and last scan per zone. A damaged file is kept as `Name.HSVNA.corrupt-<date>`. |
| `Name.HSLog` | Logbook |
| `Name.HSREC.sqlite` | Recording index |
| `Name.recordings/<source>/<YYYY-MM-DD>/` | Recordings (`.cs16` or archived `.zst`) with a `.json` sidecar each |
| `Name.recordings/.cache/` | Waveform thumbnails; safe to delete |

The sidecars are the source of truth for recordings: **Rescan** rebuilds the index from them.

## 15.2 Program files

| Path (installed copy) | Contents |
| --- | --- |
| `~/.local/bin/commander` | The program |
| `~/.local/share/hamspot-commander/modems/` | Shipped pipe files (read-only) |
| `~/.local/share/hamspot-commander/modems.d/` | Your own pipe files |
| `bsp/` beside the program | Band plans |
| `~/.local/share/hamspot-commander/gqrx/<name>.conf` | GQRX settings Commander writes for each GQRX row |

The portable folder keeps all of these inside `HamSpotCommander` ([3.3](#s3-3)).

## 15.3 Thumb drive

| Path on the drive | Contents |
| --- | --- |
| `HamSpot-REC/` | Exported recordings and sidecars |
| `HamSpot-REC/index.json` | What was exported, read by **Import** and **Re-attach** |

---

<a id="ch16"></a>

# 16. Troubleshooting

| What you see | Likely cause | What to do |
| --- | --- | --- |
| Status `device missing` | Cable unplugged or wrong device path | Plug it in or fix **Device**; no restart needed |
| Status `retrying` over and over | Wrong model, speed or CI-V address | Edit the row; check with `rigctld -m <model> -r <device>` |
| Serial port won't open | Not in `dialout` | `sudo usermod -aG dialout "$USER"`, log out and in |
| GQRX stuck on `choose I/O device` | GQRX is waiting for its input | Pick the device in GQRX's window |
| A button is dark | The radio doesn't report that function, or it isn't wired in this version | Expected; see [5.5](#ch5) |
| `MOX` doesn't key | The band plan refused the mode on that segment | Read the note on Tune; change mode or frequency |
| No audio | RX card not wired to a speaker | Wire it in Config → Audio |
| `rx pipe disabled: custom command not allowed …` | The station names a command, not a pipe file | **Pipes…** → wrap it ([13.5](#s13-5)) |
| `rx pipe restarting (…)` every 2 s | The pipe exits at once | See [13.7](#s13-7) |
| Encrypted calls are skipped | No filled key | Expected. Fill a key you're entitled to on Keys |
| Keys, Logbook or REC says to save first | No station file yet | Save the station on Config |
| REC playback says `no voice` | Decoder found no voice; or the call was `OUT OF BAND` | Check the tag; test with `commander-rec render-wav` |
| `decoder script not found: modems/…` | That pipe file isn't on this machine | Install or copy it into `modems.d` |
| `⚠ DISK CAP REACHED` | Recordings hit the cap | Raise **Cap GB**, archive, move to a drive, or tick overwrite |
| `ID UNRELIABLE: POWER CYCLE VNA` | NanoVNA restarted while being read | Charge it or use a powered port, power cycle, reconnect |
| NanoVNA port missing or busy | ModemManager took it, or not in `dialout` | Install `docs/nanovna-v2.rules`, check groups |
| Station file won't load | It has a field Commander doesn't know, or bad JSON | Fix the file; the running station is unchanged |

---

<a id="ch17"></a>

# 17. Glossary

| Term | Meaning |
| --- | --- |
| **Bandspread (BSP)** | The coloured strip of plan zones on Tune that you click to tune |
| **CAT** | Computer control of a radio, here through Hamlib |
| **Control channel** | The trunked-system channel that announces which voice channel each call uses |
| **cs16** | The raw recording format: complex samples, 16-bit |
| **Decoder parameters** | `HAMSPOT_*` settings saved with a recording and passed to its decoder on replay |
| **Dwell** | Hold on the current channel while scanning |
| **Grant** | A control-channel message assigning a call to a voice channel |
| **Hamlib** | The radio-control library; Commander runs its `rigctld`, `rotctld` and `ampctld` |
| **Lane / block** | A row per talkgroup or channel on the REC timeline, and one recording on it |
| **Pipe file** | A small executable (`.rxpipe`, `.txpipe`) that processes audio between radio and speaker or microphone |
| **Priority talkgroup** | One that can interrupt a held call |
| **Sidecar** | The `.json` file next to each recording with its details |
| **Station file** | `Name.HSCommander`, holding one station's setup |
| **Talkgroup (TG)** | A group of radios on a trunked system that talk together |
| **Voice hang** | How long a receiver waits on a voice channel for a re-key |
| **Zone** | A named group of bands, channels or talkgroups |

---

<a id="ch18"></a>

# 18. Recording, the law and your privacy

- Recording or listening to radio traffic may be regulated where you are. That is your responsibility.
- **Encrypted traffic is never recorded.** Encrypted talkgroups and channels can't be marked, and calls found to be encrypted are discarded, even if you hold a key.
- Recordings stay on your computer and the drives you choose. **Nothing is uploaded.**
- Keys are stored only in `Name.HSKeys`, readable by you alone. Use only keys you are entitled to.

---

<a id="ch19"></a>

# 19. Roadmap

Planned work, including a TV tab, a cleaner layout for station data, and more decoders, is tracked on the project's [GitHub issues](https://github.com/hamspot/hamspot-commander/issues). This guide describes only what the program does today.

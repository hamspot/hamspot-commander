# Hamspot Commander

**User Guide**

Version 0.8.5

24 October 2026

Hamspot

This guide tells you how to run Hamspot Commander, tune a radio from the window, save a station file, and keep a portable copy on an Ubuntu live USB. It describes the program that ships in release 0.8.5. It does not describe a later build.

Ham Radio Deluxe is a trademark of its owner. This project does not copy that program's icons, pictures, or manual text. The chapter order of this guide follows the same kind of operator manual: what the program is, how to start it, how the face works, then how to configure it.

The program is licensed under the GNU General Public License, version 3 or any later version. See the `LICENSE` file in the source tree.

---

## Table of contents

The printed pages number the chapters. In this Markdown master the same headings are the contents.

1. Introduction
2. Getting started
3. The window
4. Tuning
5. Band plans
6. Channels
7. Meters and levels
8. The station file
9. CAT control and GQRX
10. Audio
11. A persistent Ubuntu live USB
12. An installed copy
13. Appendix: files and dark controls

---

# 1. Introduction

## 1.1 What Hamspot Commander is

Hamspot Commander is a station face for a radio you can reach with Hamlib, and for a GQRX receiver the program starts itself. You use it to read the frequency, change the VFO, set the mode, key the transmitter when the radio allows it, and keep a band plan and a local channel file.

The window is a dark set of grouped frames: a top bar, a row of page tabs, and the page. The frames reach the bottom of the window. The standing pages are Tune, Plan, Channels, Meters, Timers, Levels, Keys, Tools, and Config. Timers sits immediately right of Meters. Keys sits immediately left of Tools. A radio that can follow a trunked system adds a TG tab after Channels, and an ID tab when that system has named radios. The label is the radio name plus TG or ID, for example `HackRF TG`. The Channels tab and each TG tab always show a speaker on the left of the name. It stays dim until voice is coming in, then it is solid, and it fades over about two seconds of silence. While that voice is up the tab also brightens and darkens. A control channel does not do this. The Config page can turn the pulse off or set it to Slow, Normal, or Fast. Turning animation off in the desktop settings leaves the speaker and holds the tab still.

Radio traffic uses Hamlib's `rigctld` text protocol. Commander does not link the Hamlib library. A GQRX row uses the short rigctl commands on the same kind of client. Commander starts those programs. It does not attach to a `rigctld` or a GQRX that some other program already started, and it stops only the processes it started.

## 1.2 What this release does not do

ADIF import, a contest logger, and a DX cluster are not in this release. The Tools page has a Logbook, which is a time-stamped diary beside the station file. It is not an ADIF log. Commander is not itself a satellite tracker. A radio row can still follow Gpredict when that row's Gpredict box is on. That box is not talkgroup Follow.

Digital voice is an RX pipe that runs `dsd-fme`. It is not a mode in the Mode menu. Rotators have their own rows on the Config page. A HackRF row captures one dongle. An audio tap takes a discriminator or line input and plays a demodulator. It does not tune.

The Channels page is a file on this computer. It is not the memory bank inside the radio. The buttons `A > Mem` and `Mem > A` use one frequency held in the window, not that file and not the radio's memories. Favorite and that one-slot frequency are written in the station file.

Some buttons on the Tune page stay dark on every radio. Section 13.4 lists them. A button that the connected radio cannot perform also stays dark, even when a different radio would light it.

## 1.3 This guide

Each chapter starts with what you are looking at, then the steps. A note is something you can skip and still operate. An important note is something that will lose a setting or key the radio if you miss it.

Button names are printed as they appear on the window, for example `Add hamlib` and `Band +`.

## 1.4 Computer

Release 0.8.5 of the portable build is a 64-bit Linux program. The install script in the source tree accepts only Ubuntu 26.04. The portable folder can be copied onto another Ubuntu machine of the same kind. It still needs the packages in the next section. Those packages are not inside the zip.

You need a graphical session. The window uses the desktop's display. Audio uses the session's PipeWire socket. Do not point the launcher at a different runtime directory. The start script leaves that socket alone on purpose.

A serial radio needs the user in the `dialout` group. A USB device node that Ubuntu assigns to `plugdev` needs that group as well. Membership is on the machine, not inside the portable folder. After a group change, log out and back in before you open the port.

## 1.5 Programs the radio face expects

| Program | Package on Ubuntu | What it is for |
| --- | --- | --- |
| `rigctld` | `libhamlib-utils` | Hamlib radios, including the built-in dummy |
| `gqrx` | `gqrx-sdr` | A GQRX row on the Config page |
| `pw-loopback`, `pw-dump` | `pipewire-bin` | The receive and transmit audio cards |
| `helvum` | `helvum` | The Helvum button on the Audio frame |
| `dsd-fme` | built from lwvmobile/dsd-fme | Digital voice, including P25, on an RX pipe |
| `sox`, `socat` | `sox`, `socat` | Resample dsd-fme's 8 kHz voice to the RX pipe's 48 kHz stream |

PipeWire itself is part of the Ubuntu desktop session. Commander does not start it.

The source-tree installer also installs `rustc`, `cargo`, `pkg-config`, and `libgtk-3-dev` so it can compile the window. It installs `build-essential`, `cmake`, `git`, `libcodec2-dev`, `libncurses-dev`, `libpulse-dev`, `librtlsdr-dev`, `libsndfile1-dev`, and `libusb-1.0-0-dev`, then builds `mbelib` and `dsd-fme` into `~/.local/bin`. It does not install GNU Radio or the Ubuntu `dsdcc` package. The portable zip is already compiled and does not include `dsd-fme`. A live USB does not need the compiler.

## 1.6 Radios

A Hamlib row stores a model number, a device path, a serial speed, a CI-V address, a PTT type, a PTT device, and any extra Hamlib `-C` options. The model list is the list `rigctld -l` prints on that machine. The closed model button shows the model number and the radio name, for example `3073  Icom IC-7300`. The saved value is the number.

An empty device is valid. Model `1` is Hamlib's dummy radio. The first time that dummy answers, the window sets VFO A to 14.153608 MHz USB, VFO B to 146.520 MHz FM, and VFO C to 446.000 MHz FM, then selects VFO A again. A later reconnect of the same dummy does not write those frequencies again.

The dummy receives from 150 kHz through 1.5 GHz. The window will not tune it outside that span.

GQRX is a receiver on this face. Its caps are one VFO, no transmit, no RIT, and no split. The face treats its receive range as 0 Hz through 6 GHz. A new GQRX row has no input device, so GQRX opens its I/O device chooser and Commander leaves that window up.

## 1.7 Where the release file is

The portable zip for this guide is attached to the GitHub release:

https://github.com/hamspot/hamspot-commander/releases/tag/v0.8.5

The file name is `hamspot-commander-0.8.5-portable-linux-x86_64.zip`. Unzipping it produces one folder, `HamSpotCommander`. Chapter 11 is the procedure for a USB stick.

---

# 2. Getting started

## 2.1 Two ways to run it

Use the portable folder when the computer is an Ubuntu live USB, or when you want the station files to travel with the program. Chapter 11 is that path.

Use `scripts/install.sh` from a source checkout when Ubuntu 26.04 is installed on the machine and you want a binary under your home directory. Chapter 12 is that path.

Both end at the same window. They put the station files in different places.

## 2.2 What you see first

The window opens on the Tune page. If the station file has no radio list, Commander starts one Hamlib row named Dummy, model 1, with no serial device.

Press `Demo` on the top bar to leave the live radio and use a small radio inside the process. Demo does not start `rigctld`. Press `Demo` again to return to the selected radio.

The in-process demo receives the ten HF segments from 160 m through 10 m, including 60 m from 5.250 MHz to 5.450 MHz. It will not tune 6 m, 2 m, or 70 cm. It starts with VFO A at 14.153608 MHz USB and VFO B at 7.074 MHz LSB, and it transmits on VFO A until you select `TX sub`.

## 2.3 Full screen and a second window

`Full screen` on the top bar switches the window. Press it again to leave full screen.

The desktop file has an action named New Window. That starts another process. Two processes do not share one radio port. They can share the same saved window folder, so the last one to exit writes the band-plan folder remembered for the file dialog.

## 2.4 Favorite

1. Press `Favorite` on the top bar.
2. Press `Save current` while a radio is online. The window stores the frequency, mode, and passband.
3. Press `Recall` to send them again, if the radio can set the frequency.

Close the Favorite window with its title-bar control. Nothing is written to disk. The next launch starts with "Nothing saved."

## 2.5 If the radio does not answer

Read the Status column on the Config page before you change frequency.

`starting` means the program has been launched and the port is not open yet.

`device missing` means a Hamlib row names a serial device that is not on the machine. Commander waits. Plug the cable in. It does not need a restart if the device appears.

`choose I/O device` means a GQRX row is up and its remote-control port is not accepting yet. Use the GQRX window to pick a device. Commander does not kill that window on a timer.

`retrying` means a Hamlib process exited. Commander starts it again about two seconds later.

A frequency the radio refuses does not move the readout. The typed number is ignored and the digits stay where the radio is.

---

# 3. The window

## 3.1 Top bar

The top bar holds `Demo`, `Favorite`, and `Full screen`.

## 3.2 Pages

The tab row starts as `Tune`, `Plan`, `Channels`, then any TG or ID tabs, then `Meters`, `Timers`, `Levels`, `Keys`, `Tools`, and `Config`. Timers is the station clocks. A TG tab reads like `HackRF TG`. An ID tab reads like `HackRF ID`. Click a tab to open that page. Pull a tab off the row, then drop it on another tab, to move it. A sideways nudge stays a click. The station file stores that order in `tabs` when it is not the built-in order. The wheel over the frequency display is handled by Tune. The band scales sit outside the scroll area so the wheel is not stolen by the list.

## 3.3 Page frame

Every page frame reaches the bottom of the window. There is no status line under the elbows.

## 3.4 Tune, from top to bottom

The Tune page is four bands of controls.

The frequency frame holds the current VFO, a column of VFO buttons, and the paired VFO when the radio has one. Under the digits is a fine scale. The next row is `ALT`, `BSP`, `LOAD`, the band chips for the zone that contains the frequency, then `Mode`, `Sub`, and `Receiver` at the right.

Under that frame is the control row: `ATU`, `Band -`, `Band +`, and the rest of the keys in section 4.8.

`BSP` shows or hides the bandspread. When it is on, one line is drawn for each zone in the loaded plan. A zone the radio cannot receive is left out.

---

# 4. Tuning

## 4.1 The digits

The readout always shows four megahertz digits, then a kilohertz group, then a hertz group. 14.153608 MHz is `0014.153.608`. 146.520 MHz is `0146.520.000`. A frequency at or above 10000 MHz prints in full.

The left readout is the current VFO. Its label is the Hamlib token, written with a space: `VFO A`, `Sub`, `Main A`. The S-meter sits on this readout. Hamlib reports one strength, so the other readout has no second S-meter.

The right readout is the paired VFO: VFO A with VFO B, Main with Sub, and the A and B forms of those names. A VFO with no pair in the radio's list has no right readout. The right digits tune the split frequency.

Click a digit to select that place and add one to it. One notch of the mouse wheel on a digit adds or subtracts one of that digit and does not change which digit is selected. A notch anywhere else on that VFO cluster, including the typed frequency, the mode text, the meter, and the gaps, steps the selected digit. One physical notch is one step. The line under the digits reads `step` and the size, such as `100 Hz` or `1 kHz`.

## 4.2 Typed frequency

Click the text under the digits, type a frequency, and press Enter.

Three groups work: `0014.153.608` or `14.153.608`. Two groups are megahertz, so `14.153608` is the same frequency. A whole number is hertz.

The radio must be able to receive that frequency. A number outside the receive ranges is ignored.

## 4.3 The fine scale

The scale marked Fine sits under the two readouts. A drag on it tunes the current VFO inside the window it opened on. It does not jump to another band.

## 4.4 VFO buttons

These sit between the two readouts. A button lights only when the radio reports that it can do the operation.

| Button | Action |
| --- | --- |
| `A <> B` | Exchange the VFOs (Hamlib `XCHG`) |
| `A > B` | Copy A to B (Hamlib `CPY`) |
| `A > Mem` | Store the current frequency in the window |
| `Mem > A` | Tune back to that one stored frequency |
| `TX main` | Split off. Transmit on the current VFO |
| `TX sub` | Split on. Transmit on VFO B |
| `VFO A`, `VFO B` | Select that VFO |
| `R-6kHz`, `R-3kHz`, `R-15kHz` | Set the RIT offset |
| `R-Auto` | Clear RIT |

`ALT` on the band row sends the same exchange as `A <> B` when the radio lists `XCHG`.

RIT is an offset. It is not a second VFO frequency. The current readout shows `RIT` and the offset in hertz when it is not zero.

VFO select on this face is A and B. A dummy radio can hold 70 cm on VFO C from the first park, and this row does not offer a VFO C button.

## 4.5 Mode, sub mode, and the receiver

`Mode` lists the modes the radio reported. Choosing one sends that mode and keeps the current passband.

`Sub` does the same for the paired VFO, when the radio can set split and it listed modes.

`Receiver` is `RX` or `Off`. It is enabled when the radio can be powered from CAT. Off is not a substitute for disconnecting the cable.

## 4.6 Band chips and Band + / Band -

The chips are the bands in the current zone that intersect the radio's receive ranges. A radio that stops at 30 MHz does not show a chip for a 500 MHz allocation. Click a chip to tune the remembered frequency for that band, or its home frequency if you have not visited it in this session.

`Band +` and `Band -` step to the next band the radio can receive, in frequency order, across every zone. There is no Hamlib band-up command. The window sends a frequency set. A band outside the receive ranges is skipped.

On the dummy, receive starts at 150 kHz, so 2200 m is not a step and 630 m is.

The chips follow the radio frequency. The Plan page does not follow, so its zone tab stays where you left it.

## 4.7 The bandspread

Turn `BSP` on. Each zone in the plan is one horizontal line, highest zone first. Low frequency is on the left. Sub-bands share the line with a small pad between them. A press in the pad or on the zone name does nothing.

Colors on a ham plan are the segment use: CW is amber, digital is blue, voice is green, and mixed is gray. On the shortwave example the line is painted by service name.

Left-click or left-drag a segment to tune the main VFO and set the main mode from that segment. CW segments ask for CW. Digital segments ask for RTTY, or another mode in that family if the radio has no RTTY. Voice and mixed segments ask for USB, unless the band's stored mode is already legal on that segment. A CW band mode does not stick when you click a voice segment.

Right-click or right-drag sets the split frequency and the split mode. It does not move the main VFO. Right-click is ignored when the radio cannot set split, or when the frequency is outside the receive ranges.

`LOAD` on this row opens a JSON file dialog titled "Load band plan". It does not save. Saving a plan is on the Plan page.

## 4.8 The control row

A control is dark when the radio is offline, when Hamlib did not list that function, or when the button has no command in this release.

| Button | Sent when the radio lists it |
| --- | --- |
| `ATU` | Tuner |
| `Band -`, `Band +` | A frequency on the next receive band |
| `NR` | Noise reduction on or off |
| `Pre 1`, `Pre 2` | Preamp at 10 dB or 20 dB |
| `Ant 1`, `Ant 2` | Antenna 1 or 2 |
| `Bk-in` | Semi break-in |
| `Fast` | Fast AGC |
| `Proc` | Speech processor |
| `Lock` | Dial lock |
| `Notch` | Automatic notch |
| `MOX` | Push-to-talk |
| `Moni` | Monitor |

These stay dark on every radio in this release: `Dimmer`, `NR sub`, `CW Spot`, `Keyer`, `Nar`, `Ant Rx`, `Chan +`, `Chan -`, `QMR`, `BC`, `PWR`, `QMS`, `Contour`, `Scan`, and `Scan Dn`.

## 4.9 Transmit and the band plan

> Important. `MOX` can key the transmitter. The band-plan check below runs first, and a refusal does not key. A mode the plan allows is sent to the radio.

`MOX` checks the loaded plan before it keys. CW is allowed on every segment use. Voice is refused on a CW segment and on a digital segment. Digital is refused on a CW segment and on a voice segment. Mixed refuses nothing. A mode the plan does not classify is not blocked.

The check uses the frequency and mode on the current VFO. Receive tuning is still the radio's receive ranges. The plan does not stop you from listening.

The refusal is a note on the page. The transmitter is not keyed.

## 4.10 Where tuning stops

The window rejects a negative frequency, and it rejects a frequency that is not a finite number. Tuning stays inside the receive ranges Hamlib printed under `RX ranges #1`. Transmit ranges are not used as the tune limit. An empty receive list closes tuning.

A step that would leave the current receive segment stops at the edge. It does not jump the gap.

---

# 5. Band plans

## 5.1 The file you start with

Startup loads `bsp/default.json` from beside the program. In the portable tree that directory is `HamSpotCommander/bsp`. The file is a version-1 JSON plan: zones, and bands inside the zones. Each band has edges, a home frequency, a home mode, and segments tagged `cw`, `digital`, `voice`, or `mixed`.

`bsp/arrl.json` is an example of the ARRL voluntary plan, one zone per named allocation. It is not the file the window loads at startup. `bsp/shortwave.json` is an example listening map (broadcast, aero, marine, time, and the rest). It is not the startup plan, and it does not apply the amateur mode limits from the ham plan.

License class and calling-frequency spots are not segments.

## 5.2 Plan page

The zone row starts with `LOAD`, `Save`, and `ADD`, then the zone names.

`LOAD` opens "Load band plan". `Save` opens "Save band plan". Both dialogs filter for JSON. The dialog starts in the `bsp` folder beside the program unless you have already picked a folder. That folder is remembered with the window.

An empty plan has no standalone Save. `ADD` appends a zone named New, or New 2, and so on, with one band around the current VFO. If the VFO cannot seed a band, the new band is 14.200 MHz USB. The chip text is `new`.

Right-click a zone name to rename it or to delete that zone and its bands. A blank name is ignored. Delete selects the previous zone. Nothing is written to disk until you Save.

The list under the dial is one row per band: slot, home in megahertz, and name. Click a row to select that band and tune its home, inside the receive ranges.

The editor on the right has Name, Home, Low, High, and Mode. It does not edit the individual segments inside the band. A click on the dial inside a band sets that band's home. A click in a gap does not widen the band.

The Plan page lists every band in the file. The Tune page hides bands the radio cannot receive.

## 5.3 A bad file

A plan that does not parse is refused. The plan already in memory stays, and the page shows the error. Saving writes pretty JSON and refuses an invalid plan.

Overlapping bands are an error. Bands that only touch at an edge are valid. The edge belongs to the band that starts there.

---

# 6. Channels

## 6.1 A memory file, not the radio

The Channels page stores named frequencies on disk. A file is version 1: zones of channels. Each channel has a name, a frequency in hertz, and a mode, and may be marked control or encrypted. A vocoder name selects a digital demodulator for that memory. The page starts with one empty zone named Memories.

This does not read or write the memories in the radio.

## 6.2 Load, save, add, recall

The buttons are `Load` and `Save`. The dialogs are titled "Load channels" and "Save channels". A new save suggests `channels.json` when no file is open yet.

The list shows slot, megahertz, and name. Click a row to recall it. Recall tunes only inside the receive ranges, and it sends the mode when the radio can set mode.

The editor has Name, Frequency, Mode, Vocoder, Control, Encrypted, `Add`, and `Delete`. Invert is shown for DMR, NXDN, and dPMR. `Add` uses the current VFO frequency and mode. `Delete` removes the selected channel.

A file with unknown fields, a missing channel list, a band-plan document, or a frequency that is not a finite number above zero is refused. The previous memories stay.

## 6.3 The strip above the list

The strip is the loaded band plan, drawn across the span of the channels in the selected zone, padded on each side. It is decoration and a place to look. Loading a channel file does not change the band plan. If the plan has no segment in that window, the built-in ham plan is used for the colors. A window outside both stays unmarked.

Channel zones and band-plan zones are different lists. They are not matched by name.

## 6.4 Control, encryption, and scan

`Control` marks a trunked control channel. The follower uses the control channels in the zone you have open, and it skips one that is also marked encrypted.

`Encrypted` marks a memory the listing calls encrypted. With no filled key on the Keys page, scan skips it and a click does not tune it. The editor still opens. A filled key lets an encrypted grant play. The click still does not tune that memory.

`SCAN` and `DWELL` themselves are not stored. A double-click marks a zone for the limited scan, and those marks are stored. An empty mark list scans the zone you have open.

### Holding a followed call

Once a receiver is on a voice channel, it holds that call. Grants for other talkgroups, housekeeping and other control traffic do not move the radio or change the readout. The readout keeps the talkgroup, source radio ID and alias of the call being decoded. Control-channel lines are still written to the decoder log and the radio ID list. A receiver set to the Control role keeps watching the control channel the whole time.

A terminator, a sync loss or a dead carrier starts the **voice hang**. The readout shows `· HANG` and the audio stays routed. Voice on that channel inside the hang picks the same call back up, which covers butt-keys and push-to-talk gaps. When the hang runs out, the radio goes back to the control channel it left. A channel that syncs but decodes no voice for 6 seconds is let go as well. Encrypted voice with no filled key still leaves at once.

On the Config page, `Trunk voice hang` sets the hang to 500 ms (the default), 1500 ms or 3000 ms. The value is stored as `voice_hang_ms` and is written only when it is not 500. A station file with another value up to 15000 still loads, and its value is shown as an extra choice.

`Emergency/priority override` is on by default (`override_hold_emergency_priority`). While it is on, only an emergency grant (the emergency bit in the grant's service options) or a talkgroup ticked `Priority` on its talkgroup page can take a receiver off a held call. The switch is made in one step, and the decoder log gets a `hamspot: emergency override` or `hamspot: priority override` line. Turn the box off to hold every call until its hang ends.

An emergency overrides even on a talkgroup you are not monitoring, as long as the override box is on. A priority talkgroup is one you have ticked, so it is already one you chose.

The HackRF wideband follow uses the same hold, hang, silence limit and override rules. A voice channel inside the capture window is decoded while every control decoder in the window keeps running, so control traffic is still logged during the call. A voice channel outside the window takes the whole capture for the call, as before, and the hold keeps that capture until the hang or the silence limit ends it.

---

## 6.5 Recording (REC)

REC records HackRF traffic for later playback. It saves the **raw radio data**, never the demodulator's audio, so a better decoder later gives better playback of old recordings.

**What is saved.** For a HackRF talkgroup call or a held channel, Commander saves the channel itself: complex IQ after DC removal, shifted to the call's frequency and averaged down to **32 ksps, 16-bit (cs16)**. That is half of the 500-sample block the live voice path uses, so replay rebuilds the exact 16 kHz stream the live discriminator sees. It costs about 128 KB/s (about 460 MB an hour of traffic). The full 8 Msps capture (about 58 GB an hour) is not saved. Each recording gets a JSON sidecar with the frequency, capture centre, offset, rate, chain, talkgroup, radio, decoder settings and Commander version.

**Encrypted traffic is never recorded.** An encrypted talkgroup or channel cannot be marked, and a call whose voice decoder reports a non-clear algorithm is discarded, even with a filled key.

On the REC page, a faint watermark along the frame's bottom bar reads: "Recording radio traffic may be regulated where you are; that is your responsibility. Encrypted traffic is never recorded. Nothing is uploaded." Hover over it to read it at full contrast.

The timeline's right edge carries a dark red shelf, the mirror of the lane-name shelf on the left: opaque at the edge and fading out to the left, just wide enough for the red **NOW** label, which stays readable on top. It is only paint; clicks, drags and the wheel work there as on the rest of the timeline.

### Marking

Right-click a talkgroup row (Talkgroups tab) or a channel row (Channels tab) to mark or unmark it. A marked row shows a red **● REC** at its right end. Marks are saved with the talkgroups and channels in the station file (`"record": true`), so they travel with Load and Save. Left click and double click are unchanged.

### The REC key

Each Talkgroups tab and the Channels tab has a REC key above LOAD and SAVE. It follows the rustcars convention:

| Press | Lamp | Records |
| --- | --- | --- |
| Click | red | marked talkgroups / channels only |
| Shift+click, or double-click | amber | all talkgroup / channel traffic |
| Click while lit (Shift+click while amber) | dark | off |
| Ctrl+click | — | reserved, does nothing |

The REC key state is saved in the station file and restored on launch; the REC tab icon comes up dark red and a log line says recording resumed. Talkgroup recordings follow the trunk follower: a segment starts on the call, keeps going through re-keys inside the voice hang, and ends after it, with about 250 ms of padding at each end. Channel recordings run while the scanner holds an open (non-control) channel on the selected HackRF radio. Long calls are split every 5 minutes.

### The REC tab

The tab has a record icon on the left of its name: **black** when no REC key is lit, **dark red** when armed, **bright red** while raw IQ is being written. Hover the icon for the same words. Across the top are source tabs, **HackRF TG** and **HackRF Channels** (always shown, even when empty), plus any other source found in the database.

Under them: one status line (REC state, TG, Channels, disk meter), the timeline toolbar, then the timeline, which takes all the remaining height down to a one-line hint and disclaimer. With few lanes the rows grow (up to twice their normal height) so the area is filled; with many, a taller window shows more lanes and extra lanes scroll with the wheel over the lane names. Messages (playback status, "No voice decoded", errors) are not shown in the tab; they appear once in the Record / Playback dialog (or the Data dialog for storage jobs). The timeline toolbar starts with **Search** (TG, channel, radio id, notes), followed by the day, zoom, Fit, Live, Play next, a small **Data** button and a small **Details** button that reopens the Record / Playback dialog. After a delete, an Undo button sits in the toolbar for 60 s. **Data** opens one *Data* dialog, shared by both source tabs, with three groups: **Storage** (disk use, Cap GB, overwrite oldest), **Archive** (Archive older than N d, Rescan, Clear cache) and **Drive** (thumb-drive copy, move, re-attach, import). Close it with its ✕ or the Data button; it starts closed each session. Job progress and Cancel show both in the tab and in the dialog.

**Timeline.** One lane per talkgroup or channel, one block per recording, like a security camera viewer. The timeline runs the full width; the lane names sit on a shelf over its left side that fades out like a shadow. The wheel over the lane names scrolls the lanes (a scrollbar appears when they overflow); click a name to highlight its lane. The wheel over the timeline zooms about the pointer, keeping the time under it fixed: each mouse notch changes the span by about 18 %, a touchpad zooms proportionally finer, and one frame never changes it by more than 1.5×. The − / + buttons step 1.5×. The span runs from 0.5 s to 30 days. The faded part of the shelf passes clicks and the wheel through to the timeline. Drag to pan, **Fit** frames the most recent recordings, ◀ Day / Today / Day ▶ to jump, **● Live / Now** to follow the present (the red NOW line; the button shows ● when following, ○ when not). A recording in progress is a red block that grows. EMERG and PRI calls are tagged and outlined; archived blocks say `zst`, offline ones are grey and say `offline`. Waveform thumbnails are drawn once they are generated in the background.

**When a replay is silent.** A digital replay that decodes no voice says so on the status line ("No voice decoded: …", with the decoder's last message), and the block is tagged `no voice`. A real failure, such as a missing audio output or decoder, reads "Playback failed: …". Blocks tagged `OUT OF BAND` were recorded while the channel was more than 3 MHz from the HackRF's centre, outside its usable 6 MHz. The call isn't in those samples, so no decoder can recover it. New recordings wait for the follower's retune instead. To check a file offline: `commander-rec render-wav FILE.cs16 out.wav --decoder ~/.local/share/hamspot-commander/modems/commander-demod-p25.rxpipe`.

**Record / Playback dialog.** Clicking a block selects it, plays from that point, and opens the *Record / Playback* window (one at a time; it follows the selection and keeps its place for the session; close it with ✕, reopen with **Details**). It has, from the top: a header (lane, time range, duration); Playback (Play/Stop, Star, Protect, Play next, the OUT OF BAND / no voice / EMERGENCY tags and a single status line); Notes and the Export WAV…, Export raw…, Archive/Unarchive and Delete buttons with a one-line disclaimer; then the Metadata label/value grid (only fields the recording has) with a **Copy** button, and a collapsible Decoder parameters list (closed by default). Metadata scrolls on its own, so the controls at the top stay in view in a short window. Clicks and the mouse wheel over the dialog (or the Data dialog) stay in the dialog and never reach the timeline below.

**Decoder parameters.** Each recording saves what the live decoder got from its environment in `decoder_params`, in the JSON sidecar, the database, and any export's `index.json`, so the parameters go with the file to a thumb drive. For P25 that includes `HAMSPOT_P25_X`, the WACN, system ID and NAC word that Phase 2 voice needs. It also saves the WACN, SYS, NAC, RFSS and site from the control channel, the modulation, and the HackRF LNA and VGA gains. Replay passes every saved parameter to the decoder script as an environment variable. Only keys matching `HAMSPOT_[A-Z0-9_]+` are passed, so a recording can't set `PATH`, `LD_PRELOAD` or similar. A new decoder, such as a GNU Radio flowgraph, can save any `HAMSPOT_*` key it reads. The Record / Playback dialog lists them one per row under Decoder parameters. Recordings made before this have none. For those, a P25 replay uses the site word this session has already learned for the same radio, if there is one. The station file holds no system word, so there's nothing else to fall back on. Override or add parameters offline with `commander-rec render-wav FILE.cs16 out.wav --decoder SCRIPT --param HAMSPOT_P25_X=BEE006BD6B5` (repeatable). **Decoder script.** A recording names its decoder only as a portable name such as `modems/commander-demod-p25.rxpipe`, never an absolute path. At replay Commander looks the name up in its own modems folders (`~/.local/share/hamspot-commander/modems`, then the install and source folders) and refuses anything absolute, containing `..`, not a plain `.rxpipe` file, or a symlink that leads outside the folder; the status line then says `decoder script not found: modems/…` and the call plays without voice decoding. Older recordings that stored a full path are shown and used by their `modems/<file>` name. A custom shell command in the radio's RX pipe is not saved as something to run. Saved parameters that name a local file (`HAMSPOT_P25_K`, `HAMSPOT_DRM_STDOUT`) or whose value is a path are not taken from a recording; `--param` on the command line still can. Imports from a thumb drive skip any `index.json` entry whose path is absolute, contains `..`, or goes through a symlink. The decoder's UDP voice port is reserved for the whole replay and released when it ends or stops, so a live decoder never takes it.

**Playback.** Click a block to play it from the point you clicked; ▶ Play / ■ Stop in the detail row; **Play next** continues with the next recording. Every playback re-runs the raw IQ through the current discriminator (and, for digital modes, through the station's voice script) and plays to the system default audio output. Playback never touches the radios' receive sinks.

**Detail row.** Star, Protect (never removed by overwrite-oldest), Notes, Export WAV (demodulated now; REC does not keep it), Export raw (IQ plus sidecar), Archive / Unarchive, and Delete (raw file, row and cache; **Undo** for 60 seconds).

### Storage, cap and retention

Files live beside the station: `Name.HSREC.sqlite` (the index) and `Name.recordings/<source>/<YYYY-MM-DD>/<HHMMSS>_<lane>_<MHz>.cs16` with a `.json` sidecar. The sidecars are the source of truth: **Rescan** rebuilds missing rows from them and flags rows whose file is gone.

The cap is adjustable (default **20 GB**). Retention is indefinite. The meter warns in words at 80% and 95%. At the cap, recording **stops with a red warning and nothing is deleted**, unless you tick *Overwrite oldest unprotected at cap*. Archived files count at their compressed size; recordings moved to a drive do not count.

The writer runs at idle CPU and I/O priority, writes 128 KiB sequential chunks without per-chunk fsync, and drops (and counts) samples rather than stall live decoding if the disk falls behind. A recording that was cut off by a crash is repaired from its file on the next start.

Waveforms are a disposable cache in `Name.recordings/.cache/`, keyed by recording and decoder version. **Clear cache** loses nothing.

### Archive

There is no live compression. **Archive older than N d** (or Archive on one recording) compresses with zstd level 9 in the background: compress to a temp file, decompress and compare sha256, rename, then remove the original. Any failure keeps the original. Archived recordings play directly. Unarchive reverses it.

### Thumb drives

With a drive mounted under `/media` or `/run/media`, open **Data**, pick it under **Drive**, and Copy or Move the selected recording or those older than N days. Files and sidecars go to `HamSpot-REC/` on the drive with an `index.json`. A **move deletes the local copy only after the copy on the drive verifies by sha256**. Moved recordings stay listed as offline and play from the drive when it is attached; **Re-attach** copies them back (verified), and **Import** reads another Commander's `HamSpot-REC/index.json`. The export ends with a sync; then you may remove the drive.

### commander-rec (cron)

`commander-rec` is a small command-line tool built with Commander (`target/release/commander-rec`). It shares the REC library and needs no GUI.

```
commander-rec archive   --station PATH [--older-than 7d] [--level 1-19] [--nice] [--dry-run]
commander-rec unarchive --station PATH [--dry-run]
commander-rec list      --station PATH [--source hackrf_tg]
commander-rec verify    --station PATH [--archived-only]
commander-rec stats     --station PATH [--cap-gb 20]
commander-rec export    --station PATH --to DIR [--label NAME] [--move] [--older-than AGE] [--dry-run]
```

Ages are `7d`, `2w`, `12h`, `30m`. Exit codes: 0 ok, 1 error, 2 partial or verify failure, 3 another run holds the lock (`Name.HSREC.sqlite.lock`; the REC tab's archive and export take the same lock). Example crontab line:

```
15 3 * * * nice -n 19 ionice -c3 commander-rec archive --older-than 7d --station ~/stations/SAFE-T.HSCommander
```

### Other radios

Hamlib and GQRX radios only give demodulated audio. The database and player already handle such recordings (`source_kind = demod_audio`, labelled "DEMOD AUDIO (not raw)" and played as-is), but recording them is not wired to the REC keys yet.

### REC test plan

Automated: channeliser (tone at offset lands at DC), live tap vs replay (synthetic FM through the live voice mixer, replayed through the shared discriminator), segment split/merge and REC key states, writer pre/post roll, re-key merge, encrypted discard, cap stop and overwrite-oldest skipping protected, crash recovery, DB migrations, range/lane queries, rescan, cache keying/pruning, archive round trip/cancel/corruption, export copy/move/re-attach/import, timeline layout, and `commander-rec` subcommands, dry run and lock contention.

On the air (by hand): a P25 follow with REC red and a marked talkgroup; amber with all traffic; a scanner hold on a marked channel; an encrypted call (nothing kept); HackRF retune mid-call; restart with REC armed; playback after a decoder upgrade; a full cap; a thumb-drive move and re-attach.


# 7. Meters and levels

## 7.1 Meters

The Meters page shows the S-meter for the current VFO, then ALC, SWR, and PWR.

S-meter numbers use Hamlib's strength in dB relative to S9. S9 is about 0 dB. Above that the label is `S9` plus the dB. Below that the label steps by about 6 dB per S-unit down to S0.

SWR is the number Hamlib reports. The bar treats 1.0 as empty and 5.0 as full. The alarm `High SWR` lights when that number is 3.0 or higher.

PWR uses `RFPOWER_METER` when the radio has it, otherwise `RFPOWER`. The numeric label is the value times 100, rounded.

ALC, SWR, and PWR read zero or 1.0 when the radio has not reported that level.

`Roofing` is a label when the radio reports a roofing value.

`Busy`, `Mic Eq`, and `PLL unlock` are drawn and stay off. They do not follow the radio in this release.

Meters are not stored in the station file.

## 7.2 Levels

The Levels page is also the set of sliders you are configuring. A slider is live only when the radio lists that level and gives it a range. Otherwise the label ends in a dash and the slider does not move.

| Control | Hamlib level | Notes |
| --- | --- | --- |
| AF gain (main) | `AF` | |
| RF gain (main) | `RF` | |
| AGC (main) | `AGC` | A drop-down of the names the radio listed |
| RF power | `RFPOWER` | |
| AF gain (sub) | none | Fixed. It never sends a command |
| RF gain (sub) | none | Fixed. It never sends a command |
| AGC (sub): Auto Fast | none | Dark |
| Noise reduction | `NR` | The level, not the NR button |
| IF shift | `IF` | Shown in hertz |
| Filter width | passband | 50 Hz through 16000 Hz, sent with the mode |
| Contour: 20 Hz | none | Dark |
| Mic gain | `MICGAIN` | |
| VOX delay | `VOXDELAY` | Milliseconds when the range allows it |
| VOX gain | `VOXGAIN` | |

Levels that the radio lists are written into the station file. PTT is not.

---

# 8. The station file

## 8.1 One JSON document

The Config page `Load` and `Save` buttons store one version-1 JSON file. The dialog remembers the folder. The label next to the buttons is `No file` until you save or load.

The file holds the band plan, the plan's file name, the channel file, the talkgroup directory, the Tune face (frequency, mode, split, RIT, Favorite, the one-slot memory, and the VFO selection the face can store), the settable levels, the radio list, the audio graph, and the rotator list. Unknown fields fail the load and leave the running face unchanged. Key text is not in this file. Section 8.4 is the key file beside it.

`Save` does not write the `bsp` directory by itself except as described in the next section, and it does not send CAT until you open a file onto a radio that is already online.

## 8.2 Opening a file

Opening a config replaces the plan, the channels, and the Tune fields in memory. It does not reconnect. If the radio is already online, the window then sends frequency, mode, passband, split, RIT, and the listed levels. A frequency the radio refuses does not block the rest. PTT is not keyed.

The plan name in the file must be a single `.json` file name, with no slash and no parent directory. If `bsp/<that name>` is missing beside the program, the embedded plan is written there. An existing file is left alone, even when its bytes differ. If the folder is missing, the face still loads and a note says the plan was not written.

The channel path is cleared on open, so a later Channels `Save` does not overwrite the channel file you had open before.

## 8.3 What you must save yourself

Live meter needles and PTT are not in the file. `SCAN` and `DWELL` being on or off is not stored. The marked scan zones are.

PipeWire changes are written only when a station file is already open. The Audio frame says "Open a station file to keep PipeWire changes." when none is open. Commander does not restore a wire that another program moved. The live graph is the source of truth, and the open file is updated to match it.

## 8.4 Keys

The Keys tab is immediately left of Tools. Until the station has been saved, the page says so and does not write a key file. The line under the buttons names the key file. A station `City.HSCommander` keeps its keys in `City.HSKeys`, in the same folder. The mode of that file is 0600.

The list columns are Key ID, Algorithm, Status, and Name. A missing key uses the alert color under Status. The key text is only in the editor on the right.

`Add` appends the next free id, from 0001 upward, as DES with blank text. `Remove` deletes the selected row. `Import` reads a dsd-fme hex CSV. The first line is the header and is skipped. `Export` writes that same CSV, mode 0600. Names are not in the CSV. `Generate` replaces the selected key text with random hex for the algorithm in the editor and saves the key file. An empty list starts as one DES key. Generate is for a radio you program yourself. It is not a fill from a system key loader. `Save` writes the editor into the key file. It does not save the station.

A key id the decoder prints, when that id is not already listed, is added with blank text. The row stays until you remove it.

With no filled key, an encrypted talkgroup or channel is not scanned or played. DES, RC4, and AES-256 are the algorithms this page can fill. A filled row of one of those lets an encrypted grant play. The air still picks the algorithm. Hytera, TYT, and DMR privacy flags are not sent.

## 8.5 What stays out of the station JSON

Do not paste key hex into the station file. Do not add a field the program does not already know. A later save from the window rewrites the station and will not invent those fields back. The key file is the only place for key text.

## 8.6 Tools

The Tools tab sits between Keys and Config. On that tab, the right end of the tab row is a square the same height as the other tabs. The mark is a box with an arrow pointing out of it. That square opens the tools in their own window titled Tools. Hovering it says Tear off. Closing that window puts the tools back on the tab. While the window is open, clicking the Tools tab brings that window forward, and the main window stays on the page it was already showing. The screens are NanoVNA, RF Calc, Calc, Smith, Hex, and Logbook. One screen is open at a time. Timers is the page after Meters, not one of these screens.

Logbook is the shack diary. Until the station has been saved, the page says so and does not write a file. A station `City.HSCommander` keeps the log in `City.HSLog`, in the same folder. `Log` stamps the selected radio's frequency, mode, and name onto the next entry and focuses the text. Enter still saves. A row click selects it, a double-click edits it, and a triple-click quotes it into a new entry. Delete asks first. `Undo delete` puts the last deleted entry back.

Calc, RF Calc, Hex, and Smith are the rustcars example screens. Hex opens on a sample font until you press `Open`. That button is on the Hex Viewer row, to the right of the title. RF Calc is bench math for wavelength, antennas, coax, SWR, power, Ohm's law, LC, matching, path loss, and a US band list. It does not key the radio. The Timers page is the station clocks, with fixed offsets and no daylight saving time.

Smith stays a manual calculator. It calculates gamma, SWR, return loss, mismatch loss, normalized impedance, admittance, and Zin from the load you type.

NanoVNA talks to a NanoVNA V2 (the S-A-A-2, USB id `04b4:0008`) over its binary protocol. Other NanoVNA families are not on this screen. The chart fills the NanoVNA screen inside the same elbow frames as the other tools. Sweep controls and the selected marker's gamma, SWR, return loss, mismatch loss, normalized impedance, and admittance are in the left frame. The chart is in the right frame. When that screen is open and the station already names a port, Commander connects to that port once. Close in Setup holds that off until you leave the NanoVNA screen and open it again. The port list is USB serial only. A NanoVNA V2 is marked. Built-in serial ports are not listed. A missing device leaves the chart empty.

Cal is the first tab and stays at the left. The tabs after it are the zones in the station band plan, such as HF, VHF, and UHF. Bands and subbands inside a zone are not listed. Click a zone tab to show the last scan stored for that zone. If that zone has no scan yet, the click measures it. Double-click a zone tab to measure it again. Press Scan to measure the selected zone. While that measurement is running, the zone tab is dark red. Run keeps sweeping that range until Stop. A sweep without open, short, and load for this device and those limits is not a valid scan, and the chart does not keep it. Scan and Run then open the calibration dialog instead of measuring the device under test. The point buttons are 11, 51, 101, 201, 301, 501, and 1023. Scan, Run or Stop, Setup, and M1 through M4 are a column on the left edge of the chart. The view buttons are at the top of the chart frame. The chart views are Smith (the same resistance and reactance circles as the Smith screen; Y adds the admittance grid), Polar, SWR, return loss, R and X, S21 in dB (the Bode magnitude), phase, unwrapped phase, group delay, and TDR. Click the chart to drop the selected marker. M1 through M4 each keep a point. Hover the Smith chart to read the impedance under the pointer. The connection status is the first line of the left frame. The selected marker's gamma, SWR, return loss, mismatch loss, and impedance are in the left frame. Setup also lists delta M1 minus M2, resonance, and the S21 peak (3 dB, 6 dB, and Q). On the TDR view, Setup shows the peak distance, the range limit, and the mean cable loss. Setup is the port, the sweep limits, and the trace options. Open, short, and load are measured from the Cal tab, not again in Setup. Velocity factor and port extension are in Setup. Smooth averages that many points on the trace. Host avg keeps that many sweeps and averages them. Both start at 1, which is off.

Cal lists every zone and the open, short, 50 ohm, cable, and both-loads sweeps stored for it: point count, frequency span, and the first sample. A square reload button on the left of a line repeats that one step for that zone. Rescan all repeats every step for every zone, even when a set is already stored. The dialog closes when the last piece is stored or skipped. The chart button row does not insert a status line. The V2 does not store your calibration. Commander keeps one set per zone tab, from that zone's lowest frequency to its highest. If this device already has an open, a short, and a 50 ohm load for that span, Commander sweeps the stored grid and scans. If it does not, a dialog opens over the chart. The diagram marks the jacks S11 and S21, the same labels as on the device. It draws what is fitted on those jacks: nothing for the open, the short, the 50 ohm load, or a cable from S11 to S21. The open, the short, and the 50 ohm load do not have to be finished on one zone before the next. Each piece is fitted once, and every zone that still needs a calibration is swept before the next piece. Step 1 leaves S11 empty and S21 empty, then Measure open. Step 2 screws the short onto S11 once, then Measure short. Step 3 screws the 50 ohm load onto S11 once, then Measure load. A cable from S11 to S21 is optional: fit it once, Measure cable, or Skip cable. A load on each jack is optional: fit them once, Measure both, or Skip loads. A zone that already has all three pieces is left as it is. Scan beside the chart does not run during these steps. When the three pieces are stored, take them off, connect the antenna or the device, and press Scan on the zone. The set is stored for that zone in the VNA file, not in the station file. A station `City.HSCommander` keeps it at `City.HSVNA`, in the same folder. That file holds the same calibration records and the most recent scan of each zone. The station file keeps the port and the sweep settings. An older station that still has calibration inside it is copied into the VNA file the next time it is opened. The device is not given the coefficients. One piece is kept even before the other two are stored. Cancel drops the step you have not measured yet. Backup writes the sets to a JSON file. Restore loads that file into the VNA file. Clear cal drops the sets for the connected device. Every stored piece is written to the VNA file at once, and again when Commander closes. The file is written to a temporary name and then renamed, so a crash never leaves half a file. When the station is opened again, its calibration comes back from that file without a manual save. The NanoVNA V2 firmware reports the chip's fixed serial number, the same value as SN on the device's About screen. Commander shows it with the leading zeros of each of the three words dropped, so `00000001-0000abcd-12345678` reads `SN 1-abcd-12345678`, and a zero word reads `0`. The device details list it as DEVICE SN in that form and as SN (FULL) with all 24 digits, the same as the About screen. The read is checked when the device connects: Commander clears the serial input and reads the id twice. When the two reads match, that id is used. When they differ, a third read is made, and the id is used if it matches either of the first two. A read that times out counts as one that matches nothing. An id of all zeros or all FF is used only if all three reads give it. Otherwise the id is set to all zeros and the status line says ID unreliable. The status line text then turns red, the ID lamp turns red, and the line adds ID UNRELIABLE: POWER CYCLE VNA, and the Cal pane shows a red note: Device ID unreliable. The NanoVNA may be power cycling: check battery charge and USB hub power, then power cycle the device and reconnect. A NanoVNA on a low battery behind a USB hub that cannot power it tends to restart during the read. Three reads that all time out give the same note. Sets stored under older, unchecked ids are kept and not rewritten. Commander uses a set stored under this connection's id first, and otherwise the newest set for the same zone and frequency range from any id. The calibration badge then says RESTORED, with the date it was saved. It also says OTHER ID when the set was stored under a different, non-zero id, and the note line names both ids. A set whose zone or frequency range does not match the sweep is not applied, and the sweep shows as uncalibrated (or CAL INT when a covering set can be interpolated). If the VNA file cannot be read, it is renamed to `Name.HSVNA.corrupt-<date>` and kept, the note line says so, and the station opens with no calibration instead of overwriting the damaged file.

CSV, s1p, and s2p write the trace you see, including calibration and port extension. The s2p file says S12 and S22 were not measured. PNG is a picture of the current chart. Screen asks the device for its display. Pixel order follows the published Python example and has not been checked on this hardware.

TX power and on-device averaging appear only when the firmware is 1.0.2 or newer and the sweep starts at or above 140 MHz. Below that, the screen says so. Commander does not flash the unit. A white screen after you hold the left button while switching on is the bootloader. Use `bootload_firmware.py` or NanoVNA-QT. Do not use a standard USB DFU tool. If ModemManager takes the port, install `docs/nanovna-v2.rules` by hand and add your user to the `dialout` group. Commander does not install that rule.

### NanoVNA page layout and keys

Layout·1, Layout·2, and Layout·4 split the chart into one, two, or four panes. Up to four traces can be shown. Each trace has its own format, such as S11 return loss, S21 dB, Smith, or S21 group delay, and its own pane. +Trace adds a trace. Pane → moves the active trace to the next pane. Several traces in one Cartesian pane are drawn over each other in the trace colors. Double-click a pane header to fill the chart with it, and double-click again to restore the grid. Max (X on the keyboard) does the same for the active trace's pane: click maximizes or restores it, double-click (Shift+X) maximizes the next pane, and triple-click (Ctrl+X) restores the grid. While a pane is maximized the key reads Restore. Each pane keeps its own scale. The layout is saved in the station file.

A header line sits above each pane: the trace number, the parameter, the format, the scale per division, the reference, AUTO or MAN, memory and math flags, AVG and the count when host averaging is above 1, LIMIT PASS (light green) or LIMIT FAIL (light red) when limit lines are on for that trace, and the calibration badge. Clicking the header opens that trace's menu. When a pane is narrow, the header drops sections from the right instead of overlapping them. Couple on shows the markers on every pane at the same frequency. Couple off shows them only on the active pane.

Marker search: Max, Min, and Next pk move the active marker. TARGET takes a value in the trace unit, and ◀ Tgt or Tgt ▶ move the marker to the next point where the trace crosses it. Track follows the maximum or the minimum on every sweep. Peaks→M puts M1 to M8 on the highest peaks. Filter, on an S21 pane, shows the center frequency, the insertion loss, the 3 dB and 6 dB edges and widths, and Q, and draws the center and the 3 dB and 6 dB edges as labelled vertical lines on the chart. Double-click Filter (Shift+click) for notch analysis instead: the notch frequency, its depth below the pass-band (the higher sweep end), the 3 dB and half-depth widths, and Q, with the notch and its 3 dB edges drawn as lines. Triple-click (Ctrl+click) turns both off. SWR BW, on an S11 pane, shows the lowest SWR and the band where SWR stays under 2:1.

The Graph button beside each stored calibration standard opens that standard in its own window. Click opens the Smith chart (polar for the cable and loads-on-both-jacks steps). Double-click opens return loss (S21 dB). Triple-click opens phase. A light grey ideal trace is drawn behind the measured one, and R marks the point furthest from ideal, with the value in the summary line. Compare draws another stored set over it. Err terms plots directivity, source match, and reflection tracking. A wild source match or tracking curve usually means a standard was fitted wrongly. PNG and CSV save what is shown. Clicking inside the graph places markers as on the live sweep, on the nearest point: click moves M1, double-click or Shift+click adds the next marker (up to M8), and triple-click or Ctrl+click clears them. On Smith and polar views the nearest point to the click is used. A line above the graph reads each marker's frequency and value in the shown format. Markers are not placed on the Err terms view.

Keys with three small dots on the right edge have click modes. Green is click, orange is double-click, and purple is triple-click. The same actions are on the keyboard while no text box has focus. A letter alone is the click, Shift and the letter is the double-click, and Ctrl and the letter is the triple-click. The letters are R Run, L Layout, T Trace, S Store, A Auto, K Limits, M Marker, X Max, and I for the status line. With the mouse, Shift+click on a key with dots is its double-click and Ctrl+click is its triple-click, the same as rustcars mode buttons. This also applies to the calibration badge and the chart. The left and right arrows move the active marker one point. The tooltip on each key lists all three actions and their keys.

### Conventions: keep in step with rustcars

Click modes and their modifiers come from rustcars, the interface library every screen uses: click is the primary action (green), double-click or Shift is the first alternate (orange), and triple-click or Ctrl is the second alternate (purple). The NanoVNA keyboard letters follow the same rule. Before adding a new shortcut or modifier anywhere in Commander, check it against rustcars (its mode buttons and any keys it handles) so the two never give the same key two meanings.

Each calibration set records when it was measured as an ISO 8601 date-time in UTC, such as `2026-10-25T22:40:17Z`. The calibration badge, the Cal list, and the Graph window show it to the minute. A set saved before this was added shows no date.

---

# 9. CAT control and GQRX

## 9.1 The CAT Control frame

Config shows a table: Name, Driver, Device, Listen, and Status. Click a row to make it the Tune radio. Other rows stay running. Each of them keeps a light frequency poll about every two seconds. The selected row is the one the Tune page drives.

Listen is `127.0.0.1` and a port. Ports are chosen from 4532 upward, the first free port.

`Add hamlib` opens the Hamlib radio window. `Add GQRX` opens the GQRX window. `Edit` opens the selected row. `Remove` stops that row and the process group Commander started for it. Closing the Commander window stops every process group it started.

Under the buttons, USB ports lists `/dev/serial/by-id` when those links exist, otherwise `/dev/ttyUSB*` and `/dev/ttyACM*`. The label `none` means no port was found. The list updates when a device appears or disappears.

## 9.2 Add a Hamlib radio

1. Press `Add hamlib`.
2. Type a Name.
3. Pick a model in the list, or type the model number in Model. The list is `rigctld -l` in fixed columns.
4. Pick a Device from the USB ports, choose `none`, or type a path.
5. Set Serial speed, CI-V, PTT type, and PTT device to what that radio's Hamlib model expects. Leave a field blank when the model does not use it.
6. Extra `-C` is passed through as Hamlib configuration. The program does not add `-o`.
7. Press `Save`.

The port is assigned when the row starts. The window shows `Listen  127.0.0.1:` and the port, or `assigning` before that.

A missing device node waits in `device missing`. It is not a failed save.

## 9.3 Add a GQRX receiver

1. Press `Add GQRX`.
2. Type a Name. The note under the name says Commander starts this GQRX and stops it on exit.
3. A new row has no input device. The window says so. GQRX will open its I/O chooser.
4. Press `Save`.

Commander writes a GQRX config for that name and starts `gqrx` on it. Remote control is turned on, bound to 127.0.0.1, and the port is the one in the Listen column. Status stays `choose I/O device` until that port accepts a connection.

Pick the device in the GQRX chooser and let GQRX finish starting. When the port has been up, a later exit folds the device you chose back into the station row. `crashed` is stored false so the next start does not open GQRX's crash guard.

If you remove the row, or close Commander, before the port has ever accepted a connection, the chooser is not copied back. A device GQRX may have written only because the window was closing is not stored.

An older station row with no GQRX settings adopts a config file that is already on disk for that name. It does not copy your personal `~/.config/gqrx/default.conf`.

GQRX rows do not ask for a model, a serial port, CI-V, or PTT.

Do not point a new row at a config whose `crashed` flag is true and whose input device is a radio that is not on the network. That is the GQRX crash guard, and the chooser will not appear until `crashed` is false and the dead device line is gone. A new row from `Add GQRX` already meets that condition.

## 9.4 Which radio Tune uses

Select the row. The Tune page follows that row after the port is up. Opening a station file does not by itself reconnect. The face settings in the file are sent when that row's session is published.

Demo leaves the selected radio running and shows the in-process radio instead. Press Demo again to return.

## 9.5 Amplifiers

Config → Automation has an Amplifiers grid under Rotators. Add opens the Amplifier window: name, a model from `ampctld -l` (or Hamlib 4.6.5's list when that fails), device, serial speed and extra `-C` settings. Commander starts one `ampctld` for each row on 127.0.0.1, on the first free port from 4534, and restarts it two seconds after it exits. A row whose serial device is unplugged shows "device missing".

Each second the row shows frequency, power state (OFF, ON, STBY, OPER), SWR and forward power when the backend reports them. The line under the keys lists every level the backend reports. Standby, Operate, Off and Reset act on the selected amplifier. Commander sends them only when you click. Opening a station file or restarting a daemon never puts an amplifier in Operate.

Under the keys, a live amplifier has a **Freq MHz** field with **Set** (sends `set_freq` once) and a **Follow rig frequency** checkbox. Follow is off by default and saved per amplifier (`follow_rig` in the station file). When it is on, Commander sends the active rig's frequency with `set_freq` after it has stayed put for 0.4 s; it never sends power or level commands. Levels that the backend lists as settable (`set_level ?`) get a **1–10** slider and a **Set** key. Step n sends n ÷ 10 to Hamlib (10 is 1.0); the slider starts at the amplifier's reading × 10, rounded and held to 1–10 (a reading of 0 shows 1). The raw Hamlib value is in the slider's tooltip; an amplifier that lists none says so. The Dummy's levels are all read-only meters. A refused command shows its `RPRT` code in the row's status.

Use the Dummy (model 1) to try the screen. Amplifier rows are saved in the station file as `amplifiers`. Each row has a fixed `id` (32 random hex digits) made when the row is created. It never changes, even when you rename the amplifier, and it is not taken from the name. A file without ids gets them on load. If two rows share an id (for example a copied file), the later row gets a new one. Amplifiers are not written to the Gpredict hardware files; Gpredict has no amplifier devices.

---

# 10. Audio

## 10.1 One card per radio

Each CAT row gets two PipeWire loopbacks while Commander is running:

- Receive is a sink named `hamspot_<id>_rx`. The radio plays into it. The description is the make, the model, the connection id, and `RX`.
- Transmit is a source named `hamspot_<id>_tx`. The radio captures it. The description ends in `TX`.

`<id>` is the connection id in the Audio frame, not the text you typed in Name.

GQRX is started with that sink and that source as its Pulse sink and source. Hamlib does not play audio by itself. The loopbacks are still created so you can wire a speaker and a microphone.

The defaults are the computer's default playback device and default capture device.

## 10.2 The Audio frame

The frame is on the Config page under CAT Control. Each row shows the connection id, the make, and the model, then a playback drop-down on the RX line and a capture drop-down on the TX line.

If Helvum is on `PATH`, the Helvum button starts it. The button stays visible and is dark when `helvum` is not an executable on `PATH`. Commander does not quit when you close Helvum.

The line under the drop-downs says: "Drag a wire from a microphone to the radio, or from the radio to a speaker."

The picture is three columns. Microphones and other capture devices are on one side, the radio in the center, and speakers and other playback devices on the other. Drag only between a device and a radio. System devices use the desktop's icons. The radio, the speaker, and the microphone use the drawings shipped with Commander.

A wire you drop is the same change as picking that device in the drop-down.

`Other hamspot devices` lists `hamspot_` nodes that are not the RX or TX card of a current row.

## 10.3 What is kept

With a station file open, devices and wires whose names start with `hamspot_` are written into the file after the graph settles. Commander does not fight a wire you move in Helvum. It records the move.

Deleting a radio card that Commander has already seen turns that row's RX or TX flag off. Commander does not immediately create the card again.

The loopbacks exist only while Commander is running. Helvum does not show `Hamlib Dummy 1 RX` after you quit.

## 10.4 Nodes this program does not create

Do not add null sinks named Radio-Playback, Radio-Mic, Radio-EQ-In, or Radio-EQ-Out. Those are not Commander cards. A real sound card and the `hamspot_<id>_rx` and `hamspot_<id>_tx` nodes are the graph this window uses.

---

# 11. A persistent Ubuntu live USB

## 11.1 What you are installing

The zip is a folder, not an Ubuntu package and not a new squashfs. Unzip it and run `hamspot-commander.sh`. The script finds its own directory, so the folder can move. Config, window state, and the GQRX files Commander writes stay inside that folder.

The read-only live image does not contain Commander, `gqrx`, or `rigctld`. A normal live boot forgets every file you unzip into the home directory and every package you install. Persistence is the part that keeps them.

You do two different writes:

1. Put `HamSpotCommander` on a Linux filesystem that the persistent session keeps.
2. Install `gqrx-sdr`, `libhamlib-utils`, `pipewire-bin`, and `helvum` with apt while that same persistent session is running, and add your user to `dialout` and `plugdev`.

The next boot has to be the persistent boot entry. A plain "Try Ubuntu" entry starts a clean image and will not show the folder or the packages.

## 11.2 Make the stick

Use a USB drive you can erase. Ubuntu 26.04 desktop is several gigabytes, and the persistence area has to hold package installs on top of that. A 32 GB stick is a practical minimum. Larger is more comfortable once GQRX and Hamlib are installed.

On a working Ubuntu machine, create the stick with mkusb and choose its persistent-live operation. That is the Ubuntu community tool that adds a writable overlay. The desktop Startup Disk Creator writes a live USB and does not add this overlay.

mkusb will ask you to confirm the target drive. Confirm the device name with `lsblk` before you accept.

> Important. mkusb erases the drive you select. A wrong disk is gone. Read the device name in the prompt and match it to `lsblk` before you accept.

Current mkusb labels the overlay partition `writable`. Older mkusb layouts label it `casper-rw`. Either label is the overlay. mkusb also creates a separate data partition, often named `usbdata`, that you can open from another computer. Use `usbdata` to carry the zip. Do not run the program from that partition when it is NTFS or FAT. The binary needs a Linux filesystem so it stays executable.

After mkusb finishes, boot the persistent menu entry, not the plain live entry.

## 11.3 Confirm the boot is persistent

Open a terminal on the live session and run:

```bash
findmnt -T /
lsblk -o NAME,LABEL,SIZE,FSTYPE,MOUNTPOINT
```

`findmnt` should show an overlay on `/`. `lsblk` should show a partition labeled `writable` or `casper-rw`. If `/` is only the read-only image, stop. Packages and an unzip into the home directory will disappear at shutdown. Boot the persistent entry, or recreate the stick with mkusb's persistent-live operation.

The live user is the one the Ubuntu image logs in as. Commands below use that user's home directory.

## 11.4 Copy the zip onto the stick

From any computer, copy `hamspot-commander-0.8.5-portable-linux-x86_64.zip` onto the `usbdata` partition, or download the release onto the live session after you have confirmed persistence.

The release page is chapter 1.7.

## 11.5 Unzip into the home directory

On the persistent live session:

```bash
ZIP=/media/$USER/usbdata/hamspot-commander-0.8.5-portable-linux-x86_64.zip
mkdir -p "$HOME/Applications"
unzip "$ZIP" -d "$HOME/Applications"
```

Change `ZIP` if you downloaded the archive somewhere else. The archive already contains the folder `HamSpotCommander`, so the tree is:

```text
$HOME/Applications/HamSpotCommander/hamspot-commander.sh
$HOME/Applications/HamSpotCommander/bin/commander
$HOME/Applications/HamSpotCommander/bsp/
$HOME/Applications/HamSpotCommander/config/
$HOME/Applications/HamSpotCommander/data/
```

`config`, `data`, `cache`, and `state` are created by the launcher if a copy omitted them. Band plans are in `bsp` next to `bin`.

Check that the binary is executable:

```bash
test -x "$HOME/Applications/HamSpotCommander/bin/commander" && echo OK
```

If that does not print OK, the unzip landed on a filesystem that dropped the executable bit. Unzip it again onto `$HOME/Applications`, which is on the overlay.

## 11.6 Install the packages the folder does not contain

Still on the persistent session:

```bash
sudo apt-get update
sudo apt-get install gqrx-sdr libhamlib-utils pipewire-bin helvum
```

Confirm the four programs:

```bash
command -v rigctld
command -v gqrx
command -v pw-loopback
command -v helvum
```

Each line should print a path.

These packages land on the overlay. They are still there after a reboot only when you boot the persistent entry again. They are not copied into the read-only squashfs by this procedure, and they are not inside the zip.

## 11.7 Device permissions

A live user is not always in `dialout` and `plugdev`. Without `dialout`, a serial radio port will not open. Without `plugdev`, some USB device nodes will not open.

```bash
sudo usermod -aG dialout,plugdev "$USER"
```

Log out and back in, or reboot and choose the persistent entry again. Group membership is read at login. Check it:

```bash
id
```

The line should include `dialout` and `plugdev`.

Ubuntu's default rules already cover `/dev/ttyUSB*`, `/dev/ttyACM*`, and `/dev/serial/by-id` for members of `dialout`. You do not add a separate udev file for this program.

## 11.8 Start Commander

```bash
"$HOME/Applications/HamSpotCommander/hamspot-commander.sh"
```

The script sets the config, data, cache, and state directories to folders inside `HamSpotCommander`, then runs `bin/commander`. Station files you save from the Config page default to that config directory if you navigate there. GQRX files Commander writes for a portable launch go under `HamSpotCommander/config`, because that directory is the config home.

The desktop file in the folder is named Hamspot Commander. Its launch line uses the desktop file's own location, so it still works after you move the folder. If the file manager will not start a desktop file, use the shell command above. That command is the one this guide relies on.

PipeWire stays on the session socket. Do not export a private runtime directory before you start the script.

## 11.9 Reboot and see that it stuck

1. Shut down.
2. Boot the same stick and choose the persistent entry again.
3. Run `findmnt -T /` and confirm the overlay.
4. Run the start command in section 11.8.
5. Open Config and confirm the radio rows you saved are the rows in the station file.

A station file you saved inside `HamSpotCommander/config` is part of the folder. Copy the whole `HamSpotCommander` folder if you want that station on another machine. The zip from the release page is a fresh tree and does not contain your station.

If you boot the non-persistent entry, the home directory is empty again and apt will not list the packages you installed. The zip you left on `usbdata` is still there, because that partition is not the overlay. You would have to install the packages again.

## 11.10 What this chapter does not do

It does not rebuild the Ubuntu squashfs. The live image stays the image you downloaded. Persistence holds your changes beside it. That is the layout the portable folder was built for: no home directory is baked into the launcher.

Rebuilding the ISO so that Commander and the four packages are inside the read-only image is a separate remaster. This release does not ship that ISO. If you later remaster, keep the `HamSpotCommander` folder as a normal directory and leave its config path relative. Do not replace `hamspot-commander.sh` with a script that points at `/home/user`.

---

# 12. An installed copy

## 12.1 When to use the installer

Use this chapter when Ubuntu 26.04 is installed on the computer, you have a source checkout, and you want `commander` on your account. The installer refuses any other distribution. It reads `/etc/os-release` and stops if the id is not `ubuntu` or the version is not `26.04`.

The portable zip in chapter 11 does not use this script.

## 12.2 Install

From the source tree:

```bash
scripts/install.sh
```

The script asks for sudo without a password prompt. On a machine where sudo requires a password in the terminal, run it from a terminal that can authenticate, or arrange passwordless sudo for the duration of the install. If sudo cannot run, the script stops before it changes packages.

It installs the packages in section 1.5, including the compiler, then builds the release binary and `dsd-fme`. It copies:

| Installed path | What it is |
| --- | --- |
| `~/.local/bin/commander` | The window |
| `~/.local/bin/dsd-fme` | The digital voice decoder |
| `~/.local/share/applications/hamspotcommander.desktop` | The desktop entry |
| `~/.local/share/icons/hicolor/128x128/apps/hamspot-commander.png` | The icon |
| `~/.local/share/hamspot-commander/installed.manifest` | What this install added |

The desktop name is Hamspot Commander. The executable line points at `~/.local/bin/commander`.

You are added to `dialout` or `plugdev` only if you are not already a member. The manifest records `added_dialout=yes` or `added_plugdev=yes` only in that case. Log out and back in if a group was added.

The script does not start Commander or GQRX.

## 12.3 Remove

```bash
scripts/uninstall.sh
```

Removal deletes the binary, the desktop file, and the icon recorded in the manifest, when those paths are under your home directory. It also deletes the `dsd-fme` and `mbelib` files listed in the manifest. It removes you from `dialout` or `plugdev` only if the manifest says this install added you. It does not remove apt packages. GQRX, Hamlib, PipeWire, and the compiler stay installed. The decoder source cache under `~/.cache/hamspot-commander` stays as well.

A second install leaves `added_dialout` and `added_plugdev` as they were, so a later uninstall does not drop a group you already belonged to.

Station files you saved, and anything under `HamSpotCommander`, are not part of the manifest. Uninstall does not delete them.

---

# 13. Appendix

## 13.1 Portable folder

| Path inside `HamSpotCommander` | Role |
| --- | --- |
| `hamspot-commander.sh` | Sets the four data directories, then execs the binary |
| `hamspotcommander.desktop` | Desktop entry that follows the folder |
| `hamspot-commander.png` | Icon used by the relocatable desktop file |
| `bin/commander` | The window |
| `bsp/*.json` | Band plans. Startup uses `default.json` |
| `config/` | Station files and GQRX INI for a portable launch |
| `data/` | Window state |
| `cache/`, `state/` | The other two relocated directories |
| `share/applications/` | A second desktop file, for the app id search path |
| `share/icons/hicolor/128x128/apps/` | The same icon on that search path |

The installed copy in chapter 12 does not use this layout. Its binary is `~/.local/bin/commander`, and its window state defaults to `~/.local/share/hamspotcommander`.

## 13.2 Station file, short form

Version must be 1. A file with a field this release does not know is rejected.

Radios and the GQRX object may be absent. An older file still loads. An empty radio list starts the Dummy row.

The plan is stored in the file, and `bsp_name` says which file name under `bsp/` to write only when that file is not already there.

Talkgroups are inside the station file. Key text is not. The matching `HSKeys` file sits beside the station. `tabs`, when present, is the page-tab order. Each entry is a page name such as `Tune` or `Talkgroups:3`. `nanovna`, when present, is the NanoVNA V2 port and sweep. Omit it unless you set one of those. Calibration and zone scans are not in the station file. They are in the matching `HSVNA` file beside it. `docs/AGENT-HSCommander.md` is the field map for building a new station from a listing.

## 13.3 Frequency entry

| You type | Meaning |
| --- | --- |
| `0014.153.608` | 14153608 Hz |
| `14.153608` | 14.153608 MHz |
| `14153608` | 14153608 Hz |

Press Enter in the text under the digits. The wheel and the digit clicks do not use this parser.

## 13.4 Dark controls

Dark on every radio, on the Tune control row: `Dimmer`, `NR sub`, `CW Spot`, `Keyer`, `Nar`, `Ant Rx`, `Chan +`, `Chan -`, `QMR`, `BC`, `PWR`, `QMS`, `Contour`, `Scan`, `Scan Dn`.

Dark on the Levels page: `AF gain (sub)`, `RF gain (sub)`, `AGC (sub): Auto Fast`, and `Contour: 20 Hz`.

Drawn on Meters and not wired to the radio: `Busy`, `Mic Eq`, and `PLL unlock`. `High SWR` does follow the SWR number.

Any other control is dark when this radio's Hamlib dump does not list it. That is expected. The dummy lists more operations than a typical GQRX row, and GQRX lists no PTT.

## 13.5 Process boundaries

Commander starts `rigctld`, `gqrx`, and `pw-loopback`. It stops those process groups on Remove and on exit. It does not stop a GQRX you started from the application menu, and it will not drive a `rigctld` you started in a terminal.

The first free TCP port at or above 4532 on 127.0.0.1 is the listen port. Nothing in this release listens on another address.

## 13.6 Further reading in the source tree

`README.md` is the short project readme. `docs/AudioGraph.md` names the PipeWire nodes for a Hamlib dummy wired to a USB sound device. `docs/AGENT-HSCommander.md` tells another session how to turn a listing into a station file and, when the operator supplies key ids, the key file beside it. This user guide is the operator manual. Where the readme still speaks of an older screen layout, trust the window and this guide.

## RF patch bay (Config tab)

The RF patch bay at the bottom of the Config tab is a drawing of how coax runs in your station. It is **visual only**: nothing on it switches, keys or tunes hardware yet.

- **Devices.** Every radio row in the station appears automatically (HackRF, GQRX and line taps are marked RX ONLY). Add amplifiers (the Hamlib `ampctl` models, or "Other amplifier"), antenna switches, tuners, SWR/power meters, dummy loads and antennas from the Add row. Click a box to rename it, note its bands, set a switch's ways and position, or remove it. Drag a box by its coloured title bar.
- **Jacks.** Inputs are on the left of a box and outputs on the right, in the transmit sense; received signal flows the other way. Round jacks are coax, small square jacks are PTT/ALC control.
- **Cables.** Drag from a jack to another jack. The end snaps to the nearest jack that accepts it. Output to output, input to input, coax to control, a box to itself and a jack that already has a cable are refused, and the reason shows below the panel. Drag a plugged end to move it; drop it on empty panel to unplug it. Right-click a cable, or click it and press Delete, to remove it.
- **Flow.** Chevrons move along each cable in the direction of the signal, with a TX (amber) or RX (teal) tag. Below the panel the bay lists each path, for example `TX path: IC-9700 → KPA1500 → ATU → Yagi`, and warns about open outputs, loops, an amplifier fed from nothing, or an amplifier in line with a receive-only radio.
- The layout is saved in the station file under `patchbay` (devices with positions, and cables from an output jack to an input jack). Older station files load with an empty bay.

- **Configured amplifiers (#96).** The amplifier model list is the same one the Automation window uses (`ampctld -l`, with Hamlib 4.6.5's list as fallback). Click an amplifier box and pick a **Configured amplifier** from the Automation list (name plus the start of its id), or leave it "unbound (visual only)". The box is bound to the amplifier's fixed id, never to its name or position, so renaming keeps the binding and two amplifiers with the same name cannot be mixed up. **Add to Automation list** creates a matching amplifier row with a new id (Dummy model when the box has no Hamlib model) and binds the box to it; it sends nothing to the amplifier. A bound box shows a power LED with its word (OFF, ON, STBY, OPER), the frequency, whether the daemon is up or the device is missing, SWR, forward power and any fault. If the bound amplifier is removed, the box shows **amplifier missing** and sends nothing; it never falls back to another amplifier. Every command (Standby, Operate, Set freq, follow rig, levels) looks the amplifier up by id when it is sent. When a station file is loaded, boxes bound to a duplicated id are unbound with a note, and boxes saved by an early test build that bound by name are moved to the id only when exactly one amplifier has that name; otherwise they are unbound with a note.
- **Power keys.** A bound, live box has Standby and Operate (amber) keys. They send only on that click, like the Automation keys. The patch bay never sends Operate on its own.
- **Amplifier warnings (#97).** With a bound amplifier on a TX path, the bay warns when the amplifier is OFF, ON or STBY while the TX path is patched through it, and when it is in OPERATE but the path does not end in an antenna or dummy load. These are reminders only.

Still to do (#97): nothing in the bay switches or keys hardware. Antenna switch control, PTT/ALC and automatic operate are not built and are not planned without a separate decision.

### VNA status line and device lamps

The bar under the VNA chart shows the sweep (START, STOP, PTS, AVG, run mode, calibration) on the left and only the device type and port on the right, for example `NANOVNA V2 · ttyACM0`. The left text never runs into the right: when the window is narrow, AVG is dropped first, then calibration and the run mode, then the sweep summary. Warnings (marked ⚠) are never dropped; if the warnings alone do not fit they are cut with `…`. Firmware, sweep time, sweep count, calibration and warnings are always listed in the device details, which open with a click on the bar or on a lamp (I). Double-click (Shift+I) reconnects and triple-click (Ctrl+I) copies the full status line.

Four small lamps sit left of the device name. Hover over a lamp to read what it means; every warning lamp also has text on the bar or in the details, never colour alone.

- LINK: green connected, red disconnected (lost, check USB), grey not connected.
- ID: green serial number read and checked, red ID unreliable (power cycle the VNA), grey no id.
- CAL: green calibration applied (exact or restored), amber interpolated (CAL INT) or correction off, grey uncalibrated.
- SWEEP: green sweeping, amber sweep errors, grey held or not connected.


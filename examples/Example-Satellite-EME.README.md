# Example-Satellite-EME.HSCommander

An example station that lists the amateur satellite and moonbounce (EME) segments a HackRF can receive. Each band is a plan zone, and each zone has subbands by mode. The radio is a HackRF set to receive only. The rotator is the Hamlib dummy, model 1.

- `bsp_name` is `satellite-eme.json`. The plan is written inside the station and does not come from `bsp/`. Opening the station may install it as `bsp/satellite-eme.json` if no file with that name exists.
- Radio: `driver: "hackrf"`, `model: 0`, `device: ""` (the only HackRF), `audio.tx: false`, and no RX or TX pipe. `sample_rate` is left out, so it loads at 8 Msps, the app's default window. Commander tunes inside an ~8 MHz window around the centre, so a band wider than that (such as 70 cm satellite or 23 cm uplink) is covered by retuning.
- Rotator: `{ "name": "Dummy", "model": 1 }`, which is Hamlib's dummy rotator, run through `rotctld -m 1`.
- The station opens on the Channels page, tuned to 145.800 MHz FM (ISS voice).
- Receive only. Do not transmit on a satellite downlink.

## Zones and subbands (MHz)

Use letters: CW = cw, DIG = digital, V = voice, MIX = mixed/all modes. A gap between segments is left unassigned on purpose.

| Zone | Band | Subbands |
| --- | --- | --- |
| HF 10 m | 10 m 28.000–29.700 (USB) | 28.000–28.070 CW · 28.070–28.150 DIG · 28.150–28.190 CW · 28.300–29.300 V · **29.300–29.510 satellite (MIX)** · 29.520–29.700 FM V |
| VHF 2 m | 2 m EME/Weak 144.000–144.300 (USB) | **144.000–144.050 EME CW** · 144.050–144.100 CW/weak · **144.100–144.275 EME & weak SSB (144.200 call)** · 144.275–144.300 beacons |
| | 2 m New OSCAR 144.300–144.500 | MIX |
| | 2 m Satellite 145.800–146.000 (FM) | MIX |
| UHF 70 cm | 70 cm EME/Weak 432.000–432.400 (USB) | **432.000–432.070 EME** · 432.070–432.100 CW · 432.100–432.300 SSB (432.100 call) · 432.300–432.400 beacons |
| | 70 cm Satellite 435.000–438.000 (FM) | MIX |
| L 23 cm | 23 cm Sat Uplink 1260–1270 | MIX |
| | 23 cm EME/Weak 1295.800–1296.400 (USB) | **1295.800–1296.080 EME** · 1296.080–1296.200 weak CW/SSB (1296.100 call) · 1296.200–1296.400 beacons |
| S 13 cm | 13 cm EME/Weak 2303.750–2304.750 (USB) | 2303.75–2304.000 weak · **2304.000–2304.100 EME** · 2304.100–2304.300 weak SSB/CW (2304.100 call) · 2304.300–2304.400 beacons · 2304.400–2304.750 weak/NBFM |
| | 13 cm Satellite 2400–2410 | MIX |
| S 9 cm (not US) | 9 cm EME 3400.000–3402.000 | MIX, 3400.100 EME centre (IARU R1). |
| C 5 cm | 5 cm Sat Uplink 5650–5668 | MIX |
| | 5 cm EME/Weak 5760.000–5760.800 | 5760.000–5760.050 · **5760.050–5760.150 EME** · 5760.150–5760.800 (MIX) |
| | 5 cm Sat Downlink 5830–5850 | MIX |

## Channels (receive frequencies)

- **ISS** (ARISS): voice downlink 145.800, APRS 145.825, SSTV 437.550, repeater downlink 437.800, UHF packet 437.825.
- **FM satellites** (downlinks, from the AMSAT FM summary updated 18 Oct 2026): PO-101 145.900, AO-91 145.960, AO-123 435.400, IO-86 435.880, TEVEL2 436.400, SO-50 436.795, RS95S 436.950, CAS-3H 437.200.
- **Weak-signal / EME calling**: 144.200, 432.100, 1296.100, 2304.100, 3400.100, 5760.100.

**These change over time.** Satellites fail, and the ISS radios change modes. On 18 Oct 2026, AMSAT said PO-101 seems to be dying, AO-91 should be used only in sunlight, TEVEL2 is unreliable, RS95S is on only during testing, and the CAS-3H transponder runs with no set schedule. Before a pass, check https://www.amsat.org/status/ and ARISS. Doppler shifts a 435 MHz downlink by up to about ±10 kHz.

## Left out, and why

- **3 cm (10368 MHz EME) and higher**: above the HackRF's 6 GHz limit. QO-100 (10489 MHz downlink) is left out for the same reason.
- **9 cm**: included, but the FCC is removing US amateur access at 3.3–3.5 GHz, and NTMS says 3400 MHz is the worldwide EME frequency. It is marked not US.
- **15 m and 20 m**: no amateur satellites or EME are active there now, and the band plans do not mark a satellite segment, so they were not added as satellite zones. The normal HF bands are in `bsp/default.json`.
- **6 m and 1.25 m EME** (50 MHz and 222.0–222.025): not asked for. They are easy to add.
- **EME digital sub-frequencies** (for example 144.174 FT8 or 432.065 Q65): conventions I couldn't confirm from a band plan, so they were not stored. The EME segments cover them.
- The 5 cm satellite downlink 5830–5850 is the ITU allocation and is not a specific satellite.
- The plan follows the ARRL and Region 2 plan. Region 1 differs: 2 m EME is 144.000–144.035, 23 cm EME is 1296.000–1296.025, and 13 cm EME is 2320.

## Sources

- ARRL band plan: https://www.arrl.org/band-plan (2 m, 70 cm, 23 cm, 13 cm, 10 m).
- AMSAT FM satellite summary: https://www.amsat.org/fm-satellite-frequency-summary/ (updated 18 Oct 2026).
- ARISS ISS frequencies: https://www.ariss.org/contact-the-iss.html
- W5LUA / NTMS "Moonbounce Basics": https://www.ntms.org/files/Mar2021/Collins_Radio_Club_W5LUA_EME.pdf (1296.0–1296.150, 2304.0–2304.150, 3400, 5760.050–5760.150, 10368).
- IARU Region 1 SHF band plan: https://on7fd.net/wp-content/uploads/2022/12/SHF-Bandplan.pdf (3400.100 EME centre, 5650–5668 uplink, 5790–5850 downlink).

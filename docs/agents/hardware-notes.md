# Hardware notes

As of 2026-10-27.

- **Scope:** NanoVNA V2 (V2.2, S-A-A-2, USB `04b4:0008`) and HackRF only. Other NanoVNA families and SDRs are out of scope; note unsupported items once in the overview issue.
- **NanoVNA ID read.** Firmware reports the chip's fixed serial. Commander clears input and reads twice; on mismatch a third read decides; all-zero/all-FF accepted only if all three agree; otherwise the id is zeroed and "ID UNRELIABLE: POWER CYCLE VNA" is shown. A V2 on low battery behind an unpowered hub tends to restart during the read, so power cycling affects ID reads.
- **NanoVNA calibration** is never sent to the device; sets live in `Name.HSVNA` per zone, timestamped in UTC. Sets under another non-zero id are used and badged OTHER ID.
- **NanoVNA screen capture** pixel order follows the published Python example and is unverified on hardware. TX power and on-device averaging need firmware ≥ 1.0.2 and start ≥ 140 MHz. Commander doesn't flash; bootloader is the white screen (use `bootload_firmware.py` or NanoVNA-QT, not USB DFU). `docs/nanovna-v2.rules` is installed by hand if ModemManager grabs the port.
- **HackRF** usable bandwidth is treated as 6 MHz around centre; channels beyond ±3 MHz aren't in the capture.
- **Hamlib dummy** (model 1) receives 150 kHz–1.5 GHz; first connect parks VFO A 14.153608 USB, B 146.520 FM, C 446.000 FM.
- **Amplifier models in Hamlib 4.6.5:** 1 Dummy, 2 NET ampctl, 201 Elecraft KPA1500, 301 Gemini DX1200/HF-1K, 401 Expert 1.3K/1.5K/2K-FA.
- **Rotators:** Yaesu G-5500 uses GS-232A (601) or GS-232B (603); see `docs/rotator/`.

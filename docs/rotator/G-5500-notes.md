# Yaesu G-5500 / G-5500DC / G-5600B az-el controllers

Sources (page numbers are PDF pages):
- G-5500DC Instruction Manual 1906V-BS (DC).
- G-5500 Instruction Manual E12901004 (AC).
- G-5400B/G-5600B manual (radiomanual.info). It is a scan with no text layer and was not OCR'd, so
  nothing below is taken from it.

**[M]** = stated in a manual. **[I]** = our inference or design.

## Ranges
- [M] "450° azimuth and 180° elevation control"; the elevation meter reads "from 0° to 180°"
  (DC p.2; AC p.3).
- [M] The azimuth starts in the **north**, unlike the G-1000DXA's factory south stop. Calibration (DC p.6, AC p.6): hold LEFT to the CCW
  end stop with the meter at the left edge. One full turn RIGHT reads 360°. RIGHT again to the
  end stop reads "right edge (90°) of the scale". The meter scale runs N 0 → E → S 180 → W 270 → N 360 → E 90 (DC p.6
  drawing). The installer points the antenna at true North with the controller at 0° (DC p.9).
  So the azimuth runs 0…450 and the overlap is 0°–90° (N to E) on the second pass.
- [M] Elevation: the meter rests mid-scale at 90° (DC p.4 step 7). UP aligns the 180° marks, and the
  meter should read 180° at the right end (DC p.6).
- [M] EXT CONTROL (DC p.7, AC p.7): 2–4.5 V = 0–450° azimuth (pin 6) and 0–180° elevation (pin 1).
  Pins for right, left, up and down.

## Speed and operation
- [M] Rotation times: elevation 180° in 65 s ±20%, azimuth 360° in 60 s ±20% (DC p.2). The AC
  G-5500 is 67 s and 58 s (AC p.4). There is **no speed control** and no preset or START on these controllers;
  only LEFT/RIGHT/UP/DOWN while held. Pressing both switches of a pair turns right or up (DC p.6).
- [M] 3 minutes of intermittent duty (DC), then rest at least 15 minutes (DC p.6). The AC G-5500 is 5 minutes.
- [M] The manuals do **not** describe "flip" tracking. They only state the 0–180° elevation range.
- [I] Past 90° the antenna points over the top. Its true direction is az + 180 (mod 360) and
  el 180 − el. That lets a 360° (non-overlap) azimuth path follow a pass through the
  north/south keyhole. Commander shows a FLIP annunciator and this equivalent.

## What Commander does [I]
- Model presets: G-1000DXA south stop (az −180…270, no elevation; el 0…90 kept for the
  Dummy), G-1000DXA north stop (0…450), G-5500/G-5500DC (az 0…450, el 0…180), G-5400B/G-5600B
  (assumed the same as the G-5500; not confirmed from the scan), and Generic (0…360, 0…90).
- **Hamlib limits come first.** Once rotctld is up, Commander reads `\dump_state` (Minimum/Maximum
  Azimuth/Elevation, which honors `-C min_az=…` conf). Those limits are used when they are sane.
  The model preset is the fallback, and the tab shows `limits: hamlib` or `limits: preset`.

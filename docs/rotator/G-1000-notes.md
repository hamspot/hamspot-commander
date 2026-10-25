# Yaesu G-1000 series controller: how it works

Main source: **G-800DXA/G-1000DXA/G-2800DXA User Manual 2102B-BS** (Yaesu). Page numbers are the
printed page numbers. Add 2 to get the PDF page. The SDX text is from the radiomanual.info /
fracassi.net copy of the G-800S/SDX & G-1000S/SDX manual (the intro page as indexed by search).
The other scans have no text layer and were not OCR'd. **[M]** = stated in a manual.
**[I]** = our inference or design.

## Rotation range and overlap
- [M] "90° overlapping rotation allows rotation through the selected stopper heading (450° total
  rotation)." (p.1). Spec: rotation range 450° ±3° (p.15). The S/SDX spec says 450° +0/−5°.
- [M] The factory start position (CCW stop) is 180° (South). The indoor check (p.8) says: hold LEFT
  to the CCW stop, and the needle should read 180°. Then RIGHT for a full 360° back to 180°. Keep
  going and "rotation automatically stops at approximately 270° (West; representing a total
  rotation range of 450° from the original starting point)" (p.8 steps 5–10).
- [M] The operator can move the stopper, for example to start at 0° (North). The PRESET then uses the
  inner 0–90 scale (p.2 note, p.9 dial heading sheet).
- [M] OVERLAP indicator (p.2): "This red LED glows when the antenna is rotated beyond about 180° (to
  270°) from its original installation direction (180°). Check this indicator before rotating the
  antenna clockwise, and turn the antenna counter-clockwise to the desired position if the
  indicator is on." Step 9 (p.8): the LED lights "as rotation passes the 180° point" on the second
  pass. The rear OVERLAP LED ADJ pot sets that threshold (p.3, p.8).
- [I] So the lamp means mechanical travel from the CCW stop is more than 360°. The bearings in the
  last 90° (180°→270° for a south stop) can be reached two ways. The dial cannot tell them apart.
  Only the lamp can.

## Speed
- [M] SPEED control: "approximately 30 and 100 seconds per 360° revolution" (p.2). Specs: 40 to
  100 s ±10/±5 per 360° for the G-800/1000DXA, and 50–120 s for the G-2800DXA (p.1, p.15). The SDX is
  43–93 s per rotation.
- [M] Only the G-2800DXA has auto slow start and slow stop (p.1). The SDX also "slows rotation speed
  as the antenna approaches the desired heading" (SDX manual intro).
- [M] The EXT CONTROL DIN (p.14) has pin 3 = 0–5 V for proportional speed and pin 4 = 2–4.5 V out for
  0–450°. That is what a GS-232B reads.
- [M] Maximum continuous duty is 3 minutes for the DXA (p.15) and 5 minutes for the S/SDX.

## PRESET / START / LEFT / RIGHT
- [M] LEFT/RIGHT seesaw: LEFT = CCW, RIGHT = CW, momentary while held (p.2, p.8).
- [M] PRESET + START: set the knob to the heading on the outer 180~270 scale, then press the
  momentary START switch "to activate automatic rotation to the desired heading. You can press the
  LEFT/RIGHT switch to abort preset operation." (p.2). The preset knob covers the full 450° (180°
  fully CCW to 270°=450° fully CW, p.8 steps 12–13). So a preset picks the travel position, not just
  the bearing.
- [I] START is only "go to preset". It is not a run/stop toggle.

## Manual tracking at a set rate
- [M] **None of these manuals describes a manual tracking-rate feature.** The SPEED knob sets the
  slew rate while LEFT/RIGHT is held or a preset runs. It does not command continuous tracking.
- [I] We design it as an operator feature. Set a rate in °/min (or °/s) and a direction (CW, CCW, or
  toward the preset), then press START/STOP. Commander steps the commanded azimuth at that rate.
  It sends `set_pos` through rotctld. It stops at the preset, at a travel limit, or on STOP/LEFT/RIGHT.
  The rate must stay below the SPEED-limited slew rate (360°/40 s ≈ 9°/s at fastest, 3.6°/s at
  slowest). A rate above that is clamped and flagged.

## Overlap handling in Commander [I]
- Model: `start` = bearing of the CCW stop (180° factory, or 0°), with 450° of travel. A rotctld
  azimuth maps to travel = az − start_az. The overlap lamp lights when travel > 360°.
- For hamlib, a south-stop rotator reports −180…270 (the dummy is −180…450 and can be limited with
  `-C min_az=-180,max_az=270`). A north-stop rotator reports 0…450.
- A preset for a bearing in the ambiguous 90° picks the travel position nearest the current one.
  We warn when a tracked path would run into a travel stop and need a long unwind.

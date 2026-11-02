# Changelog

## 0.9.0 (2026-11-01)

Since 0.8.5.

- **Tune page**: NanoVNA layout below the tuning display, keys on the left half and the spectrum scope on the right at full height (stacked on narrow windows). The sub VFO sits under the main VFO, the Fine bar is gone, and status text is in one strip at the bottom of the page. The VFO key grid has equal compact keys. VFO digits are blue seven-segment VFD digits. The scope has **Auto Floor** (on by default): the dB axis follows the noise floor and the strongest signal. LNA/VGA gain fields are slim, single-row, instrument style.
- **Rotator**: its own top tab (LED bar, detachable). Optional azimuthal-equidistant world map behind the rose, centred on a lat/lon: black ocean, dark blue land (Natural Earth 1:110m, public domain), and the globe is see-through while the map is shown.
- **Config**: Keys and Levels are Config sections. Every group and the whole page detach from their end pill. Fixed: Save refused stations with Rotator/REC/HF Data in the tab row ("tab Rotator is not a page"). Fixed: Config scrolling grew without bound and could leave a blank page; saved scroll positions are no longer reused at start.
- **Rendering**: GPU-safe default. Commander uses OpenGL only (no Vulkan/wgpu), and has a Rendering setting (Auto / GPU / Software) plus `HAMSPOT_SOFTWARE_RENDER=1`. Spectrum frames only redraw while a page shows them, and hidden windows redraw slowly.
- **Detached windows**: same page frame and elbow bar as docked, fill the window, keep drawing while the main window is minimized or covered, never maximized. Meters, Tools, Timers, REC, HF Data and Config detach with the page pill. E-STOP/RELEASE is in every detached page window.
- **Tools**: shared page frame with sub-tabs in the bar. Tool keys are a folder-tab row (RF Calc, Calc, Hex, Logbook). NanoVNA-style status strips. NanoVNA: fixed SWR scale, status strip with a STATUS window, Scan keeps the typed range, the band plan drives limits, and Run never stalls on a failed sweep.
- **Station safety**: one latched E-STOP for transmitters, amplifiers and rotators (F9-F12 chord). Station TX lockout during VNA device scans.
- **New**: HF Data page (slot clock, presets, decode list). VNA device scan wizard in the RF patch.
- **REC**: controls in the top bar, timeline never shows the future, the database is created when REC is armed, and every radio ID heard on a call is saved with its recording.
- **Buttons**: square LCARS bar buttons everywhere, toggles with a thin border and a ●/○ marker.

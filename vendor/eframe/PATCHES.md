# HamSpot patches to eframe 0.37.0

Upstream: crates.io eframe 0.37.0, copied unchanged except:

1. `src/native/glow_integration.rs`: a `PassMode::LogicOnly` pass (run when a
   window's `RedrawRequested` is overdue, e.g. GNOME Wayland withholds frame
   callbacks from a minimized or fully covered window) still runs the UI if
   the window has a visible *immediate* child viewport, so detached windows
   (immediate viewports, drawn only inside their parent's pass) keep updating.
   The parent itself is not painted in that pass. `logic_pass_plan`,
   `has_visible_immediate_child`, test `hamspot_patch_tests`.
2. `src/native/run.rs`: `MISSED_REDRAW_TIMEOUT` 250 ms -> 40 ms, so those
   passes come at about 25 fps instead of 4 — only for a window that has a
   visible detached (immediate) child (`WinitApp::hs_fast_overdue`); any other
   unseen window keeps upstream's 250 ms (`UPSTREAM_MISSED_REDRAW_TIMEOUT`).

Remove when upstream draws immediate children of an unseen parent.
3. Diagnostics: `HAMSPOT_VP_TRACE=1` prints the viewport event flow to
   stderr (`hs_trace!`, `winit_integration::hs_trace_on`): window events per
   window id (not cursor motion), `request_redraw`, overdue logic-only
   passes, each pass's mode / ui / paint / detached-child-visible / event
   count, and each detached (immediate) pass with its event and pointer
   button counts. No effect when unset.

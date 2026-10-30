//! When a new HackRF spectrum frame should redraw the window (David,
//! 2026-10-30: "work on the redraws when tuning is not visible"). Capture,
//! demodulation, audio, recording and the FFT itself are not touched; only
//! the ~15/s repaint that a frame asks for is skipped when nothing shows it.

/// What is on screen this frame, as far as the spectrum is concerned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The selected page draws spectrum data (Tune chart, TG scanner RSSI).
    pub page_shows_spectrum: bool,
    /// A detached window that draws spectrum data is open and visible.
    pub detached_shows_spectrum: bool,
    /// The main window is minimized.
    pub root_minimized: bool,
    /// A memory scan is stepping on spectrum frames (runs in the UI pass).
    pub scanning: bool,
}

/// Do spectrum frames need a redraw each? A scan always does, so it never
/// stalls when its page is hidden or the window is minimized.
pub fn spectrum_wanted(v: View) -> bool {
    v.scanning || v.detached_shows_spectrum || (v.page_shows_spectrum && !v.root_minimized)
}

/// Repaint for one new frame: when watched, or when the capture started or
/// stopped (so the page and status line are right when shown again).
pub fn repaint_for_frame(watched: bool, started_or_stopped: bool) -> bool {
    watched || started_or_stopped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_page_wants_frames() {
        assert!(spectrum_wanted(View { page_shows_spectrum: true, ..Default::default() }));
    }

    #[test]
    fn hidden_page_does_not() {
        assert!(!spectrum_wanted(View::default()));
    }

    #[test]
    fn minimized_main_window_does_not_unless_detached_shows_it() {
        let v = View { page_shows_spectrum: true, root_minimized: true, ..Default::default() };
        assert!(!spectrum_wanted(v));
        assert!(spectrum_wanted(View { detached_shows_spectrum: true, ..v }));
    }

    #[test]
    fn scanning_always_keeps_frames_coming() {
        for page in [false, true] {
            for min in [false, true] {
                assert!(spectrum_wanted(View { page_shows_spectrum: page, root_minimized: min, scanning: true, ..Default::default() }));
            }
        }
    }

    #[test]
    fn start_or_stop_always_repaints() {
        assert!(repaint_for_frame(false, true));
        assert!(!repaint_for_frame(false, false));
        assert!(repaint_for_frame(true, false));
    }
}

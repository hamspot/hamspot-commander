//! Trunking-system clock for the meters page.
//!
//! `dsd-fme` prints a synchronization broadcast as
//! `Date: 2026.10.18 Time: 14:32:05 UTC` and, when the offset is not zero,
//! `Local Time Offset: -5.0 Hours;`. The printed instant is stored as the
//! difference from this machine's clock. Later frames add that skew, so the
//! digits keep moving until the next broadcast. The face shows that local
//! time as hours and minutes plus the NATO zone letter (`02:34Q`).

use eframe::egui;

/// System UTC minus the machine clock, in milliseconds, plus the local offset.
#[derive(Clone, Debug, Default)]
pub struct SystemClock {
    skew_ms: i64,
    offset_hours: f64,
    seen: bool,
}

/// One painted instant. The digits are local time in the trunking zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Readout {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// NATO zone letter. `Q` is UTC−4, `R` is UTC−5, `Z` is UTC.
    pub zone: char,
    /// True when the offset is a half hour. The face draws a trailing `*`.
    pub zone_half: bool,
    /// Local civil date, for example `18 OCT 2026`.
    pub date: String,
}

impl SystemClock {
    /// Apply one decoder line. `now_ms` is the machine clock as Unix milliseconds.
    pub fn note_line(&mut self, line: &str, now_ms: i64) {
        if let Some(hours) = parse_offset(line) {
            if self.seen {
                self.offset_hours = hours;
            }
            return;
        }
        let Some((year, month, day, hour, minute, second)) = parse_stamp(line) else {
            return;
        };
        let unix = days_from_civil(year, month, day) * 86_400
            + hour as i64 * 3_600
            + minute as i64 * 60
            + second as i64;
        self.skew_ms = unix.saturating_mul(1000).saturating_sub(now_ms);
        // A missing offset line means the decoder saw zero.
        self.offset_hours = 0.0;
        self.seen = true;
    }

    pub fn face_at(&self, now_ms: i64) -> Option<Readout> {
        if !self.seen {
            return None;
        }
        let ms = shift_hours(now_ms.saturating_add(self.skew_ms), self.offset_hours);
        let sec = ms.div_euclid(1000);
        let days = sec.div_euclid(86_400);
        let sod = sec.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(days);
        let mon = MONTHS
            .get(month.wrapping_sub(1) as usize)
            .copied()
            .unwrap_or("???");
        let (zone, zone_half) = nato_zone(self.offset_hours);
        Some(Readout {
            hour: (sod / 3_600) as u8,
            minute: ((sod % 3_600) / 60) as u8,
            second: (sod % 60) as u8,
            zone,
            zone_half,
            date: format!("{day:02} {mon} {year}"),
        })
    }

    /// Wait until the displayed minute changes. The digits have no seconds,
    /// and nothing else signals that minute.
    pub fn next_paint_delay(&self, now_ms: i64) -> std::time::Duration {
        let local = shift_hours(now_ms.saturating_add(self.skew_ms), self.offset_hours);
        let into = local.rem_euclid(60_000);
        let remain = (60_000 - into).max(1) as u64;
        std::time::Duration::from_millis(remain)
    }
}

pub fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|dur| dur.as_millis() as i64)
        .unwrap_or(0)
}

fn shift_hours(ms: i64, hours: f64) -> i64 {
    if !hours.is_finite() {
        return ms;
    }
    let delta = (hours * 3_600_000.0).round();
    if !delta.is_finite() {
        return ms;
    }
    ms.saturating_add(delta as i64)
}

/// Military zone letter for a whole-hour offset. `J` is not used, so `K` is
/// UTC+10. A half hour keeps the letter of the whole hour toward zero and
/// sets the half flag (`Q*` is UTC−4.5). Offsets outside ±12 have no letter.
fn nato_zone(hours: f64) -> (char, bool) {
    if !hours.is_finite() {
        return ('Z', false);
    }
    let snapped = (hours * 2.0).round() / 2.0;
    let half = snapped.abs() - snapped.abs().trunc() >= 0.25;
    let whole = snapped.trunc() as i32;
    let letter = match whole {
        0 => 'Z',
        1..=9 => (b'A' + (whole as u8 - 1)) as char,
        10..=12 => (b'A' + whole as u8) as char,
        -12..=-1 => (b'N' + (whole.unsigned_abs() as u8 - 1)) as char,
        _ => 'Z',
    };
    (letter, half && (-12..=12).contains(&whole))
}

fn parse_stamp(line: &str) -> Option<(i32, u32, u32, u32, u32, u32)> {
    let rest = line.split_once("Date:")?.1.trim();
    let (date, rest) = rest.split_once("Time:")?;
    let mut date = date.trim().split('.');
    let year: i32 = date.next()?.trim().parse().ok()?;
    let month: u32 = date.next()?.trim().parse().ok()?;
    let day: u32 = date.next()?.trim().parse().ok()?;
    if date.next().is_some() || !(2000..=2099).contains(&year) || !valid_day(year, month, day) {
        return None;
    }
    let time = rest.trim().strip_suffix("UTC")?.trim();
    let mut time = time.split(':');
    let hour: u32 = time.next()?.trim().parse().ok()?;
    let minute: u32 = time.next()?.trim().parse().ok()?;
    let second: u32 = time.next()?.trim().parse().ok()?;
    if time.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some((year, month, day, hour, minute, second))
}

fn parse_offset(line: &str) -> Option<f64> {
    let rest = line.split_once("Local Time Offset:")?.1.trim();
    let token = rest.split_whitespace().next()?.trim_end_matches(';');
    let hours: f64 = token.parse().ok()?;
    if hours.is_finite() && (-14.0..=14.0).contains(&hours) {
        Some(hours)
    } else {
        None
    }
}

fn valid_day(year: i32, month: u32, day: u32) -> bool {
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(year) => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max).contains(&day)
}

fn leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Days since 1970-01-01. Howard Hinnant's civil algorithm.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year as i64 - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = (year - era * 400) as u64;
    let month_prime = if month > 2 { month - 3 } else { month + 9 } as u64;
    let doy = (153 * month_prime + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

fn civil_from_days(mut z: i64) -> (i32, u32, u32) {
    z += 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day)
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

const ON: egui::Color32 = egui::Color32::from_rgb(255, 58, 42);
const GHOST: egui::Color32 = egui::Color32::from_rgb(48, 18, 16);
const BEZEL: egui::Color32 = egui::Color32::from_rgb(14, 10, 10);

/// Hours and minutes, then the NATO letter, filling one meter cell.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, readout: &Readout) {
    painter.rect_filled(rect, egui::CornerRadius::same(6), BEZEL);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(6),
        egui::Stroke::new(1.0, egui::Color32::from_rgb(72, 32, 28)),
        egui::StrokeKind::Inside,
    );
    let pad = 5.0;
    let title_h = 16.0;
    painter.text(
        rect.min + egui::vec2(pad + 2.0, 3.0),
        egui::Align2::LEFT_TOP,
        "System Clock",
        egui::FontId::proportional(13.0),
        egui::Color32::from_rgb(214, 186, 170),
    );

    let avail_w = (rect.width() - pad * 2.0).max(40.0);
    let n_date = readout.date.chars().count().max(1) as f32;
    let date_dot = (avail_w / (1.45 * (6.0 * n_date - 1.0))).min(1.9);
    let date_h = led_height(date_dot);
    let row_h = (rect.height() - pad * 2.0 - title_h - date_h - 4.0).max(8.0);
    // Four digits, one colon, the zone letter, and a half-hour star.
    let letter_u = 1.7 * 5.0 / 7.0;
    let star_u = if readout.zone_half { 0.7 } else { 0.0 };
    let gaps = if readout.zone_half { 6.0 } else { 5.0 };
    let units = 4.0 + 0.36 + letter_u + star_u + gaps * 0.16;
    let digit_w = (avail_w / units).min(row_h / 1.7).max(8.0);
    let digit_h = (digit_w * 1.7).min(row_h);
    let gap = digit_w * 0.16;
    let colon_w = digit_w * 0.36;
    let dot = digit_h / (1.45 * 7.0);
    let letter_w = led_width(&readout.zone.to_string(), dot);
    let star_dot = dot * 0.62;
    let star_w = if readout.zone_half {
        led_width("*", star_dot)
    } else {
        0.0
    };
    let clock = format!("{:02}:{:02}", readout.hour, readout.minute);
    let mut total = letter_w + gap;
    if readout.zone_half {
        total += gap + star_w;
    }
    for ch in clock.chars() {
        total += if ch == ':' { colon_w } else { digit_w };
        total += gap;
    }
    total -= gap;
    let mut x = rect.center().x - total * 0.5;
    let y = rect.top() + pad + title_h + (row_h - digit_h) * 0.5;
    for ch in clock.chars() {
        let w = if ch == ':' { colon_w } else { digit_w };
        paint_digit(painter, egui::pos2(x, y), w, digit_h, ch);
        x += w + gap;
    }
    paint_led(
        painter,
        egui::pos2(x, y),
        &readout.zone.to_string(),
        dot,
        ON,
    );
    if readout.zone_half {
        let star_h = led_height(star_dot);
        paint_led(
            painter,
            egui::pos2(x + letter_w + gap, y + (digit_h - star_h) * 0.5),
            "*",
            star_dot,
            ON,
        );
    }
    let date_w = led_width(&readout.date, date_dot);
    paint_led(
        painter,
        egui::pos2(rect.center().x - date_w * 0.5, rect.bottom() - pad - date_h),
        &readout.date,
        date_dot,
        ON,
    );
}

fn led_height(dot: f32) -> f32 {
    dot * 1.45 * 7.0
}

fn paint_digit(painter: &egui::Painter, origin: egui::Pos2, w: f32, h: f32, ch: char) {
    if ch == ':' {
        let r = (w * 0.28).max(2.0);
        let cx = origin.x + w * 0.5;
        painter.circle_filled(egui::pos2(cx, origin.y + h * 0.32), r, ON);
        painter.circle_filled(egui::pos2(cx, origin.y + h * 0.68), r, ON);
        return;
    }
    let mask = seven(ch);
    let t = h * 0.11;
    let inset = t * 0.55;
    let mid = origin.y + h * 0.5 - t * 0.5;
    let horiz = |bit: u16, y: f32| {
        let color = if mask & bit != 0 { ON } else { GHOST };
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(origin.x + inset + t * 0.3, y),
                egui::vec2(w - 2.0 * inset - t * 0.6, t),
            ),
            egui::CornerRadius::same(1),
            color,
        );
    };
    let vert = |bit: u16, x: f32, y: f32, height: f32| {
        let color = if mask & bit != 0 { ON } else { GHOST };
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(t, height)),
            egui::CornerRadius::same(1),
            color,
        );
    };
    horiz(1, origin.y);
    horiz(64, mid);
    horiz(8, origin.y + h - t);
    let upper = h * 0.5 - t - inset;
    let lower_y = origin.y + h * 0.5 + inset * 0.35;
    vert(32, origin.x, origin.y + t * 0.7, upper);
    vert(2, origin.x + w - t, origin.y + t * 0.7, upper);
    vert(16, origin.x, lower_y, upper);
    vert(4, origin.x + w - t, lower_y, upper);
}

fn seven(ch: char) -> u16 {
    match ch {
        '0' => 0x3F,
        '1' => 0x06,
        '2' => 0x5B,
        '3' => 0x4F,
        '4' => 0x66,
        '5' => 0x6D,
        '6' => 0x7D,
        '7' => 0x07,
        '8' => 0x7F,
        '9' => 0x6F,
        _ => 0,
    }
}

fn paint_led(painter: &egui::Painter, origin: egui::Pos2, text: &str, dot: f32, color: egui::Color32) {
    let pitch = dot * 1.45;
    let advance = pitch * 6.0;
    for (index, ch) in text.chars().enumerate() {
        let rows = matrix(ch);
        let x0 = origin.x + index as f32 * advance;
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) == 0 {
                    continue;
                }
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x0 + col as f32 * pitch, origin.y + row as f32 * pitch),
                        egui::vec2(dot, dot),
                    ),
                    egui::CornerRadius::same(1),
                    color,
                );
            }
        }
    }
}

fn led_width(text: &str, dot: f32) -> f32 {
    let n = text.chars().count() as f32;
    if n == 0.0 {
        0.0
    } else {
        n * dot * 1.45 * 6.0 - dot * 1.45
    }
}

fn matrix(ch: char) -> [u8; 7] {
    match ch {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b01110, 0b10001, 0b00001, 0b00110, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '*' => [0b00100, 0b10101, 0b01110, 0b00100, 0b01110, 0b10101, 0b00100],
        '+' => [0b00100, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00100],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip_covers_the_epoch_and_a_leap_day() {
        for (year, month, day) in [(1970, 1, 1), (2024, 2, 29), (2026, 10, 18), (2099, 12, 31)] {
            let days = days_from_civil(year, month, day);
            assert_eq!(civil_from_days(days), (year, month, day));
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn a_sync_broadcast_becomes_a_running_clock() {
        let mut clock = SystemClock::default();
        let now = 1_700_000_000_000;
        clock.note_line("  Date: 2026.10.18 Time: 14:32:05 UTC", now);
        clock.note_line("  Local Time Offset: -5.0 Hours;", now);
        let face = clock.face_at(now).expect("clock");
        assert_eq!((face.hour, face.minute, face.second), (9, 32, 5));
        assert_eq!(face.zone, 'R');
        assert!(!face.zone_half);
        assert_eq!(face_text(&face), "09:32R");
        assert_eq!(face.date, "18 OCT 2026");
        let later = clock.face_at(now + 90_000).expect("later");
        assert_eq!((later.hour, later.minute, later.second), (9, 33, 35));
    }

    #[test]
    fn the_clock_rolls_the_date_from_the_machine_skew() {
        let mut clock = SystemClock::default();
        let now = 1_800_000_000_000;
        clock.note_line("Date: 2026.10.18 Time: 23:59:50 UTC", now);
        let next = clock.face_at(now + 20_000).expect("next day");
        assert_eq!((next.hour, next.minute, next.second), (0, 0, 10));
        assert_eq!(next.zone, 'Z');
        assert_eq!(face_text(&next), "00:00Z");
        assert_eq!(next.date, "19 OCT 2026");
        let mut west = SystemClock::default();
        west.note_line("Date: 2026.10.18 Time: 02:00:00 UTC", now);
        west.note_line("Local Time Offset: -5.0 Hours;", now);
        let evening = west.face_at(now).expect("previous local date");
        assert_eq!(face_text(&evening), "21:00R");
        assert_eq!(evening.date, "17 OCT 2026");
    }

    #[test]
    fn a_half_hour_offset_and_unrelated_lines_stay_put() {
        let mut clock = SystemClock::default();
        clock.note_line(" Up to Date (Correct)", 0);
        assert!(clock.face_at(0).is_none());
        clock.note_line("  Local Time Offset: -4.5 Hours;", 10);
        assert!(clock.face_at(10).is_none());
        clock.note_line("Date: 2026.10.18 Time: 06:34:00 UTC", 10);
        clock.note_line("  Local Time Offset: -4.0 Hours;", 10);
        let quebec = clock.face_at(10).unwrap();
        assert_eq!(face_text(&quebec), "02:34Q");
        clock.note_line("Date: 2026.10.18 Time: 06:34:00 UTC", 10);
        clock.note_line("  Local Time Offset: -4.5 Hours;", 10);
        let half = clock.face_at(10).unwrap();
        assert_eq!(face_text(&half), "02:04Q*");
        assert!(half.zone_half);
        clock.note_line("Date: 2026.10.18 Time: 01:02:04 UTC", 1_010);
        assert_eq!(clock.face_at(1_010).unwrap().zone, 'Z');
        assert_eq!(nato_zone(10.0).0, 'K');
        assert_eq!(nato_zone(-12.0).0, 'Y');
    }

    fn face_text(readout: &Readout) -> String {
        let mut text = format!("{:02}:{:02}{}", readout.hour, readout.minute, readout.zone);
        if readout.zone_half {
            text.push('*');
        }
        text
    }
}

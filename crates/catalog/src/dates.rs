//! Calendar helpers for ISO 8601 local times (`2026-09-30T14:05:00`): seconds ↔ civil date,
//! weekday and month names, shifting a time by an offset, and grouping a photo list into runs of
//! the same day / month / year (the date headers of the photo grid).

use serde::{Deserialize, Serialize};

pub use crate::stacks::iso_seconds;
use crate::{Catalog, PhotoId, SortKey};

pub const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
pub const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// Unix seconds → `YYYY-MM-DDTHH:MM:SS` (proleptic Gregorian; H. Hinnant's algorithm).
pub fn civil(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Days since 1970-01-01 → (year, month 1..=12, day 1..=31).
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Weekday name of an ISO date (`None` if it doesn't parse).
pub fn weekday(iso: &str) -> Option<&'static str> {
    let days = iso_seconds(iso.get(..10)?)?.div_euclid(86_400);
    // 1970-01-01 was a Thursday
    Some(WEEKDAYS[(days + 3).rem_euclid(7) as usize])
}

/// Shift an ISO time by `secs`, keeping anything after the seconds (fractions, zone) as it was.
/// A date without a time gets one. `None` if it doesn't parse.
pub fn shift_iso(iso: &str, secs: i64) -> Option<String> {
    let t = iso_seconds(iso)?;
    let tail = iso.trim().get(19..).unwrap_or("");
    Some(format!("{}{tail}", civil(t + secs)))
}

/// Normalize user input to `YYYY-MM-DDTHH:MM:SS` (accepts a space instead of `T`, missing
/// seconds or time). `None` if it isn't a valid date.
pub fn normalize_iso(s: &str) -> Option<String> {
    let t = iso_seconds(s)?;
    let out = civil(t);
    // reject impossible dates that the arithmetic silently rolls over (2026-02-30)
    if out.get(..10) != s.trim().get(..10) {
        return None;
    }
    Some(out)
}

/// What someone types for a capture time, read against the photo's `current` time (ISO), →
/// `YYYY-MM-DDTHH:MM:SS`:
/// - a time on the same day: `3:34 pm`, `3:34pm`, `15:34`, `15:34:20`, `3pm`, `3 p.m.`;
/// - a date, with or without a time: `2026-06-14 15:34:12`, `2026-06-14T3:34 pm`, `2026-06-14`
///   (the date alone keeps the time of day);
/// - a step: `+1h`, `-30m`, `+1d`, `+1h30m`, `-1d 2h`, `+90s`, `+2w`, `+1.5h` (units `w` `d` `h`
///   `m`/`min` `s`, also spelled out).
///
/// Seconds that aren't typed keep `current`'s, so "3:34 pm" on a photo at 2:34:12 PM is exactly
/// one hour later. `None` when the input or `current` doesn't parse, or the date doesn't exist.
pub fn parse_time_input(input: &str, current: &str) -> Option<String> {
    let cur = iso_seconds(current)?;
    let cur_iso = civil(cur);
    let s = input.trim().to_ascii_lowercase().replace('\u{2212}', "-");
    if let Some(rest) = s.strip_prefix('+').map(|r| (1i64, r)).or_else(|| s.strip_prefix('-').map(|r| (-1i64, r))) {
        return parse_step(rest.1).map(|d| civil(cur + rest.0 * d));
    }
    // a leading date
    let (date, rest, had_date) = match s.get(..10).filter(|d| d.as_bytes().get(4) == Some(&b'-') && d.as_bytes().get(7) == Some(&b'-')) {
        Some(d) => {
            let d = normalize_iso(d)?;
            (d.get(..10)?.to_string(), s.get(10..)?.trim_start_matches(['t', ' ']).trim(), true)
        }
        None => (cur_iso.get(..10)?.to_string(), s.as_str(), false),
    };
    let cur_time = cur_iso.get(11..19)?;
    if rest.is_empty() {
        return had_date.then(|| format!("{date}T{cur_time}"));
    }
    let (h, m, sec) = parse_clock(rest, cur_time.get(6..8)?.parse().ok()?)?;
    normalize_iso(&format!("{date}T{h:02}:{m:02}:{sec:02}"))
}

/// `1h30m`, `1 d 2 h`, `90s`, `1.5h` → seconds.
fn parse_step(s: &str) -> Option<i64> {
    let s = s.trim();
    let mut total = 0f64;
    let mut any = false;
    let mut rest = s;
    while !rest.trim_start().is_empty() {
        rest = rest.trim_start();
        let n_end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
        let n: f64 = rest.get(..n_end)?.parse().ok()?;
        rest = rest.get(n_end..)?.trim_start();
        let u_end = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
        let unit = match rest.get(..u_end)? {
            "w" | "wk" | "wks" | "week" | "weeks" => 604_800.0,
            "d" | "day" | "days" => 86_400.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            _ => return None,
        };
        rest = rest.get(u_end..)?;
        total += n * unit;
        any = true;
    }
    // ±100 years is plenty; beyond that the input is a typo
    (any && total.is_finite() && total.abs() < 3.2e9).then(|| total.round() as i64)
}

/// `3:34 pm`, `15:34`, `15:34:20`, `3pm`, `3 a.m.` → (hour, minute, second); seconds not typed
/// are `keep_secs`.
fn parse_clock(s: &str, keep_secs: u32) -> Option<(u32, u32, u32)> {
    let s = s.trim().replace('.', "");
    let (body, pm) = if let Some(b) = s.strip_suffix("pm").or_else(|| s.strip_suffix('p')) {
        (b.trim(), Some(true))
    } else if let Some(b) = s.strip_suffix("am").or_else(|| s.strip_suffix('a')) {
        (b.trim(), Some(false))
    } else {
        (s.as_str(), None)
    };
    let parts: Vec<&str> = body.split(':').map(str::trim).collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || p.len() > 2 || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let num = |i: usize| parts.get(i).and_then(|p| p.parse::<u32>().ok());
    let (mut h, m, sec) = (num(0)?, num(1).unwrap_or(0), if parts.len() == 3 { num(2)? } else { keep_secs });
    // a bare number without am/pm isn't a time ("15" could be anything)
    if parts.len() == 1 && pm.is_none() {
        return None;
    }
    match pm {
        Some(pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            h = match (pm, h) {
                (false, 12) => 0,
                (true, 12) => 12,
                (true, h) => h + 12,
                (false, h) => h,
            };
        }
        None if h > 23 => return None,
        None => {}
    }
    (m < 60 && sec < 60).then_some((h, m, sec))
}

/// "Sat, Jun 14 2026 · 2:34:12 PM" (the date alone when there's no time; the input unchanged
/// when it doesn't parse).
pub fn display_long(iso: &str) -> String {
    let Some(secs) = iso_seconds(iso) else { return iso.to_string() };
    let c = civil(secs);
    let num = |r: std::ops::Range<usize>| c.get(r).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let (y, mo, d, h, mi, s) = (num(0..4), num(5..7), num(8..10), num(11..13), num(14..16), num(17..19));
    let wd = weekday(&c).and_then(|w| w.get(..3)).unwrap_or("");
    let mon = MONTHS.get((mo as usize).wrapping_sub(1)).and_then(|m| m.get(..3)).unwrap_or("");
    let date = format!("{wd}, {mon} {d} {y}");
    if iso.trim().len() < 16 {
        return date;
    }
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{date} · {h12}:{mi:02}:{s:02} {ampm}")
}

/// A shift in seconds for people: `+1 h 0 m`, `−2 d 3 h 0 m`, `+56 m 15 s`, "no change".
pub fn display_delta(secs: i64) -> String {
    if secs == 0 {
        return "no change".into();
    }
    let sign = if secs < 0 { "\u{2212}" } else { "+" };
    let a = secs.unsigned_abs();
    let (d, h, m, s) = (a / 86_400, a % 86_400 / 3600, a % 3600 / 60, a % 60);
    let mut out = String::from(sign);
    if d > 0 {
        out.push_str(&format!("{d} d "));
    }
    if d > 0 || h > 0 {
        out.push_str(&format!("{h} h "));
    }
    out.push_str(&format!("{m} m"));
    if s > 0 {
        out.push_str(&format!(" {s} s"));
    }
    out
}

/// Granularity of the grid's date headers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupBy {
    /// Day headers, or months/years when the thumbnails are small (decided by the UI).
    #[default]
    Auto,
    None,
    Day,
    Month,
    Year,
}

impl GroupBy {
    pub fn parse(s: &str) -> Option<GroupBy> {
        Some(match s.to_ascii_lowercase().as_str() {
            "auto" => GroupBy::Auto,
            "none" | "off" => GroupBy::None,
            "day" => GroupBy::Day,
            "month" => GroupBy::Month,
            "year" => GroupBy::Year,
            _ => return None,
        })
    }
    /// Length of the ISO prefix that identifies a group.
    fn prefix(self) -> usize {
        match self {
            GroupBy::Year => 4,
            GroupBy::Month => 7,
            _ => 10,
        }
    }
}

/// A run of consecutive photos with the same date (one header in the grid).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateRun {
    /// `2026-09-30` / `2026-09` / `2026`, or empty for photos without a usable date.
    pub key: String,
    /// "Wednesday, 30 September 2026" / "September 2026" / "2026" / "Unknown Date".
    pub label: String,
    /// Index of the first photo in the list.
    pub start: usize,
    pub count: usize,
}

/// A capture time for display: `2022-03-30T22:11:11` → "March 30, 2022 at 10:11:11 PM" (the
/// date alone when there is no time; the input unchanged when it doesn't parse).
pub fn display_time(iso: &str) -> String {
    let num = |r: std::ops::Range<usize>| iso.get(r).and_then(|v| v.parse::<u32>().ok());
    let (Some(y), Some(mo), Some(d)) = (num(0..4), num(5..7), num(8..10)) else { return iso.to_string() };
    let Some(month) = MONTHS.get((mo as usize).wrapping_sub(1)) else { return iso.to_string() };
    let date = format!("{month} {d}, {y}");
    match (num(11..13), num(14..16), num(17..19)) {
        (Some(h), Some(mi), s) => {
            let (h12, ampm) = match h {
                0 => (12, "AM"),
                1..=11 => (h, "AM"),
                12 => (12, "PM"),
                _ => (h - 12, "PM"),
            };
            format!("{date} at {h12}:{mi:02}:{:02} {ampm}", s.unwrap_or(0))
        }
        _ => date,
    }
}

/// Header text for a group key.
pub fn group_label(key: &str) -> String {
    let month = |k: &str| k.get(5..7).and_then(|m| m.parse::<usize>().ok()).and_then(|m| MONTHS.get(m.wrapping_sub(1)));
    // `get` rather than `[..]`: a non-ASCII key must not split a character
    let year = key.get(..4).unwrap_or(key);
    match key.len() {
        10 => match (weekday(key), month(key), key.get(8..10).and_then(|d| d.parse::<u32>().ok())) {
            (Some(w), Some(m), Some(d)) => format!("{w}, {d} {m} {year}"),
            _ => key.to_string(),
        },
        7 => month(key).map(|m| format!("{m} {year}")).unwrap_or_else(|| key.to_string()),
        4 => key.to_string(),
        _ => "Unknown Date".into(),
    }
}

impl Catalog {
    /// Split `ids` (the grid order) into runs of the same day / month / year for `key`'s date.
    /// Members of a stack share the group of the stack's first photo in the list, so a stack is
    /// never split by a header. Empty for sort keys without a date and for [`GroupBy::None`]
    /// ([`GroupBy::Auto`] means days here).
    pub fn date_runs(&self, ids: &[PhotoId], key: SortKey, by: GroupBy) -> Vec<DateRun> {
        if by == GroupBy::None || ids.is_empty() {
            return Vec::new();
        }
        let n = by.prefix();
        let date_of = |id: &PhotoId| -> String {
            let Some(p) = self.photo(*id) else { return String::new() };
            let d = match key {
                SortKey::CaptureDate => Some(p.date()),
                SortKey::ImportDate => Some(p.imported.as_str()),
                SortKey::EditDate => p.edited.as_deref(),
                _ => None,
            };
            d.filter(|d| iso_seconds(d.get(..10).unwrap_or("")).is_some()).and_then(|d| d.get(..n)).unwrap_or("").to_string()
        };
        if !matches!(key, SortKey::CaptureDate | SortKey::ImportDate | SortKey::EditDate) {
            return Vec::new();
        }
        let stacks = self.stack_index();
        let mut runs: Vec<DateRun> = Vec::new();
        let mut stack_key: std::collections::HashMap<crate::StackId, String> = Default::default();
        for (i, id) in ids.iter().enumerate() {
            let k = match stacks.get(id) {
                Some((sid, _)) => stack_key.entry(*sid).or_insert_with(|| date_of(id)).clone(),
                None => date_of(id),
            };
            match runs.last_mut() {
                Some(r) if r.key == k => r.count += 1,
                _ => runs.push(DateRun { label: group_label(&k), key: k, start: i, count: 1 }),
            }
        }
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Op, Photo, Source};

    #[test]
    fn calendar() {
        assert_eq!(civil(0), "1970-01-01T00:00:00");
        assert_eq!(civil(951_782_400), "2000-02-29T00:00:00");
        assert_eq!(weekday("2026-09-30"), Some("Wednesday"));
        assert_eq!(weekday("2026-10-01T09:00:00"), Some("Thursday"));
        assert_eq!(weekday("2000-02-29"), Some("Tuesday"));
        assert_eq!(group_label("2026-09-29"), "Tuesday, 29 September 2026");
        assert_eq!(group_label("2026-02"), "February 2026");
        assert_eq!(group_label("2026"), "2026");
        assert_eq!(group_label(""), "Unknown Date");
        // non-ASCII keys of a date's length used to be sliced mid-character
        assert_eq!(group_label("2024-01é0"), "2024-01é0");
        assert_eq!(group_label("202é01"), "January 202é01");
        assert_eq!(shift_iso("2026-12-31T23:30:00", 3600).as_deref(), Some("2027-01-01T00:30:00"));
        assert_eq!(shift_iso("2026-03-01T00:00:00.25+02:00", -1).as_deref(), Some("2026-02-28T23:59:59.25+02:00"));
        assert_eq!(shift_iso("nope", 5), None);
        assert_eq!(normalize_iso("2026-04-01 10:05").as_deref(), Some("2026-04-01T10:05:00"));
        assert_eq!(normalize_iso("2026-02-30"), None);
    }

    /// Typed capture times: clock times on the photo's day (keeping its seconds), dates, steps.
    #[test]
    fn typed_capture_times() {
        let cur = "2026-06-13T14:34:12";
        let p = |s: &str| parse_time_input(s, cur);
        assert_eq!(p("3:34 pm").as_deref(), Some("2026-06-13T15:34:12"));
        assert_eq!(p("3:34PM").as_deref(), Some("2026-06-13T15:34:12"));
        assert_eq!(p("3 p.m.").as_deref(), Some("2026-06-13T15:00:12"));
        assert_eq!(p("15:34").as_deref(), Some("2026-06-13T15:34:12"));
        assert_eq!(p("15:34:00").as_deref(), Some("2026-06-13T15:34:00"));
        assert_eq!(p("12:05 am").as_deref(), Some("2026-06-13T00:05:12"));
        assert_eq!(p("12:05 pm").as_deref(), Some("2026-06-13T12:05:12"));
        assert_eq!(p("2026-06-14 3:34 pm").as_deref(), Some("2026-06-14T15:34:12"));
        assert_eq!(p("2026-06-14T15:34:20").as_deref(), Some("2026-06-14T15:34:20"));
        assert_eq!(p("2026-06-15").as_deref(), Some("2026-06-15T14:34:12"));
        assert_eq!(p("+1h").as_deref(), Some("2026-06-13T15:34:12"));
        assert_eq!(p("-30m").as_deref(), Some("2026-06-13T14:04:12"));
        assert_eq!(p("\u{2212}30 min").as_deref(), Some("2026-06-13T14:04:12"));
        assert_eq!(p("+1d").as_deref(), Some("2026-06-14T14:34:12"));
        assert_eq!(p("+1h30m").as_deref(), Some("2026-06-13T16:04:12"));
        assert_eq!(p("-1d 2h").as_deref(), Some("2026-06-12T12:34:12"));
        assert_eq!(p("+1.5h").as_deref(), Some("2026-06-13T16:04:12"));
        assert_eq!(p("+2 weeks").as_deref(), Some("2026-06-27T14:34:12"));
        for bad in ["", "yesterday", "15", "25:00", "13:00 pm", "0 am", "3:60 pm", "+", "+1x", "+h", "2026-02-30 10:00", "1:2:3:4", "+99999999d", "é"]
        {
            assert_eq!(p(bad), None, "{bad}");
        }
        assert_eq!(parse_time_input("3:34 pm", "garbage"), None);
        assert_eq!(display_long("2026-06-13T14:34:12"), "Sat, Jun 13 2026 · 2:34:12 PM");
        assert_eq!(display_long("2026-06-13"), "Sat, Jun 13 2026");
        assert_eq!(display_long("nope"), "nope");
        assert_eq!(display_delta(3600), "+1 h 0 m");
        assert_eq!(display_delta(-(86_400 + 3 * 3600)), "\u{2212}1 d 3 h 0 m");
        assert_eq!(display_delta(90), "+1 m 30 s");
        assert_eq!(display_delta(-2 * 86_400), "\u{2212}2 d 0 h 0 m");
        assert_eq!(display_delta(0), "no change");
    }

    #[test]
    fn runs_by_day_month_year_keep_stacks_together() {
        let mut c = Catalog::new();
        let times = ["2026-09-30T18:00:00", "2026-09-30T09:00:00", "2026-09-29T23:59:00", "2026-08-01T10:00:00", "2025-12-31T10:00:00", ""];
        let mut ids = Vec::new();
        for t in times {
            let id = c.alloc_photo_id();
            let mut p = Photo::new(id, Source::Demo { scene: 1 }, "a.jpg", "JPEG", 3, 2, "2026-10-01T00:00:00");
            p.captured = (!t.is_empty()).then(|| t.to_string());
            c.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
            ids.push(id);
        }
        let shape = |r: &[DateRun]| r.iter().map(|r| (r.key.clone(), r.start, r.count)).collect::<Vec<_>>();
        let day = c.date_runs(&ids, SortKey::CaptureDate, GroupBy::Day);
        // the undated photo falls back to its import date
        assert_eq!(
            shape(&day),
            vec![
                ("2026-09-30".into(), 0, 2),
                ("2026-09-29".into(), 2, 1),
                ("2026-08-01".into(), 3, 1),
                ("2025-12-31".into(), 4, 1),
                ("2026-10-01".into(), 5, 1)
            ]
        );
        assert_eq!(day[0].label, "Wednesday, 30 September 2026");
        assert_eq!(c.date_runs(&ids, SortKey::CaptureDate, GroupBy::Auto), day);
        assert_eq!(
            shape(&c.date_runs(&ids[..5], SortKey::CaptureDate, GroupBy::Month)),
            vec![("2026-09".into(), 0, 3), ("2026-08".into(), 3, 1), ("2025-12".into(), 4, 1)]
        );
        assert_eq!(shape(&c.date_runs(&ids[..5], SortKey::CaptureDate, GroupBy::Year)), vec![("2026".into(), 0, 4), ("2025".into(), 4, 1)]);
        assert!(c.date_runs(&ids, SortKey::FileName, GroupBy::Day).is_empty());
        assert!(c.date_runs(&ids, SortKey::CaptureDate, GroupBy::None).is_empty());
        // edit date: unedited photos have no date
        let r = c.date_runs(&ids, SortKey::EditDate, GroupBy::Day);
        assert_eq!(shape(&r), vec![(String::new(), 0, 6)]);
        assert_eq!(r[0].label, "Unknown Date");
        // a stack spanning midnight stays in its first member's group
        let op = c.group_ops(ids[1], &[ids[1], ids[2]], false).unwrap();
        c.apply(op).unwrap();
        let arranged = c.arrange_stacks(&ids[..5]);
        assert_eq!(
            shape(&c.date_runs(&arranged, SortKey::CaptureDate, GroupBy::Day))[..2],
            [("2026-09-30".into(), 0, 3), ("2026-08-01".into(), 3, 1)]
        );
    }
}

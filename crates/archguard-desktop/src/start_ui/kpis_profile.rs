//! The health ring of the score card and the "Architecture profile and
//! history" of the glance: `renderDashHealth` with its sparkline
//! (viewer_dash_health.js) and the zone counts of `renderDashZones`
//! (viewer_dash_zones.js). Geometry is computed here; Slint draws it.

use archguard_engine::core::{HealthLevel, ReviewKpis};

use crate::format::Locale;
use crate::model_sync::sync_rows;
use crate::ui::{StartGlance, StartKpiPart};

/// One measured health score of a snapshot (`reviewHistory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub timestamp: u64,
    pub score: f64,
}

/// `+1.5` / `-2.0`: a delta with its sign.
pub fn signed(l: Locale, v: f64, places: usize) -> String {
    let n = l.dec(v, places);
    if v >= 0.0 { format!("+{n}") } else { n }
}

pub fn level(level: Option<HealthLevel>) -> i32 {
    match level {
        Some(HealthLevel::Green) => 1,
        Some(HealthLevel::Yellow) => 2,
        Some(HealthLevel::Red) => 3,
        None => 0,
    }
}

/// Fill the health head, its parts, the zone counts and the sparkline.
pub fn fill(gl: &mut StartGlance, old: &StartGlance, k: &ReviewKpis, history: &[Point], l: Locale) {
    if let Some(h) = &k.health {
        let score = h.score.filter(|s| s.is_finite());
        gl.score = score.map_or(String::new(), |s| l.dec(s, 2)).into();
        gl.ring = score.map_or(String::new(), ring).into();
        gl.level = level(h.level);
        gl.trend = h.trend.map_or(String::new(), |t| signed(l, t, 1)).into();
        gl.rising = h.trend.is_some_and(|t| t >= 0.0);
        // `parts` is a BTreeMap: sorted by name, as the web sorts them.
        let parts = h.parts.iter().map(|(name, v)| StartKpiPart {
            name: name.as_str().into(),
            value: l.dec(*v, 2).into(),
        });
        gl.parts = sync_rows(&old.parts, parts);
    }
    if let Some(z) = &k.zones {
        gl.zones_known = true;
        let n = |v: usize| i32::try_from(v).unwrap_or(i32::MAX);
        gl.pain = n(z.zone_of_pain);
        gl.useless = n(z.zone_of_uselessness);
        gl.main = n(z.main_sequence);
        gl.unmeasured = n(z.unmeasured);
    }
    if let Some((path, from, to)) = spark(history) {
        gl.spark = path.into();
        gl.spark_from = from.into();
        gl.spark_to = to.into();
    }
}

/// The score arc in a 100 × 100 box, clockwise from the top (the web's
/// conic `--score-angle`); "" for no visible arc.
pub fn ring(score: f64) -> String {
    const R: f64 = 42.0;
    let share = score.clamp(0.0, 100.0) / 100.0;
    if share <= 0.0 {
        return String::new();
    }
    if share >= 1.0 {
        return format!("M 50 8 A {R} {R} 0 1 1 50 92 A {R} {R} 0 1 1 50 8");
    }
    let a = share * std::f64::consts::TAU;
    let (x, y) = (50.0 + R * a.sin(), 50.0 - R * a.cos());
    let large = u8::from(share > 0.5);
    format!("M 50 8 A {R} {R} 0 {large} 1 {x:.2} {y:.2}")
}

/// The score polyline in a 250 × 48 box with the first and last date;
/// `None` below two measured snapshots. An equal series is a centred flat
/// line, never a division by zero.
pub fn spark(points: &[Point]) -> Option<(String, String, String)> {
    let (first, last) = (points.first()?, points.last()?);
    if points.len() < 2 {
        return None;
    }
    let (min, max) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
        (lo.min(p.score), hi.max(p.score))
    });
    let step = 250.0 / (points.len() - 1) as f64;
    let y = |s: f64| {
        if max > min {
            (1.0 - (s - min) / (max - min)) * 48.0
        } else {
            24.0
        }
    };
    let path = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let cmd = if i == 0 { 'M' } else { 'L' };
            format!("{cmd} {:.1} {:.1}", i as f64 * step, y(p.score))
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some((path, date(first.timestamp), date(last.timestamp)))
}

/// Unix seconds as the UTC date `YYYY-MM-DD` (days to civil date).
pub fn date(timestamp: u64) -> String {
    let days = i64::try_from(timestamp / 86_400).unwrap_or(0) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_utc_calendar_days() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_794_009_600), "2026-11-07");
    }

    /// The arc starts at the top; a half score ends at the bottom, a full
    /// one closes the circle, none draws nothing.
    #[test]
    fn the_ring_follows_the_score() {
        assert_eq!(ring(0.0), "");
        assert_eq!(ring(50.0), "M 50 8 A 42 42 0 0 1 50.00 92.00");
        assert_eq!(ring(75.0), "M 50 8 A 42 42 0 1 1 8.00 50.00");
        assert!(ring(140.0).ends_with("50 8"));
    }

    #[test]
    fn the_sparkline_spans_the_measured_scores() {
        let p = |timestamp: u64, score: f64| Point { timestamp, score };
        assert_eq!(spark(&[p(0, 50.0)]), None);
        let (path, from, to) = spark(&[p(0, 50.0), p(86_400, 70.0), p(172_800, 60.0)]).unwrap();
        assert_eq!(path, "M 0.0 48.0 L 125.0 0.0 L 250.0 24.0");
        assert_eq!((from.as_str(), to.as_str()), ("1970-01-01", "1970-01-03"));
        let flat = spark(&[p(0, 5.0), p(1, 5.0)]).unwrap().0;
        assert_eq!(flat, "M 0.0 24.0 L 250.0 24.0");
    }
}

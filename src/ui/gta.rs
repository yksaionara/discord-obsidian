//! The Vice City easter egg: a countdown to GTA VI, which Search settings
//! shows for "vice city".
//!
//! Rockstar releases it "midnight local time on November 19, 2026, in all
//! regions". Store listings put the US stores on one hour, midnight Eastern,
//! so the rest of the United States plays from the evening before; a clock in
//! a US time zone counts to that instead.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use obsidian_ui::GtaClock;

/// Midnight Eastern on November 19, 2026 (EST, UTC-5).
fn eastern_midnight() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 19, 5, 0, 0).unwrap()
}

/// Whether an offset from UTC is one of the US mainland's, Alaska's or
/// Hawaii's in November, from Eastern (-5) to Hawaii (-10).
fn american(offset: &FixedOffset) -> bool {
    (-10 * 3600..=-5 * 3600).contains(&offset.local_minus_utc())
}

/// When it unlocks for a clock at this offset, and how to say so.
pub fn unlock(offset: &FixedOffset) -> (DateTime<Utc>, &'static str) {
    if american(offset) {
        return (
            eastern_midnight(),
            "US stores unlock together at midnight Eastern on November 19, so the rest of the country plays the evening before.",
        );
    }
    let midnight = NaiveDate::from_ymd_opt(2026, 11, 19)
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .expect("a real date");
    let at = offset
        .from_local_datetime(&midnight)
        .single()
        .map(|local| local.with_timezone(&Utc))
        .unwrap_or_else(eastern_midnight);
    (at, "Rockstar unlocks it at midnight local time on November 19, 2026.")
}

/// The countdown as it stands at `now` for a clock at `offset`.
pub fn clock(now: DateTime<Utc>, offset: &FixedOffset) -> GtaClock {
    let (at, note) = unlock(offset);
    let left = (at - now).num_seconds();
    if left <= 0 {
        return GtaClock {
            note: "It's out. Rockstar released it on November 19, 2026.".into(),
            out: true,
            ..Default::default()
        };
    }
    GtaClock {
        days: format!("{}", left / 86_400).into(),
        hours: format!("{:02}", left / 3600 % 24).into(),
        minutes: format!("{:02}", left / 60 % 60).into(),
        seconds: format!("{:02}", left % 60).into(),
        note: note.into(),
        out: false,
    }
}

/// The countdown now, on this computer's clock.
pub fn now() -> GtaClock {
    clock(Utc::now(), chrono::Local::now().offset())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hours(hours: i32) -> FixedOffset {
        FixedOffset::east_opt(hours * 3600).unwrap()
    }

    #[test]
    fn the_west_coast_unlocks_with_new_york() {
        let (pacific, _) = unlock(&hours(-8));
        let (eastern, _) = unlock(&hours(-5));
        assert_eq!(pacific, eastern);
        assert_eq!(pacific, Utc.with_ymd_and_hms(2026, 11, 19, 5, 0, 0).unwrap());
    }

    #[test]
    fn elsewhere_it_unlocks_at_local_midnight() {
        let (london, _) = unlock(&hours(0));
        assert_eq!(london, Utc.with_ymd_and_hms(2026, 11, 19, 0, 0, 0).unwrap());
        let (riyadh, _) = unlock(&hours(3));
        assert_eq!(riyadh, Utc.with_ymd_and_hms(2026, 11, 18, 21, 0, 0).unwrap());
    }

    #[test]
    fn counts_days_hours_minutes_and_seconds() {
        let now = Utc.with_ymd_and_hms(2026, 11, 14, 2, 59, 50).unwrap();
        let left = clock(now, &hours(3));
        assert_eq!((left.days.as_str(), left.hours.as_str()), ("4", "18"));
        assert_eq!((left.minutes.as_str(), left.seconds.as_str()), ("00", "10"));
        assert!(!left.out);
    }

    #[test]
    fn says_when_its_out() {
        let now = Utc.with_ymd_and_hms(2026, 11, 20, 0, 0, 0).unwrap();
        assert!(clock(now, &hours(0)).out);
    }
}

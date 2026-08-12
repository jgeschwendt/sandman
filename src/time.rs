//! UTC timestamps at seconds precision, hand-rolled — the crate takes no
//! dependencies.
//!
//! Two renderings are used by the bank format: ISO-8601 Z for `created:` and
//! `updated:`, and the compact `YYYYMMDDTHHMMSS` stamp for `_archive/` names.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

/// Seconds in a day.
const SECONDS_PER_DAY: i64 = 86_400;
/// Days from 0000-03-01 to 1970-01-01 — the shift into Hinnant's era algebra.
const DAYS_TO_CIVIL_EPOCH: i64 = 719_468;
/// Days in a 400-year era.
const DAYS_PER_ERA: i64 = 146_097;

/// An instant, UTC, truncated to the second.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Timestamp {
    unix_seconds: i64,
}

impl Timestamp {
    /// The current instant.
    pub fn now() -> Result<Self> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Clock)?;
        let seconds = i64::try_from(elapsed.as_secs()).map_err(|_| Error::Clock)?;
        Ok(Self::from_unix_seconds(seconds))
    }

    /// An instant from seconds since the Unix epoch.
    #[must_use]
    pub const fn from_unix_seconds(unix_seconds: i64) -> Self {
        Self { unix_seconds }
    }

    /// Seconds since the Unix epoch.
    #[must_use]
    pub const fn unix_seconds(self) -> i64 {
        self.unix_seconds
    }

    /// `YYYY-MM-DDTHH:MM:SSZ` — the frontmatter form.
    #[must_use]
    pub fn iso8601(self) -> String {
        let (year, month, day, hour, minute, second) = self.parts();
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    }

    /// `YYYYMMDDTHHMMSS` — the `_archive/` filename prefix.
    #[must_use]
    pub fn stamp(self) -> String {
        let (year, month, day, hour, minute, second) = self.parts();
        format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}")
    }

    /// Read `YYYY-MM-DDTHH:MM:SS`, optionally followed by a fractional part
    /// and/or a `Z`, as UTC.
    ///
    /// Deliberately narrow: a numeric offset (`+02:00`) is rejected rather
    /// than silently read as UTC, and a caller that cannot date a record drops
    /// it — the rule `memory-recall.js` applies to `queued_at`.
    #[must_use]
    pub fn parse_iso8601(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() < 19 {
            return None;
        }
        let digits = |range: std::ops::Range<usize>| -> Option<i64> {
            let slice = text.get(range)?;
            if slice.bytes().all(|byte| byte.is_ascii_digit()) {
                slice.parse().ok()
            } else {
                None
            }
        };
        if bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' || bytes[16] != b':' {
            return None;
        }
        if bytes[10] != b'T' && bytes[10] != b' ' {
            return None;
        }
        match &bytes[19..] {
            [] | [b'Z'] => {}
            [b'.', rest @ ..] => {
                let end = rest.iter().position(|byte| !byte.is_ascii_digit());
                match end.map(|index| &rest[index..]) {
                    None | Some([b'Z']) => {}
                    _ => return None,
                }
            }
            _ => return None,
        }

        let year = digits(0..4)?;
        let month = digits(5..7)?;
        let day = digits(8..10)?;
        let hour = digits(11..13)?;
        let minute = digits(14..16)?;
        let second = digits(17..19)?;
        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || minute > 59
            || second > 60
        {
            return None;
        }
        let days = days_from_civil(year, month, day);
        Some(Self::from_unix_seconds(
            days * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second,
        ))
    }

    /// Broken-down UTC: year, month, day, hour, minute, second.
    #[must_use]
    pub fn parts(self) -> (i64, i64, i64, i64, i64, i64) {
        let days = self.unix_seconds.div_euclid(SECONDS_PER_DAY);
        let second_of_day = self.unix_seconds.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        (
            year,
            month,
            day,
            second_of_day / 3600,
            (second_of_day / 60) % 60,
            second_of_day % 60,
        )
    }
}

/// Days since 1970-01-01 → civil `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`, exact for the whole proleptic Gregorian
/// range this crate can represent.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + DAYS_TO_CIVIL_EPOCH;
    let era = (if shifted >= 0 {
        shifted
    } else {
        shifted - (DAYS_PER_ERA - 1)
    }) / DAYS_PER_ERA;
    let day_of_era = shifted - era * DAYS_PER_ERA;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Civil `(year, month, day)` → days since 1970-01-01.
///
/// Hinnant's `days_from_civil` — the exact inverse of [`civil_from_days`].
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let year_of_era = year - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * DAYS_PER_ERA + day_of_era - DAYS_TO_CIVIL_EPOCH
}

#[cfg(test)]
mod tests {
    use super::Timestamp;

    /// Reference values from `datetime.datetime.fromtimestamp(s, timezone.utc)`.
    const KNOWN: [(i64, &str); 10] = [
        (0, "1970-01-01T00:00:00Z"),
        (1_000_000_000, "2001-09-09T01:46:40Z"),
        (1_234_567_890, "2009-02-13T23:31:30Z"),
        (951_782_399, "2000-02-28T23:59:59Z"),
        (951_782_400, "2000-02-29T00:00:00Z"),
        (1_748_736_000, "2025-06-01T00:00:00Z"),
        (1_767_225_599, "2025-12-31T23:59:59Z"),
        (4_102_444_800, "2100-01-01T00:00:00Z"),
        (-1, "1969-12-31T23:59:59Z"),
        (-86_400, "1969-12-31T00:00:00Z"),
    ];

    #[test]
    fn iso8601_matches_known_epochs() {
        for (seconds, expected) in KNOWN {
            assert_eq!(
                Timestamp::from_unix_seconds(seconds).iso8601(),
                expected,
                "for {seconds}"
            );
        }
    }

    #[test]
    fn stamp_is_the_compact_form_of_iso8601() {
        for (seconds, expected) in KNOWN {
            let compact: String = expected
                .chars()
                .filter(|ch| ch.is_ascii_digit() || *ch == 'T')
                .collect();
            assert_eq!(
                Timestamp::from_unix_seconds(seconds).stamp(),
                compact,
                "for {seconds}"
            );
        }
        // Live archive name: 20260806T121137_project_sandman_memory_extraction_plan.md
        assert_eq!(
            Timestamp::from_unix_seconds(1_786_018_297).stamp(),
            "20260806T121137"
        );
    }

    #[test]
    fn every_day_of_a_leap_cycle_advances_by_one() {
        // Walk 1996-01-01 .. 2103-12-28 a day at a time: the civil conversion
        // must be strictly monotone and never repeat or skip a date.
        let mut previous = String::new();
        let mut day = 820_454_400_i64;
        let end = 4_228_243_200_i64;
        let mut count = 0_u32;
        while day < end {
            let iso = Timestamp::from_unix_seconds(day).iso8601();
            assert!(iso > previous, "{iso} did not advance past {previous}");
            previous = iso;
            day += 86_400;
            count += 1;
        }
        assert_eq!(count, 39_442);
    }

    #[test]
    fn parsing_inverts_rendering_for_every_known_epoch() {
        for (seconds, iso) in KNOWN {
            assert_eq!(
                Timestamp::parse_iso8601(iso).map(Timestamp::unix_seconds),
                Some(seconds),
                "for {iso}"
            );
        }
        // A whole leap cycle, one day at a time.
        let mut day = 820_454_400_i64;
        while day < 4_228_243_200_i64 {
            let stamp = Timestamp::from_unix_seconds(day);
            assert_eq!(
                Timestamp::parse_iso8601(&stamp.iso8601()),
                Some(stamp),
                "for {}",
                stamp.iso8601()
            );
            day += 86_400;
        }
    }

    #[test]
    fn parsing_accepts_the_tolerated_shapes_and_refuses_the_rest() {
        let epoch = Timestamp::from_unix_seconds(1_754_000_000);
        let iso = epoch.iso8601();
        let bare = iso.trim_end_matches('Z').to_owned();
        assert_eq!(Timestamp::parse_iso8601(&bare), Some(epoch));
        assert_eq!(
            Timestamp::parse_iso8601(&format!("{bare}.123Z")),
            Some(epoch)
        );
        assert_eq!(
            Timestamp::parse_iso8601(&bare.replace('T', " ")),
            Some(epoch)
        );

        for refused in [
            "",
            "2026-08-11",
            "2026-08-11T12:00",
            "2026-08-11T12:00:00+02:00",
            "2026-13-11T12:00:00Z",
            "2026-08-32T12:00:00Z",
            "2026-08-11T24:00:00Z",
            "2026-08-11T12:60:00Z",
            "20260811T120000Z",
            "not a time at all",
        ] {
            assert_eq!(Timestamp::parse_iso8601(refused), None, "for {refused:?}");
        }
    }

    #[test]
    fn now_is_after_the_crate_was_written() {
        let now = Timestamp::now().expect("clock after the epoch");
        assert!(now.unix_seconds() > 1_754_000_000);
    }
}

//! Implements creation, display, efficient storage and creation from system time for timestamps.

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use std::{cmp::Ordering, fmt, ops::Sub, str::FromStr};

/// The number of nanoseconds per second.
const NANOS_PER_SEC: i128 = 1_000_000_000;

/// Represents a timestamp.
#[derive(PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct Timestamp {
    /// The number of seconds since the unix epoch.
    secs: i64,
    /// The number of nanoseconds since the last full second.
    nanos: u32,
}

impl Timestamp {
    /// Creates a new timestamp.
    pub(crate) fn now() -> Timestamp {
        Timestamp::from(std::time::SystemTime::now())
    }

    /// Converts the given NTFS timestamp to a `Timestamp`.
    pub(crate) fn from_ntfs_timestamp(val: i64) -> Timestamp {
        const NTFS_EPOCH: time::OffsetDateTime = time::macros::datetime!(1601-01-01 00:00 UTC);

        let val = val as i128 * 100;

        let seconds = val / NANOS_PER_SEC;
        let nanos = val % NANOS_PER_SEC;
        let timestamp = NTFS_EPOCH + time::Duration::new(seconds as i64, nanos as i32);

        timestamp.into()
    }

    /// A helper for displaying only the date of the time stamp.
    pub(crate) fn date(&self) -> Date {
        Date(*self)
    }
}

impl FromStr for Timestamp {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let format = time::macros::format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second] [offset_hour][offset_minute]"
        );

        let mut split = s.split(' ');
        let date = split
            .next()
            .expect("split always yields at least one element");
        let time = split.next();

        let mut dsplit = date.split('-');
        let year = dsplit
            .next()
            .expect("split always yields at least one element");
        let month = dsplit.next().unwrap_or("01");
        let day = dsplit.next().unwrap_or("01");

        let mut hour = "00";
        let mut minute = "00";
        let mut second = "00";

        if let Some(time) = time {
            let mut tsplit = time.split(':');

            hour = tsplit.next().unwrap_or("00");
            minute = tsplit.next().unwrap_or("00");
            second = tsplit.next().unwrap_or("00");
        }

        let rewritten = format!("{year}-{month}-{day} {hour}:{minute}:{second} 0000");

        time::OffsetDateTime::parse(&rewritten, format)
            .with_context(|| format!("failed parsing `{s}` as date"))
            .map(|ts| ts.into())
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            time::OffsetDateTime::from(*self)
                .format(time::macros::format_description!(
                    "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:1+]"
                ))
                .unwrap()
        )
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.secs.cmp(&other.secs) {
            Ordering::Equal => self.nanos.cmp(&other.nanos),
            order => order,
        }
    }
}

impl From<std::time::SystemTime> for Timestamp {
    fn from(systime: std::time::SystemTime) -> Self {
        let mut before_unix = false;

        let duration = match systime.duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration,
            Err(_) => {
                before_unix = true;
                std::time::UNIX_EPOCH
                    .duration_since(systime)
                    .expect("timestamp was neither before nor after unix epoch")
            }
        };

        let sign = if before_unix { -1 } else { 1 };

        Self {
            secs: (duration.as_secs() as i64) * sign,
            nanos: duration.subsec_nanos(),
        }
    }
}

impl From<time::OffsetDateTime> for Timestamp {
    fn from(offsettime: time::OffsetDateTime) -> Self {
        let nanos = offsettime.unix_timestamp_nanos();

        let nanos = if nanos < 0 {
            (NANOS_PER_SEC - (-nanos) % NANOS_PER_SEC) % NANOS_PER_SEC
        } else {
            nanos % NANOS_PER_SEC
        };

        Self {
            secs: offsettime.unix_timestamp(),
            nanos: nanos as u32,
        }
    }
}

impl From<Timestamp> for time::OffsetDateTime {
    fn from(timestamp: Timestamp) -> Self {
        time::OffsetDateTime::from_unix_timestamp(timestamp.secs).unwrap()
            + std::time::Duration::from_nanos(timestamp.nanos as u64)
    }
}

/// A helper type for displaying only the date in the time stamp.
#[derive(Debug)]
pub(crate) struct Date(Timestamp);

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            time::OffsetDateTime::from(self.0)
                .format(time::macros::format_description!("[year]-[month]-[day]"))
                .unwrap()
        )
    }
}

/// A range of dates from `from` to `to`.
#[derive(Debug)]
pub(crate) struct DateRange {
    /// The start time of the date range.
    pub(crate) from: Timestamp,
    /// The end time of the date range.
    pub(crate) to: Timestamp,
}

impl fmt::Display for DateRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let from_date = time::OffsetDateTime::from(self.from).to_calendar_date();
        let to_date = time::OffsetDateTime::from(self.to).to_calendar_date();

        if from_date == to_date {
            write!(f, "{}", self.from.date())
        } else {
            write!(f, "{} to {}", self.from.date(), self.to.date())
        }
    }
}

/// Represents a time span.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) struct Duration {
    /// The inner duration.
    inner: time::Duration,
}

impl Sub<Timestamp> for Timestamp {
    type Output = Duration;

    fn sub(self, rhs: Timestamp) -> Self::Output {
        let self_time: time::OffsetDateTime = self.into();
        let other_time: time::OffsetDateTime = rhs.into();
        let duration = self_time - other_time;

        Self::Output { inner: duration }
    }
}

impl fmt::Debug for Duration {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let duration = self.inner.abs();

        if self.inner.is_negative() {
            write!(f, "-")?;
        }

        if duration.whole_days().abs() >= 365 {
            write!(
                f,
                "{:.1}{}",
                duration.whole_minutes() as f64 / 365.25 / 24.0 / 60.0,
                if f.alternate() { " years" } else { "y" }
            )
        } else if duration.whole_weeks().abs() >= 1 {
            write!(
                f,
                "{:.1}{}",
                duration.whole_minutes() as f64 / 7.0 / 24.0 / 60.0,
                if f.alternate() { " weeks" } else { "w" }
            )
        } else if duration.whole_hours() >= 48 {
            write!(
                f,
                "{:.1}{}",
                duration.whole_seconds() as f64 / 24.0 / 60.0 / 60.0,
                if f.alternate() { " days" } else { "d" }
            )
        } else if duration.whole_hours() >= 1 {
            write!(
                f,
                "{:.1}{}",
                duration.whole_seconds() as f64 / 60.0 / 60.0,
                if f.alternate() { " hours" } else { "h" }
            )
        } else if duration.whole_minutes() >= 1 {
            write!(
                f,
                "{:.1}{}",
                duration.whole_nanoseconds() as f64 / 60.0 / NANOS_PER_SEC as f64,
                if f.alternate() { " min" } else { "min" }
            )
        } else if duration.whole_seconds() >= 1 {
            write!(
                f,
                "{:.1}{}",
                duration.whole_nanoseconds() as f64 / NANOS_PER_SEC as f64,
                if f.alternate() { " s" } else { "s" }
            )
        } else if duration.whole_milliseconds() >= 1 {
            write!(
                f,
                "{:.1}{}",
                duration.whole_nanoseconds() as f64 / 1_000_000.0,
                if f.alternate() { " ms" } else { "ms" }
            )
        } else if duration.whole_microseconds() >= 1 {
            write!(
                f,
                "{:.1}{}",
                duration.whole_nanoseconds() as f64 / 1_000.0,
                if f.alternate() { " µs" } else { "µs" }
            )
        } else {
            write!(
                f,
                "{}{}",
                duration.whole_nanoseconds(),
                if f.alternate() { " ns" } else { "ns" }
            )
        }
    }
}

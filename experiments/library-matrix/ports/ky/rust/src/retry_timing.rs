//! Ky 2.2.0 `core/retry-timing.js`: which header gives a server-provided retry delay, and that
//! delay. Ky matches the three HTTP-date forms with anchored regular expressions made of ASCII
//! literals and `\d`; [`Scanner`] matches the same strings, piece by piece.

use crate::host::{Headers, math_max, now, s, to_number};

/// A reset value is only read as an epoch from this second onwards (2001-09-09).
const TIMESTAMP_THRESHOLD_SECONDS: f64 = 1_000_000_000.0;
const MS_PER_DAY: f64 = 86_400_000.0;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAY_NAMES: [&str; 7] = [
    "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
];

pub struct TimingHeader {
    pub value: &'static str,
    pub allow_timestamp: bool,
}

pub fn get_retry_timing_header(headers: Headers) -> Option<TimingHeader> {
    let header = |value, allow_timestamp| TimingHeader { value, allow_timestamp };
    // Retry-After is authoritative when present. Only missing Retry-After falls through to
    // rate-limit headers.
    if let Some(value) = headers.get("Retry-After") {
        return Some(header(value, false));
    }
    if let Some(value) = headers.get("RateLimit-Reset") {
        return Some(header(value, true));
    }
    // Symfony-based services
    if let Some(value) = headers.get("X-RateLimit-Retry-After") {
        return Some(header(value, false));
    }
    // GitHub, then Twitter
    headers
        .get("X-RateLimit-Reset")
        .or_else(|| headers.get("X-Rate-Limit-Reset"))
        .map(|value| header(value, true))
}

/// `/^\d+$/`: one or more ASCII digits and nothing else.
fn is_delay(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// The pieces of an anchored pattern, consumed from the front of the value.
struct Scanner<'a> {
    rest: &'a str,
}

impl<'a> Scanner<'a> {
    fn literal(&mut self, expected: &str) -> Option<()> {
        self.rest = self.rest.strip_prefix(expected)?;
        Some(())
    }

    /// An alternation of words: the index of the one that matches.
    fn word(&mut self, words: &[&str]) -> Option<usize> {
        let index = words.iter().position(|word| self.rest.starts_with(word))?;
        self.rest = &self.rest[words[index].len()..];
        Some(index)
    }

    /// `\d{count}`, as its value.
    fn digits(&mut self, count: usize) -> Option<i32> {
        let taken = self.rest.get(..count)?;
        if !is_delay(taken) {
            return None;
        }
        self.rest = &self.rest[count..];
        taken.parse().ok()
    }

    fn end(&self) -> Option<()> {
        self.rest.is_empty().then_some(())
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: usize) -> i32 {
    match month {
        1 if is_leap_year(year) => 29,
        1 => 28,
        3 | 5 | 8 | 10 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date, as ECMAScript's `MakeDay` counts them.
/// The day may run past the month's end, which carries into the next month as `MakeDay` does.
fn days_from_civil(year: i32, month: usize, day: i32) -> i32 {
    let month = month as i32 + 1;
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The year, month index and day of a day count, as `getUTCFullYear` and its siblings give them.
fn civil_from_days(days: i32) -> (i32, usize, i32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 2 } else { shifted_month - 10 };
    let year = year_of_era + era * 400 + if month <= 1 { 1 } else { 0 };
    (year, month as usize, day)
}

/// Ky's `createTimestamp`: `Date.UTC` of the parts, with the year set again by `setUTCFullYear`,
/// and `undefined` unless every part round-trips. `Date.UTC` reads a year from 0 to 99 as
/// 1900 + year, so the day has to exist in that year too.
fn create_timestamp(
    year: i32,
    month: usize,
    day: i32,
    hours: i32,
    minutes: i32,
    seconds: i32,
) -> Option<f64> {
    if hours > 23 || minutes > 59 || seconds > 60 {
        return None;
    }
    let normalized_seconds = seconds.min(59);
    let utc_year = if (0..=99).contains(&year) { year + 1900 } else { year };
    if day < 1 || day > days_in_month(utc_year, month) || day > days_in_month(year, month) {
        return None;
    }
    let timestamp = f64::from(days_from_civil(year, month, day)) * MS_PER_DAY
        + f64::from(hours * 3_600_000 + minutes * 60_000 + normalized_seconds * 1000);
    // A leap second is represented by parsing 60 as 59 and adding one second.
    Some(if seconds == 60 { timestamp + 1000.0 } else { timestamp })
}

/// IMF-fixdate, `Sun, 06 Nov 1994 08:49:37 GMT`.
fn imf_date(value: &str) -> Option<Option<f64>> {
    let mut scan = Scanner { rest: value };
    scan.word(&WEEKDAYS)?;
    scan.literal(", ")?;
    let day = scan.digits(2)?;
    scan.literal(" ")?;
    let month = scan.word(&MONTHS)?;
    scan.literal(" ")?;
    let year = scan.digits(4)?;
    scan.literal(" ")?;
    let (hours, minutes, seconds) = time(&mut scan)?;
    scan.literal(" GMT")?;
    scan.end()?;
    Some(create_timestamp(year, month, day, hours, minutes, seconds))
}

/// `08:49:37`
fn time(scan: &mut Scanner<'_>) -> Option<(i32, i32, i32)> {
    let hours = scan.digits(2)?;
    scan.literal(":")?;
    let minutes = scan.digits(2)?;
    scan.literal(":")?;
    let seconds = scan.digits(2)?;
    Some((hours, minutes, seconds))
}

/// RFC 850, `Sunday, 06-Nov-94 08:49:37 GMT`. RFC 9110 maps two-digit years to the latest
/// matching century that is not more than 50 years in the future.
fn rfc850_date(value: &str) -> Option<Option<f64>> {
    let mut scan = Scanner { rest: value };
    scan.word(&WEEKDAY_NAMES)?;
    scan.literal(", ")?;
    let day = scan.digits(2)?;
    scan.literal("-")?;
    let month = scan.word(&MONTHS)?;
    scan.literal("-")?;
    let two_digit_year = scan.digits(2)?;
    scan.literal(" ")?;
    let (hours, minutes, seconds) = time(&mut scan)?;
    scan.literal(" GMT")?;
    scan.end()?;
    let now = now();
    let today = (now / MS_PER_DAY) as i32;
    let (current_year, current_month, current_day) = civil_from_days(today);
    let current_century_year = current_year.div_euclid(100) * 100 + two_digit_year;
    let fifty_years_from_now = f64::from(days_from_civil(current_year + 50, current_month, current_day))
        * MS_PER_DAY
        + (now - f64::from(today) * MS_PER_DAY);
    let mut timestamp = None;
    for year in [current_century_year - 100, current_century_year, current_century_year + 100] {
        if let Some(candidate) = create_timestamp(year, month, day, hours, minutes, seconds) {
            if candidate <= fifty_years_from_now {
                timestamp = Some(candidate);
            }
        }
    }
    Some(timestamp)
}

/// asctime, `Sun Nov  6 08:49:37 1994`: the day is two digits or a space and one digit.
fn asctime_date(value: &str) -> Option<Option<f64>> {
    let mut scan = Scanner { rest: value };
    scan.word(&WEEKDAYS)?;
    scan.literal(" ")?;
    let month = scan.word(&MONTHS)?;
    scan.literal(" ")?;
    let day = match scan.digits(2) {
        Some(day) => day,
        None => {
            scan.literal(" ")?;
            scan.digits(1)?
        }
    };
    scan.literal(" ")?;
    let (hours, minutes, seconds) = time(&mut scan)?;
    scan.literal(" ")?;
    let year = scan.digits(4)?;
    scan.end()?;
    Some(create_timestamp(year, month, day, hours, minutes, seconds))
}

/// Ky's `parseDate`: the first of the three forms whose pattern matches decides.
fn parse_date(value: &str) -> Option<f64> {
    imf_date(value)
        .or_else(|| rfc850_date(value))
        .or_else(|| asctime_date(value))
        .flatten()
}

pub fn calculate_retry_timing_delay(header: &TimingHeader) -> Option<f64> {
    if is_delay(header.value) {
        let seconds = to_number(s(header.value));
        // Standard Retry-After numbers are delay seconds. Timestamp compatibility only applies to
        // reset headers.
        let delay = if header.allow_timestamp && seconds >= TIMESTAMP_THRESHOLD_SECONDS {
            seconds * 1000.0 - now()
        } else {
            seconds * 1000.0
        };
        return Some(math_max(0.0, delay));
    }
    // `createTimestamp()` only returns valid dates, so the delay is always finite.
    parse_date(header.value).map(|timestamp| math_max(0.0, timestamp - now()))
}

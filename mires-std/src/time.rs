//! Temporal quantification: `Duration`, monotonic `Instant`, and wall-clock `SystemTime`.

use crate::error::Error;
use crate::fmt;
use crate::ops::{Add, AddAssign, Sub, SubAssign};

pub use core_crate::time::{Duration, TryFromFloatSecsError};

#[inline]
fn ms_to_duration(ms: f64) -> Duration {
    if !ms.is_finite() || ms <= 0.0 {
        return Duration::ZERO;
    }
    let whole_ms = ms.trunc();
    let secs = (whole_ms / 1000.0).trunc() as u64;
    let rem_ms = ms - (secs as f64) * 1000.0;
    let nanos = (rem_ms * 1_000_000.0).max(0.0).min(999_999_999.0) as u32;
    Duration::new(secs, nanos)
}

/// A measurement of a monotonically nondecreasing clock, backed by JS `performance.now()`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Instant(Duration);

impl Instant {
    /// Returns an instant corresponding to "now".
    #[must_use]
    #[inline]
    pub fn now() -> Instant {
        Instant(ms_to_duration(crate::sys::now_ms()))
    }

    /// Returns the amount of time elapsed from another instant to this one,
    /// or zero duration if `earlier` is later than `self`.
    #[must_use]
    #[inline]
    pub fn duration_since(&self, earlier: Instant) -> Duration {
        self.saturating_duration_since(earlier)
    }

    /// Returns the amount of time elapsed from another instant to this one,
    /// or `None` if `earlier` is later than `self`.
    #[must_use]
    #[inline]
    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        self.0.checked_sub(earlier.0)
    }

    /// Returns the amount of time elapsed from another instant to this one,
    /// saturating at zero.
    #[must_use]
    #[inline]
    pub fn saturating_duration_since(&self, earlier: Instant) -> Duration {
        self.0.saturating_sub(earlier.0)
    }

    /// Returns the amount of time elapsed since this instant was created.
    #[must_use]
    #[inline]
    pub fn elapsed(&self) -> Duration {
        Instant::now().saturating_duration_since(*self)
    }

    /// Returns `Some(t)` where `t` is the time `self + duration`, if representable.
    #[must_use]
    #[inline]
    pub fn checked_add(&self, duration: Duration) -> Option<Instant> {
        self.0.checked_add(duration).map(Instant)
    }

    /// Returns `Some(t)` where `t` is the time `self - duration`, if representable.
    #[must_use]
    #[inline]
    pub fn checked_sub(&self, duration: Duration) -> Option<Instant> {
        self.0.checked_sub(duration).map(Instant)
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    #[inline]
    fn add(self, other: Duration) -> Instant {
        self.checked_add(other)
            .expect("overflow when adding duration to instant")
    }
}

impl AddAssign<Duration> for Instant {
    #[inline]
    fn add_assign(&mut self, other: Duration) {
        *self = *self + other;
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;

    #[inline]
    fn sub(self, other: Duration) -> Instant {
        self.checked_sub(other)
            .expect("overflow when subtracting duration from instant")
    }
}

impl SubAssign<Duration> for Instant {
    #[inline]
    fn sub_assign(&mut self, other: Duration) {
        *self = *self - other;
    }
}

impl Sub<Instant> for Instant {
    type Output = Duration;

    #[inline]
    fn sub(self, other: Instant) -> Duration {
        self.duration_since(other)
    }
}

/// An error returned from the `duration_since` and `elapsed` methods on [`SystemTime`].
#[derive(Clone, Debug)]
pub struct SystemTimeError(Duration);

impl SystemTimeError {
    /// Returns the positive duration representing how far forward the second system time was.
    #[must_use]
    #[inline]
    pub fn duration(&self) -> Duration {
        self.0
    }
}

impl fmt::Display for SystemTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "second time provided was later than self")
    }
}

impl Error for SystemTimeError {}

/// A measurement of the system clock, backed by JS `Date.now()`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SystemTime {
    /// Signed nanoseconds relative to `UNIX_EPOCH`.
    nanos: i128,
}

/// An anchor in time corresponding to `1970-01-01 00:00:00 UTC`.
pub const UNIX_EPOCH: SystemTime = SystemTime { nanos: 0 };

impl SystemTime {
    /// The Unix epoch constant (`1970-01-01 00:00:00 UTC`).
    pub const UNIX_EPOCH: SystemTime = UNIX_EPOCH;

    /// Returns the system time corresponding to "now".
    #[must_use]
    #[inline]
    pub fn now() -> SystemTime {
        let dur = ms_to_duration(crate::sys::epoch_ms());
        SystemTime {
            nanos: dur.as_nanos() as i128,
        }
    }

    /// Returns the amount of time elapsed from `earlier` to `self`, or an error if `earlier` is later.
    pub fn duration_since(&self, earlier: SystemTime) -> Result<Duration, SystemTimeError> {
        let diff = self.nanos - earlier.nanos;
        if diff >= 0 {
            let secs = (diff / 1_000_000_000) as u64;
            let subsec = (diff % 1_000_000_000) as u32;
            Ok(Duration::new(secs, subsec))
        } else {
            let abs = diff.unsigned_abs();
            let secs = (abs / 1_000_000_000) as u64;
            let subsec = (abs % 1_000_000_000) as u32;
            Err(SystemTimeError(Duration::new(secs, subsec)))
        }
    }

    /// Returns the difference from this time to the current clock time.
    #[inline]
    pub fn elapsed(&self) -> Result<Duration, SystemTimeError> {
        SystemTime::now().duration_since(*self)
    }

    /// Returns `Some(t)` where `t` is the time `self + duration`, if representable.
    #[must_use]
    #[inline]
    pub fn checked_add(&self, duration: Duration) -> Option<SystemTime> {
        let delta = i128::try_from(duration.as_nanos()).ok()?;
        self.nanos
            .checked_add(delta)
            .map(|nanos| SystemTime { nanos })
    }

    /// Returns `Some(t)` where `t` is the time `self - duration`, if representable.
    #[must_use]
    #[inline]
    pub fn checked_sub(&self, duration: Duration) -> Option<SystemTime> {
        let delta = i128::try_from(duration.as_nanos()).ok()?;
        self.nanos
            .checked_sub(delta)
            .map(|nanos| SystemTime { nanos })
    }
}

impl Add<Duration> for SystemTime {
    type Output = SystemTime;

    #[inline]
    fn add(self, dur: Duration) -> SystemTime {
        self.checked_add(dur)
            .expect("overflow when adding duration to SystemTime")
    }
}

impl AddAssign<Duration> for SystemTime {
    #[inline]
    fn add_assign(&mut self, other: Duration) {
        *self = *self + other;
    }
}

impl Sub<Duration> for SystemTime {
    type Output = SystemTime;

    #[inline]
    fn sub(self, dur: Duration) -> SystemTime {
        self.checked_sub(dur)
            .expect("overflow when subtracting duration from SystemTime")
    }
}

impl SubAssign<Duration> for SystemTime {
    #[inline]
    fn sub_assign(&mut self, other: Duration) {
        *self = *self - other;
    }
}

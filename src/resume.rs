//! Suspend / resume detection without D-Bus.
//!
//! `CLOCK_MONOTONIC` stops while the system is suspended, `CLOCK_BOOTTIME`
//! keeps counting. Their difference therefore grows by exactly the time
//! spent asleep and never changes otherwise (unlike the wall clock, which
//! NTP or the user can move). The daemon samples it about once a second;
//! a jump of more than [`RESUME_THRESHOLD`] means the machine just woke up.
//!
//! The project has no D-Bus dependency, so logind's `PrepareForSleep`
//! signal is not used; this detector needs no extra crate and also works
//! without systemd-logind.

use std::time::Duration;

/// Smallest sleep that counts as a suspend. Well above scheduler jitter,
/// well below any real suspend (`rtcwake -s 20` sleeps 20 s).
pub const RESUME_THRESHOLD: Duration = Duration::from_secs(3);

/// Time spent suspended since boot (`CLOCK_BOOTTIME - CLOCK_MONOTONIC`).
/// `None` when either clock cannot be read.
pub fn suspended_total() -> Option<Duration> {
    fn read(clock: libc::clockid_t) -> Option<Duration> {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `ts` is a valid, writable timespec for the duration of
        // the call; clock_gettime has no other memory requirements.
        let rc = unsafe { libc::clock_gettime(clock, &mut ts) };
        (rc == 0 && ts.tv_sec >= 0 && ts.tv_nsec >= 0)
            .then(|| Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
    }
    let mono = read(libc::CLOCK_MONOTONIC)?;
    let boot = read(libc::CLOCK_BOOTTIME)?;
    Some(boot.saturating_sub(mono))
}

/// Pure: how long the machine slept between two samples of
/// [`suspended_total`], or `None` when it did not (growth below
/// `threshold`, first sample, or a clock that went backwards).
pub fn detect_resume(
    previous: Option<Duration>,
    now: Duration,
    threshold: Duration,
) -> Option<Duration> {
    let slept = now.checked_sub(previous?)?;
    (slept >= threshold).then_some(slept)
}

/// Stateful wrapper for a polling loop.
#[derive(Debug, Default)]
pub struct ResumeDetector {
    last: Option<Duration>,
}

impl ResumeDetector {
    /// Feed one sample; returns the sleep length when a resume happened.
    pub fn observe(&mut self, suspended_total: Duration) -> Option<Duration> {
        let hit = detect_resume(self.last, suspended_total, RESUME_THRESHOLD);
        self.last = Some(suspended_total);
        hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn first_sample_is_never_a_resume() {
        assert_eq!(detect_resume(None, S(500), RESUME_THRESHOLD), None);
    }

    #[test]
    fn small_growth_is_jitter_not_sleep() {
        assert_eq!(detect_resume(Some(S(10)), S(10), RESUME_THRESHOLD), None);
        assert_eq!(
            detect_resume(
                Some(S(10)),
                S(10) + Duration::from_millis(2900),
                RESUME_THRESHOLD
            ),
            None
        );
    }

    #[test]
    fn suspend_is_reported_with_its_length() {
        assert_eq!(
            detect_resume(Some(S(10)), S(13), RESUME_THRESHOLD),
            Some(S(3))
        );
        assert_eq!(
            detect_resume(Some(S(10)), S(30), RESUME_THRESHOLD),
            Some(S(20))
        );
    }

    #[test]
    fn backwards_clock_is_ignored() {
        assert_eq!(detect_resume(Some(S(30)), S(10), RESUME_THRESHOLD), None);
    }

    #[test]
    fn detector_fires_once_per_resume() {
        let mut d = ResumeDetector::default();
        assert_eq!(d.observe(S(0)), None);
        assert_eq!(d.observe(S(0)), None);
        assert_eq!(d.observe(S(20)), Some(S(20)));
        assert_eq!(d.observe(S(20)), None, "same sample again: no second hit");
        assert_eq!(d.observe(S(21)), None);
    }

    #[test]
    fn real_clocks_are_readable() {
        let a = suspended_total().expect("clocks readable");
        let b = suspended_total().unwrap();
        assert!(b >= a || a - b < Duration::from_millis(5));
    }
}

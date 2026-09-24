use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use dali2rust_platform::wall_clock::{LocalCivilTime, TimeError, TimeSource, WallClock};

const MIN_PLAUSIBLE_UNIX_MS: u64 = 1_704_067_200_000;

const DEFAULT_TZ: &str = "UTC0";

const MAX_TZ_BYTES: usize = 64;

pub struct SystemWallClock {
    state: Mutex<ClockState>,
}

struct ClockState {
    offset_ms: i64,
    source: TimeSource,
    timezone: String,
}

impl Default for SystemWallClock {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemWallClock {
    pub fn new() -> Self {
        let clock = Self {
            state: Mutex::new(ClockState {
                offset_ms: 0,
                source: TimeSource::Unset,
                timezone: DEFAULT_TZ.to_string(),
            }),
        };
        install_tz(DEFAULT_TZ);
        clock
    }

    pub fn adopt_sntp(&self) {
        let mut state = self.lock();
        state.offset_ms = 0;
        state.source = TimeSource::Sntp;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ClockState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn system_clock_is_plausible() -> bool {
        Self::raw_now_ms() >= MIN_PLAUSIBLE_UNIX_MS
    }

    fn raw_now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn anchored_ms(state: &ClockState) -> Option<u64> {
        let now = Self::raw_now_ms() as i64 + state.offset_ms;
        let now = u64::try_from(now).ok()?;
        (now >= MIN_PLAUSIBLE_UNIX_MS).then_some(now)
    }
}

impl WallClock for SystemWallClock {
    fn now_ms(&self) -> Option<u64> {
        Self::anchored_ms(&self.lock())
    }

    fn source(&self) -> TimeSource {
        let state = self.lock();
        if state.source == TimeSource::Unset && Self::anchored_ms(&state).is_some() {
            return TimeSource::Sntp;
        }
        state.source
    }

    fn set_manual_ms(&self, unix_ms: u64) -> Result<(), TimeError> {
        if unix_ms < MIN_PLAUSIBLE_UNIX_MS {
            return Err(TimeError::NotPlausible);
        }
        let mut state = self.lock();
        state.offset_ms = unix_ms as i64 - Self::raw_now_ms() as i64;
        state.source = TimeSource::Manual;
        Ok(())
    }

    fn timezone(&self) -> String {
        self.lock().timezone.clone()
    }

    fn set_timezone(&self, posix_tz: &str) -> Result<(), TimeError> {
        if posix_tz.is_empty() || posix_tz.len() > MAX_TZ_BYTES || posix_tz.contains('\0') {
            return Err(TimeError::InvalidTimezone);
        }
        if !install_tz(posix_tz) {
            return Err(TimeError::InvalidTimezone);
        }
        self.lock().timezone = posix_tz.to_string();
        Ok(())
    }

    fn local(&self) -> Option<LocalCivilTime> {
        let unix_ms = self.now_ms()?;
        local_civil_time(unix_ms)
    }
}

extern "C" {
    fn tzset();
}

fn install_tz(tz: &str) -> bool {
    let Ok(value) = std::ffi::CString::new(tz) else {
        return false;
    };
    let name = c"TZ";
    // SAFETY: NUL-terminated strings live across the calls and `setenv` copies them; zone changes are rare.
    unsafe {
        libc::setenv(name.as_ptr(), value.as_ptr(), 1);
        tzset();
    }
    true
}

fn local_civil_time(unix_ms: u64) -> Option<LocalCivilTime> {
    let seconds = (unix_ms / 1000) as libc::time_t;
    let local = civil(seconds, true)?;
    let utc = civil(seconds, false)?;
    Some(LocalCivilTime {
        minutes_since_midnight: (local.tm_hour as u16) * 60 + local.tm_min as u16,
        weekday: ((local.tm_wday + 6) % 7) as u8,
        year_day: (local.tm_yday + 1) as u16,
        utc_offset_minutes: offset_minutes(&local, &utc),
    })
}

fn civil(seconds: libc::time_t, local: bool) -> Option<libc::tm> {
    // SAFETY: `libc::tm` is plain integers and a pointer, all valid when zeroed.
    let mut tm: libc::tm = unsafe { core::mem::zeroed() };
    // SAFETY: both functions fill the caller-owned `tm`; the pointers are valid for the call and not retained.
    let filled = unsafe {
        if local {
            libc::localtime_r(&raw const seconds, &raw mut tm)
        } else {
            libc::gmtime_r(&raw const seconds, &raw mut tm)
        }
    };
    (!filled.is_null()).then_some(tm)
}

fn offset_minutes(local: &libc::tm, utc: &libc::tm) -> i16 {
    let day_minutes = |tm: &libc::tm| tm.tm_hour * 60 + tm.tm_min;
    let mut diff = day_minutes(local) - day_minutes(utc);
    let day_shift = if local.tm_year != utc.tm_year {
        if local.tm_year > utc.tm_year {
            1
        } else {
            -1
        }
    } else {
        (local.tm_yday - utc.tm_yday).signum()
    };
    diff += day_shift * 24 * 60;
    diff as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUNDAY_NOON_UTC_MS: u64 = 1_785_067_200_000;

    #[test]
    fn an_unanchored_clock_reports_no_time() {
        let clock = SystemWallClock::new();
        clock
            .lock()
            .offset_ms = -(SystemWallClock::raw_now_ms() as i64);
        assert_eq!(clock.now_ms(), None);
        assert!(clock.local().is_none());
    }

    #[test]
    fn a_manual_anchor_is_readable_and_plausible_only() {
        let clock = SystemWallClock::new();
        assert_eq!(
            clock.set_manual_ms(1_000),
            Err(TimeError::NotPlausible),
            "a client clock stuck in 1970 must not start the schedule"
        );

        clock.set_manual_ms(SUNDAY_NOON_UTC_MS).expect("plausible");
        assert_eq!(clock.source(), TimeSource::Manual);
        let now = clock.now_ms().expect("anchored");
        assert!(now.abs_diff(SUNDAY_NOON_UTC_MS) < 2_000, "got {now}");
    }

    static TZ_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn the_zone_moves_local_time_and_dst_rules_apply() {
        let _zone = TZ_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let clock = SystemWallClock::new();
        clock.set_manual_ms(SUNDAY_NOON_UTC_MS).expect("plausible");

        clock.set_timezone("UTC0").expect("utc");
        let utc = clock.local().expect("local");
        assert_eq!(utc.minutes_since_midnight, 12 * 60);
        assert_eq!(utc.weekday, 6, "2026-07-26 is a Sunday");
        assert_eq!(utc.year_day, 207);
        assert_eq!(utc.utc_offset_minutes, 0);

        clock.set_timezone("MSK-3").expect("moscow");
        assert_eq!(clock.local().expect("local").minutes_since_midnight, 15 * 60);
        assert_eq!(clock.local().expect("local").utc_offset_minutes, 180);

        clock
            .set_timezone("CET-1CEST,M3.5.0/2,M10.5.0/3")
            .expect("cet");
        assert_eq!(clock.local().expect("local").utc_offset_minutes, 120);
    }

    #[test]
    fn a_junk_zone_is_refused_and_the_previous_one_stays() {
        let _zone = TZ_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let clock = SystemWallClock::new();
        clock.set_timezone("MSK-3").expect("moscow");
        assert_eq!(clock.set_timezone(""), Err(TimeError::InvalidTimezone));
        assert_eq!(
            clock.set_timezone(&"X".repeat(MAX_TZ_BYTES + 1)),
            Err(TimeError::InvalidTimezone)
        );
        assert_eq!(clock.timezone(), "MSK-3");
    }

    #[test]
    fn sntp_supersedes_a_manual_offset_instead_of_stacking_on_it() {
        let clock = SystemWallClock::new();
        clock
            .set_manual_ms(SUNDAY_NOON_UTC_MS + 3_600_000)
            .expect("plausible");
        clock.adopt_sntp();
        assert_eq!(clock.source(), TimeSource::Sntp);
        let now = clock.now_ms().expect("anchored");
        assert!(
            now.abs_diff(SystemWallClock::raw_now_ms()) < 2_000,
            "the manual hour must be gone, got {now}"
        );
    }
}

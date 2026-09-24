use dali2rust_platform::clock::Clock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Ok,
    Degraded,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub uptime_secs: u64,
    pub version: &'static str,
}

pub struct HealthService {
    start_ms: u64,
    version: &'static str,
}

impl HealthService {
    pub fn new(clock: &dyn Clock, version: &'static str) -> Self {
        Self {
            start_ms: clock.monotonic_ms(),
            version,
        }
    }

    pub fn check(&self, clock: &dyn Clock) -> HealthReport {
        let uptime_ms = clock.monotonic_ms().saturating_sub(self.start_ms);
        HealthReport {
            status: HealthStatus::Ok,
            uptime_secs: uptime_ms / 1000,
            version: self.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeClock;

    #[test]
    fn health_check_returns_ok() {
        let clock = FakeClock::new(0);
        let svc = HealthService::new(&clock, "0.1.0-test");
        let report = svc.check(&clock);
        assert_eq!(report.status, HealthStatus::Ok);
    }

    #[test]
    fn health_check_version() {
        let clock = FakeClock::new(0);
        let svc = HealthService::new(&clock, "1.2.3");
        let report = svc.check(&clock);
        assert_eq!(report.version, "1.2.3");
    }

    #[test]
    fn health_check_uptime_increases() {
        let clock = FakeClock::new(0);
        let svc = HealthService::new(&clock, "test");
        let t1 = svc.check(&clock).uptime_secs;
        assert_eq!(t1, 0);
        clock.advance(5100);
        let t2 = svc.check(&clock).uptime_secs;
        assert_eq!(t2, 5);
    }
}

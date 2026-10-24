use dali2rust_platform::clock::Uptime;

const MS_PER_SECOND: u64 = 1_000;

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
    uptime: Uptime,
    version: &'static str,
}

impl HealthService {
    pub fn new(uptime: Uptime, version: &'static str) -> Self {
        Self { uptime, version }
    }

    pub fn check(&self) -> HealthReport {
        HealthReport {
            status: HealthStatus::Ok,
            uptime_secs: self.uptime.elapsed_ms() / MS_PER_SECOND,
            version: self.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dali2rust_platform::clock::Clock;

    use super::*;
    use crate::test_support::FakeClock;

    fn service_at(clock: &Arc<FakeClock>, version: &'static str) -> HealthService {
        HealthService::new(Uptime::start(Arc::clone(clock) as Arc<dyn Clock>), version)
    }

    #[test]
    fn health_check_returns_ok() {
        let clock = Arc::new(FakeClock::new(0));
        let report = service_at(&clock, "0.1.0-test").check();
        assert_eq!(report.status, HealthStatus::Ok);
    }

    #[test]
    fn health_check_version() {
        let clock = Arc::new(FakeClock::new(0));
        let report = service_at(&clock, "1.2.3").check();
        assert_eq!(report.version, "1.2.3");
    }

    #[test]
    fn health_check_uptime_increases() {
        let clock = Arc::new(FakeClock::new(0));
        let svc = service_at(&clock, "test");
        let t1 = svc.check().uptime_secs;
        assert_eq!(t1, 0);
        clock.advance(5100);
        let t2 = svc.check().uptime_secs;
        assert_eq!(t2, 5);
    }
}

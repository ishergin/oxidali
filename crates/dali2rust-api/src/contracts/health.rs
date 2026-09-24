use crate::contracts::BufferBytes;
use dali2rust_domain::health::{HealthReport, HealthStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HealthResponseJson {
    status: String,
    uptime_seconds: u64,
    version: String,
}

pub struct HealthResponseBuffer {
    data: Vec<u8>,
}

impl HealthResponseBuffer {
    pub fn from_report(report: &HealthReport) -> Self {
        let status_str = match report.status {
            HealthStatus::Ok => "ok",
            HealthStatus::Degraded => "degraded",
            HealthStatus::Down => "down",
        };
        let v = HealthResponseJson {
            status: status_str.to_string(),
            uptime_seconds: report.uptime_secs,
            version: report.version.to_string(),
        };
        Self {
            data: serde_json::to_vec(&v).unwrap_or_default(),
        }
    }
}

impl BufferBytes for HealthResponseBuffer {
    fn inner_data(&self) -> &Vec<u8> {
        &self.data
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthResponseRead {
    pub status: String,
    pub uptime_seconds: u64,
    pub version: String,
}

impl HealthResponseRead {
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        let resp: HealthResponseJson = serde_json::from_slice(buf).ok()?;
        Some(Self {
            status: resp.status,
            uptime_seconds: resp.uptime_seconds,
            version: resp.version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_buffer_from_report() {
        let report = HealthReport {
            status: HealthStatus::Ok,
            uptime_secs: 42,
            version: "0.1.0",
        };
        let buf = HealthResponseBuffer::from_report(&report);
        assert!(!buf.as_bytes().is_empty());
    }

    #[test]
    fn health_buffer_roundtrip() {
        let report = HealthReport {
            status: HealthStatus::Ok,
            uptime_secs: 100,
            version: "1.0.0-test",
        };
        let buf = HealthResponseBuffer::from_report(&report);
        let read = HealthResponseRead::from_bytes(buf.as_bytes()).expect("parse");

        assert_eq!(read.status, "ok");
        assert_eq!(read.uptime_seconds, 100);
        assert_eq!(read.version, "1.0.0-test");
    }

    #[test]
    fn health_buffer_degraded_status() {
        let report = HealthReport {
            status: HealthStatus::Degraded,
            uptime_secs: 0,
            version: "0.1.0",
        };
        let buf = HealthResponseBuffer::from_report(&report);
        let read = HealthResponseRead::from_bytes(buf.as_bytes()).expect("parse");
        assert_eq!(read.status, "degraded");
    }

    #[test]
    fn health_buffer_parse_empty_fails() {
        assert!(HealthResponseRead::from_bytes(&[]).is_none());
    }

    #[test]
    fn health_buffer_parse_garbage_fails() {
        assert!(HealthResponseRead::from_bytes(&[0xFF; 4]).is_none());
    }
}

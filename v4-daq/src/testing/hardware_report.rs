//! Hardware-specific test reporting and metrics collection.
//!
//! This module provides detailed hardware testing capabilities including:
//! - Temperature and power monitoring
//! - Position and movement tracking
//! - Safety incident logging
//! - Performance metrics collection
//! - Hardware-specific error categorization

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Hardware-specific test report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareReport {
    /// Report generation timestamp
    pub timestamp: DateTime<Utc>,
    /// Hardware device identifier
    pub device_id: String,
    /// Device type (e.g., "MaiTai", "Newport1830C")
    pub device_type: String,
    /// Firmware version if available
    pub firmware_version: Option<String>,
    /// Hardware status
    pub status: HardwareStatus,
    /// Environmental conditions
    pub environment: EnvironmentalMetrics,
    /// Safety incidents during test
    pub safety_incidents: Vec<SafetyIncident>,
    /// Performance metrics
    pub performance: HardwarePerformance,
    /// Detailed test logs
    pub test_logs: Vec<TestLog>,
    /// Hardware-specific measurements
    pub measurements: HashMap<String, MeasurementData>,
}

/// Hardware operational status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HardwareStatus {
    /// Hardware operating normally
    Healthy,
    /// Hardware operating with warnings
    Degraded,
    /// Hardware has errors but is operational
    Faulty,
    /// Hardware is not responding
    Unresponsive,
    /// Hardware test in progress
    Testing,
    /// Hardware initialization complete
    Ready,
}

impl HardwareStatus {
    /// Check if status indicates normal operation
    pub fn is_operational(&self) -> bool {
        matches!(
            self,
            HardwareStatus::Healthy | HardwareStatus::Degraded | HardwareStatus::Ready
        )
    }

    /// Get human-readable status string
    pub fn as_str(&self) -> &'static str {
        match self {
            HardwareStatus::Healthy => "HEALTHY",
            HardwareStatus::Degraded => "DEGRADED",
            HardwareStatus::Faulty => "FAULTY",
            HardwareStatus::Unresponsive => "UNRESPONSIVE",
            HardwareStatus::Testing => "TESTING",
            HardwareStatus::Ready => "READY",
        }
    }
}

/// Environmental conditions during testing
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvironmentalMetrics {
    /// Ambient temperature in Celsius
    pub ambient_temperature_c: Option<f64>,
    /// Humidity percentage
    pub humidity_percent: Option<f64>,
    /// Barometric pressure in hPa
    pub pressure_hpa: Option<f64>,
    /// Dust level measurement
    pub dust_level: Option<f64>,
    /// Vibration level measurement
    pub vibration_level: Option<f64>,
}

/// Safety incident during test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyIncident {
    /// Timestamp of incident
    pub timestamp: DateTime<Utc>,
    /// Incident severity
    pub severity: IncidentSeverity,
    /// Incident type
    pub incident_type: IncidentType,
    /// Detailed description
    pub description: String,
    /// Recovery action taken
    pub recovery_action: Option<String>,
}

/// Safety incident severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum IncidentSeverity {
    /// Minor issue with no impact
    Info,
    /// Warning-level issue
    Warning,
    /// Serious issue requiring attention
    Critical,
    /// Shutdown-level emergency
    Emergency,
}

impl IncidentSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            IncidentSeverity::Info => "INFO",
            IncidentSeverity::Warning => "WARNING",
            IncidentSeverity::Critical => "CRITICAL",
            IncidentSeverity::Emergency => "EMERGENCY",
        }
    }
}

/// Incident type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IncidentType {
    /// Temperature out of range
    TemperatureExceeded,
    /// Power anomaly detected
    PowerAnomaly,
    /// Motion limit reached
    MotionLimitReached,
    /// Unexpected communication loss
    CommunicationLoss,
    /// Hardware timeout
    Timeout,
    /// User intervention required
    UserIntervention,
    /// Environmental condition violation
    EnvironmentalViolation,
    /// Other safety issue
    Other,
}

impl IncidentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IncidentType::TemperatureExceeded => "TEMPERATURE_EXCEEDED",
            IncidentType::PowerAnomaly => "POWER_ANOMALY",
            IncidentType::MotionLimitReached => "MOTION_LIMIT_REACHED",
            IncidentType::CommunicationLoss => "COMMUNICATION_LOSS",
            IncidentType::Timeout => "TIMEOUT",
            IncidentType::UserIntervention => "USER_INTERVENTION",
            IncidentType::EnvironmentalViolation => "ENVIRONMENTAL_VIOLATION",
            IncidentType::Other => "OTHER",
        }
    }
}

/// Hardware performance metrics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HardwarePerformance {
    /// Response time in milliseconds
    pub response_time_ms: Option<f64>,
    /// Command success rate percentage
    pub command_success_rate: Option<f64>,
    /// Average power consumption in watts
    pub power_consumption_w: Option<f64>,
    /// Temperature stability (standard deviation)
    pub temperature_stability: Option<f64>,
    /// Position repeatability in microns
    pub position_repeatability_um: Option<f64>,
    /// Throughput (commands/sec)
    pub throughput_cps: Option<f64>,
    /// Error rate (errors/1000 operations)
    pub error_rate_per_1k: Option<f64>,
}

/// Test log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestLog {
    /// Log timestamp
    pub timestamp: DateTime<Utc>,
    /// Log level
    pub level: LogLevel,
    /// Log message
    pub message: String,
    /// Optional context data
    pub context: Option<HashMap<String, String>>,
}

/// Log level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    /// Debug information
    Debug,
    /// Informational message
    Info,
    /// Warning message
    Warn,
    /// Error message
    Error,
    /// Critical/fatal error
    Critical,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Critical => "CRITICAL",
        }
    }
}

/// Measurement data collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementData {
    /// Measurement name
    pub name: String,
    /// Unit of measurement
    pub unit: String,
    /// Individual measurements
    pub values: Vec<f64>,
    /// Statistical summary
    pub stats: MeasurementStats,
}

/// Statistical summary of measurements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementStats {
    /// Minimum value
    pub min: f64,
    /// Maximum value
    pub max: f64,
    /// Average value
    pub mean: f64,
    /// Standard deviation
    pub std_dev: f64,
    /// Number of measurements
    pub count: usize,
}

impl HardwareReport {
    /// Create a new hardware report
    pub fn new(device_id: String, device_type: String) -> Self {
        Self {
            timestamp: Utc::now(),
            device_id,
            device_type,
            firmware_version: None,
            status: HardwareStatus::Testing,
            environment: EnvironmentalMetrics::default(),
            safety_incidents: Vec::new(),
            performance: HardwarePerformance::default(),
            test_logs: Vec::new(),
            measurements: HashMap::new(),
        }
    }

    /// Set firmware version
    pub fn with_firmware(mut self, version: String) -> Self {
        self.firmware_version = Some(version);
        self
    }

    /// Set hardware status
    pub fn with_status(mut self, status: HardwareStatus) -> Self {
        self.status = status;
        self
    }

    /// Add safety incident
    pub fn add_safety_incident(&mut self, incident: SafetyIncident) {
        self.safety_incidents.push(incident);
    }

    /// Add test log entry
    pub fn add_log(&mut self, level: LogLevel, message: String) {
        self.test_logs.push(TestLog {
            timestamp: Utc::now(),
            level,
            message,
            context: None,
        });
    }

    /// Add test log with context
    pub fn add_log_with_context(
        &mut self,
        level: LogLevel,
        message: String,
        context: HashMap<String, String>,
    ) {
        self.test_logs.push(TestLog {
            timestamp: Utc::now(),
            level,
            message,
            context: Some(context),
        });
    }

    /// Add measurement data
    pub fn add_measurement(&mut self, name: String, unit: String, values: Vec<f64>) {
        let stats = calculate_stats(&values);
        self.measurements.insert(
            name.clone(),
            MeasurementData {
                name,
                unit,
                values,
                stats,
            },
        );
    }

    /// Get critical incidents
    pub fn critical_incidents(&self) -> Vec<&SafetyIncident> {
        self.safety_incidents
            .iter()
            .filter(|i| i.severity == IncidentSeverity::Critical || i.severity == IncidentSeverity::Emergency)
            .collect()
    }

    /// Get error logs
    pub fn error_logs(&self) -> Vec<&TestLog> {
        self.test_logs
            .iter()
            .filter(|l| l.level == LogLevel::Error || l.level == LogLevel::Critical)
            .collect()
    }

    /// Generate markdown report
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str(&format!(
            "# Hardware Report: {}\n\n**Device**: {}\n**Timestamp**: {}\n**Status**: {}\n\n",
            self.device_id,
            self.device_type,
            self.timestamp.format("%Y-%m-%d %H:%M:%S"),
            self.status.as_str()
        ));

        if let Some(fw) = &self.firmware_version {
            md.push_str(&format!("**Firmware**: {}\n\n", fw));
        }

        // Environmental Conditions
        md.push_str("## Environmental Conditions\n\n");
        if let Some(temp) = self.environment.ambient_temperature_c {
            md.push_str(&format!("- **Temperature**: {:.1}°C\n", temp));
        }
        if let Some(humidity) = self.environment.humidity_percent {
            md.push_str(&format!("- **Humidity**: {:.1}%\n", humidity));
        }
        if let Some(pressure) = self.environment.pressure_hpa {
            md.push_str(&format!("- **Pressure**: {:.1} hPa\n", pressure));
        }
        md.push_str("\n");

        // Performance Metrics
        md.push_str("## Performance Metrics\n\n");
        if let Some(resp_time) = self.performance.response_time_ms {
            md.push_str(&format!("- **Response Time**: {:.2} ms\n", resp_time));
        }
        if let Some(success) = self.performance.command_success_rate {
            md.push_str(&format!("- **Command Success Rate**: {:.1}%\n", success));
        }
        if let Some(power) = self.performance.power_consumption_w {
            md.push_str(&format!("- **Power Consumption**: {:.2} W\n", power));
        }
        if let Some(repeatability) = self.performance.position_repeatability_um {
            md.push_str(&format!("- **Position Repeatability**: ±{:.2} µm\n", repeatability));
        }
        md.push_str("\n");

        // Safety Incidents
        if !self.safety_incidents.is_empty() {
            md.push_str("## Safety Incidents\n\n");
            for incident in &self.safety_incidents {
                md.push_str(&format!(
                    "### {} - {} ({})\n\n",
                    incident.timestamp.format("%H:%M:%S"),
                    incident.incident_type.as_str(),
                    incident.severity.as_str()
                ));
                md.push_str(&format!("{}\n\n", incident.description));
                if let Some(recovery) = &incident.recovery_action {
                    md.push_str(&format!("**Recovery**: {}\n\n", recovery));
                }
            }
        }

        // Measurements
        if !self.measurements.is_empty() {
            md.push_str("## Measurements\n\n");
            for (_key, measurement) in &self.measurements {
                md.push_str(&format!(
                    "### {}\n\n**Unit**: {}\n\n",
                    measurement.name, measurement.unit
                ));
                md.push_str(&format!(
                    "- **Min**: {:.4}\n- **Max**: {:.4}\n- **Mean**: {:.4}\n- **StdDev**: {:.4}\n- **Count**: {}\n\n",
                    measurement.stats.min,
                    measurement.stats.max,
                    measurement.stats.mean,
                    measurement.stats.std_dev,
                    measurement.stats.count
                ));
            }
        }

        // Test Logs
        if !self.test_logs.is_empty() {
            md.push_str("## Test Logs\n\n");
            md.push_str("```\n");
            for log in &self.test_logs {
                md.push_str(&format!(
                    "[{}] {}: {}\n",
                    log.timestamp.format("%H:%M:%S"),
                    log.level.as_str(),
                    log.message
                ));
            }
            md.push_str("```\n\n");
        }

        md
    }
}

/// Calculate statistics for measurement values
fn calculate_stats(values: &[f64]) -> MeasurementStats {
    if values.is_empty() {
        return MeasurementStats {
            min: 0.0,
            max: 0.0,
            mean: 0.0,
            std_dev: 0.0,
            count: 0,
        };
    }

    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mean = values.iter().sum::<f64>() / values.len() as f64;

    let variance = values
        .iter()
        .map(|v| (v - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    let std_dev = variance.sqrt();

    MeasurementStats {
        min,
        max,
        mean,
        std_dev,
        count: values.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_report_creation() {
        let report = HardwareReport::new("DEV001".to_string(), "MaiTai".to_string());

        assert_eq!(report.device_id, "DEV001");
        assert_eq!(report.device_type, "MaiTai");
        assert_eq!(report.status, HardwareStatus::Testing);
    }

    #[test]
    fn test_add_safety_incident() {
        let mut report = HardwareReport::new("DEV001".to_string(), "MaiTai".to_string());

        let incident = SafetyIncident {
            timestamp: Utc::now(),
            severity: IncidentSeverity::Warning,
            incident_type: IncidentType::TemperatureExceeded,
            description: "Temperature exceeded threshold".to_string(),
            recovery_action: Some("Reduced power output".to_string()),
        };

        report.add_safety_incident(incident);
        assert_eq!(report.safety_incidents.len(), 1);
    }

    #[test]
    fn test_add_measurement() {
        let mut report = HardwareReport::new("DEV001".to_string(), "MaiTai".to_string());

        let values = vec![25.0, 26.0, 24.5, 25.5, 26.2];
        report.add_measurement(
            "Temperature".to_string(),
            "Celsius".to_string(),
            values.clone(),
        );

        assert!(report.measurements.contains_key("Temperature"));
        let measurement = &report.measurements["Temperature"];
        assert_eq!(measurement.stats.count, 5);
        assert_eq!(measurement.stats.min, 24.5);
        assert_eq!(measurement.stats.max, 26.2);
    }

    #[test]
    fn test_measurement_stats() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let stats = calculate_stats(&values);

        assert_eq!(stats.min, 1.0);
        assert_eq!(stats.max, 5.0);
        assert_eq!(stats.mean, 3.0);
        assert_eq!(stats.count, 5);
    }

    #[test]
    fn test_critical_incidents() {
        let mut report = HardwareReport::new("DEV001".to_string(), "MaiTai".to_string());

        report.add_safety_incident(SafetyIncident {
            timestamp: Utc::now(),
            severity: IncidentSeverity::Warning,
            incident_type: IncidentType::Other,
            description: "Minor issue".to_string(),
            recovery_action: None,
        });

        report.add_safety_incident(SafetyIncident {
            timestamp: Utc::now(),
            severity: IncidentSeverity::Critical,
            incident_type: IncidentType::TemperatureExceeded,
            description: "Critical temperature".to_string(),
            recovery_action: None,
        });

        assert_eq!(report.critical_incidents().len(), 1);
    }

    #[test]
    fn test_markdown_generation() {
        let report = HardwareReport::new("DEV001".to_string(), "MaiTai".to_string());
        let md = report.to_markdown();

        assert!(md.contains("DEV001"));
        assert!(md.contains("MaiTai"));
        assert!(md.contains("Hardware Report"));
    }
}

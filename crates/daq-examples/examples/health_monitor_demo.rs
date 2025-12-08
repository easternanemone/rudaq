//! Health monitoring demonstration (bd-pauy)
//!
//! This example demonstrates the SystemHealthMonitor service for headless operation.
//! Run with: cargo run --example health_monitor_demo

use rust_daq::health::{ErrorSeverity, HealthMonitorConfig, SystemHealthMonitor};
use std::time::Duration;

#[tokio::main]
async fn main() {
    println!("=== Health Monitoring Demo ===\n");

    // Create health monitor with custom config
    let config = HealthMonitorConfig {
        heartbeat_timeout: Duration::from_secs(5),
        max_error_history: 100,
    };
    let monitor = SystemHealthMonitor::new(config);

    // Simulate modules registering heartbeats
    println!("1. Registering module heartbeats...");
    monitor.heartbeat("data_acquisition").await;
    monitor.heartbeat("camera_driver").await;
    monitor
        .heartbeat_with_message("stage_controller", Some("Initialized".to_string()))
        .await;

    let modules = monitor.get_module_health().await;
    println!("   Registered {} modules", modules.len());
    for m in &modules {
        println!(
            "   - {}: {}",
            m.name,
            if m.is_healthy { "healthy" } else { "unhealthy" }
        );
    }

    // Check system health (should be healthy)
    println!("\n2. Checking system health...");
    let health = monitor.get_system_health().await;
    println!("   System status: {:?}", health);

    // Simulate a warning
    println!("\n3. Simulating warning from camera...");
    monitor
        .report_error(
            "camera_driver",
            ErrorSeverity::Warning,
            "Frame rate dropped to 15 fps",
            vec![("target_fps", "30"), ("actual_fps", "15")],
        )
        .await;

    let health = monitor.get_system_health().await;
    println!("   System status: {:?}", health);

    // Get error history
    println!("\n4. Checking error history...");
    let errors = monitor.get_error_history(Some(10)).await;
    println!("   Found {} errors:", errors.len());
    for err in &errors {
        println!(
            "   - [{}] {}: {}",
            err.severity, err.module_name, err.message
        );
        if !err.context.is_empty() {
            println!("     Context: {:?}", err.context);
        }
    }

    // Simulate a critical error
    println!("\n5. Simulating critical error from stage...");
    monitor
        .report_error(
            "stage_controller",
            ErrorSeverity::Critical,
            "Stage position readout failed",
            vec![("device_id", "stage0"), ("last_position", "unknown")],
        )
        .await;

    let health = monitor.get_system_health().await;
    println!("   System status: {:?}", health);

    // Get module-specific errors
    println!("\n6. Checking stage controller errors...");
    let stage_errors = monitor.get_module_errors("stage_controller", None).await;
    println!("   Stage errors: {}", stage_errors.len());
    for err in &stage_errors {
        println!("   - [{}] {}", err.severity, err.message);
    }

    // Simulate module timeout
    println!("\n7. Waiting for heartbeat timeout (5 seconds)...");
    tokio::time::sleep(Duration::from_secs(6)).await;

    let modules = monitor.get_module_health().await;
    println!("   Module health after timeout:");
    for m in &modules {
        let status = if m.is_healthy { "healthy" } else { "UNHEALTHY" };
        println!("   - {}: {}", m.name, status);
    }

    let health = monitor.get_system_health().await;
    println!("   System status: {:?}", health);

    // Refresh heartbeat
    println!("\n8. Refreshing data_acquisition heartbeat...");
    monitor.heartbeat("data_acquisition").await;

    let modules = monitor.get_module_health().await;
    println!("   Module health after refresh:");
    for m in &modules {
        let status = if m.is_healthy { "healthy" } else { "UNHEALTHY" };
        println!("   - {}: {}", m.name, status);
    }

    // Show final statistics
    println!("\n9. Final statistics:");
    println!("   Total modules: {}", monitor.module_count().await);
    println!("   Total errors: {}", monitor.error_count().await);

    let health = monitor.get_system_health().await;
    println!("   Final system status: {:?}", health);

    println!("\n=== Demo Complete ===");
}

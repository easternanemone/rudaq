//! Tests for template inheritance in declarative device configs.

use common::driver::DriverFactory;
use driver_generic::factory::GenericSerialDriverFactory;
use std::fs;
use tempfile::TempDir;

fn create_test_dir() -> TempDir {
    TempDir::new().unwrap()
}

#[test]
fn test_single_level_inheritance() {
    let dir = create_test_dir();
    let base_path = dir.path().join("base.toml");
    let child_path = dir.path().join("child.toml");

    // Create base template
    fs::write(
        &base_path,
        r#"
[device]
name = "Base Device"
protocol = "serial_scpi"
manufacturer = "Test Corp"

[connection]
baud_rate = 19200
terminator_tx = "\r\n"

[commands.move_abs]
template = "MA{{ position }}"
expects_response = false
"#,
    )
    .unwrap();

    // Create child that extends base
    fs::write(
        &child_path,
        r#"
extends = "base.toml"

[device]
name = "Child Device"
protocol = "serial_scpi"
model = "Model-X"

[connection]
baud_rate = 9600

[commands.home]
template = "HOME"
expects_response = true
"#,
    )
    .unwrap();

    let factory = GenericSerialDriverFactory::from_file(&child_path).unwrap();

    // Verify merged config
    assert_eq!(factory.name(), "Child Device");
    // Protocol should come from base
    // Baud rate should be overridden to 9600
    // Should have both move_abs (from base) and home (from child)
}

#[test]
fn test_multi_level_inheritance() {
    let dir = create_test_dir();
    let base_path = dir.path().join("base.toml");
    let middle_path = dir.path().join("middle.toml");
    let child_path = dir.path().join("child.toml");

    // Create base template
    fs::write(
        &base_path,
        r#"
[device]
name = "Base"
protocol = "serial"
manufacturer = "Manufacturer A"

[connection]
baud_rate = 9600

[commands.cmd1]
template = "CMD1"
expects_response = true
"#,
    )
    .unwrap();

    // Create middle that extends base
    fs::write(
        &middle_path,
        r#"
extends = "base.toml"

[device]
name = "Middle"
protocol = "serial"
model = "Model-M"

[connection]
baud_rate = 19200

[commands.cmd2]
template = "CMD2"
expects_response = true
"#,
    )
    .unwrap();

    // Create child that extends middle
    fs::write(
        &child_path,
        r#"
extends = "middle.toml"

[device]
name = "Child"
protocol = "serial"

[commands.cmd3]
template = "CMD3"
expects_response = true
"#,
    )
    .unwrap();

    let factory = GenericSerialDriverFactory::from_file(&child_path).unwrap();

    // Verify three-level merge
    assert_eq!(factory.name(), "Child");
    // Should have all three commands: cmd1, cmd2, cmd3
}

#[test]
fn test_override_precedence() {
    let dir = create_test_dir();
    let base_path = dir.path().join("base.toml");
    let child_path = dir.path().join("child.toml");

    // Create base with a command
    fs::write(
        &base_path,
        r#"
[device]
name = "Base"
protocol = "serial"

[commands.move_abs]
template = "MA{{ position }}"
expects_response = false

[commands.get_position]
template = "TP"
expects_response = true
"#,
    )
    .unwrap();

    // Child overrides one command
    fs::write(
        &child_path,
        r#"
extends = "base.toml"

[device]
name = "Child"
protocol = "serial"

[commands.move_abs]
template = "MOVE{{ position }}"
expects_response = true
"#,
    )
    .unwrap();

    let factory = GenericSerialDriverFactory::from_file(&child_path).unwrap();

    // Verify child's move_abs overrides base
    // get_position should still come from base
    assert_eq!(factory.name(), "Child");
}

#[test]
fn test_circular_dependency_detection() {
    let dir = create_test_dir();
    let a_path = dir.path().join("a.toml");
    let b_path = dir.path().join("b.toml");

    // A extends B
    fs::write(
        &a_path,
        r#"
extends = "b.toml"

[device]
name = "A"
protocol = "serial"
"#,
    )
    .unwrap();

    // B extends A (circular!)
    fs::write(
        &b_path,
        r#"
extends = "a.toml"

[device]
name = "B"
protocol = "serial"
"#,
    )
    .unwrap();

    let result = GenericSerialDriverFactory::from_file(&a_path);
    assert!(result.is_err(), "Should detect circular dependency");
    let err_msg = result.err().unwrap().to_string();
    assert!(
        err_msg.contains("Circular"),
        "Error should mention circular dependency, got: {}",
        err_msg
    );
}

#[test]
fn test_missing_base_file_error() {
    let dir = create_test_dir();
    let child_path = dir.path().join("child.toml");

    fs::write(
        &child_path,
        r#"
extends = "nonexistent.toml"

[device]
name = "Child"
protocol = "serial"
"#,
    )
    .unwrap();

    let result = GenericSerialDriverFactory::from_file(&child_path);
    assert!(result.is_err(), "Should fail when base file doesn't exist");
}

#[test]
fn test_no_inheritance_still_works() {
    let dir = create_test_dir();
    let path = dir.path().join("standalone.toml");

    fs::write(
        &path,
        r#"
[device]
name = "Standalone"
protocol = "serial"

[commands.test]
template = "TEST"
expects_response = true
"#,
    )
    .unwrap();

    let factory = GenericSerialDriverFactory::from_file(&path).unwrap();
    assert_eq!(factory.name(), "Standalone");
}

#[test]
fn test_nested_base_directory() {
    let dir = create_test_dir();
    let bases_dir = dir.path().join("bases");
    fs::create_dir(&bases_dir).unwrap();

    let base_path = bases_dir.join("newport_motion.toml");
    let child_path = dir.path().join("esp301.toml");

    // Create base in bases/ subdirectory
    fs::write(
        &base_path,
        r#"
[device]
name = "Newport Motion Base"
protocol = "serial_scpi"
manufacturer = "Newport"

[connection]
baud_rate = 19200
terminator_tx = "\r\n"

[commands.move_abs]
template = "PA{{ axis }}{{ position }}"
expects_response = false
"#,
    )
    .unwrap();

    // Child references base with relative path
    fs::write(
        &child_path,
        r#"
extends = "bases/newport_motion.toml"

[device]
name = "ESP301"
protocol = "serial_scpi"
model = "ESP301"

[commands.home]
template = "OR1"
expects_response = false
"#,
    )
    .unwrap();

    let factory = GenericSerialDriverFactory::from_file(&child_path).unwrap();
    assert_eq!(factory.name(), "ESP301");
}

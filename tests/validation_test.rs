use rust_daq::validation::*;

#[test]
fn test_is_valid_port() {
    assert!(is_valid_port(8080).is_ok());
    assert!(is_valid_port(0).is_err());
}

#[test]
fn test_is_valid_ip() {
    assert!(is_valid_ip("127.0.0.1").is_ok());
    assert!(is_valid_ip("256.0.0.1").is_err());
    assert!(is_valid_ip("not an ip").is_err());
}

#[test]
fn test_is_valid_path() {
    assert!(is_valid_path("/some/path").is_ok());
    assert!(is_valid_path("").is_err());
    assert!(is_valid_path("path/with\0/null.txt").is_err());
}

#[test]
fn test_is_in_range() {
    assert!(is_in_range(5, 1..=10).is_ok());
    assert!(is_in_range(11, 1..=10).is_err());
}

#[test]
fn test_is_not_empty() {
    assert!(is_not_empty("hello").is_ok());
    assert!(is_not_empty("").is_err());
}
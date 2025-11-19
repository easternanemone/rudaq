use rust_daq::scripting::ScriptHost;
use tokio::runtime::Handle;

#[tokio::test]
async fn test_simple_script() {
    let host = ScriptHost::new(Handle::current());
    let result = host.run_script("5 + 5").unwrap();
    assert_eq!(result.as_int().unwrap(), 10);
}

#[tokio::test]
async fn test_safety_limit() {
    let host = ScriptHost::new(Handle::current());
    let infinite_loop = "loop { }";
    let result = host.run_script(infinite_loop);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    // The error contains "Script terminated" with our safety message in the debug output
    assert!(err_msg.contains("Script terminated") || err_msg.contains("Safety limit exceeded"));
}

#[tokio::test]
async fn test_script_validation() {
    let host = ScriptHost::new(Handle::current());

    // Valid script
    assert!(host.validate_script("let x = 10;").is_ok());

    // Invalid syntax
    assert!(host.validate_script("let x = ;").is_err());
}

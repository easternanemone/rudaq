use daq_core::observable::Observable;
use daq_core::parameter::Parameter;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Duration;

// bd-ga2o: Use current_thread flavor since daq-core doesn't enable rt-multi-thread
#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("--- Test 1: NaN Initial Value ---");
    // Test if we can create an observable with NaN and then apply range constraints
    let obs = Observable::new("test_nan", f64::NAN);
    // This should probably not panic, but let's see if the validator checks the current value
    let obs = obs.with_range_introspectable(0.0, 10.0);
    println!("Observable created with NaN: {}", obs.get());
    
    // Try to set it to a valid value
    match obs.set(5.0) {
        Ok(_) => println!("Set to 5.0 succeeded"),
        Err(e) => println!("Set to 5.0 failed: {}", e),
    }

    // Try to set it back to NaN
    match obs.set(f64::NAN) {
        Ok(_) => println!("Set to NaN succeeded"),
        Err(e) => println!("Set to NaN failed: {}", e),
    }

    println!("\n--- Test 2: Parameter Set Race Condition ---");
    // Simulate a slow hardware write to expose race conditions
    let hw_state = Arc::new(Mutex::new(Vec::new()));
    let hw_state_clone = hw_state.clone();

    let mut param = Parameter::new("race_param", 0);
    param.connect_to_hardware_write(move |val| {
        let hw_state = hw_state_clone.clone();
        Box::pin(async move {
            // Simulate delay
            tokio::time::sleep(Duration::from_millis(10)).await;
            hw_state.lock().await.push(val);
            Ok(())
        })
    });

    let param = Arc::new(param);
    let p1 = param.clone();
    let p2 = param.clone();

    // Spawn two tasks that set different values
    let t1 = tokio::spawn(async move {
        println!("T1 setting 1");
        p1.set(1).await.unwrap();
        println!("T1 done");
    });
    
    let t2 = tokio::spawn(async move {
        println!("T2 setting 2");
        p2.set(2).await.unwrap();
        println!("T2 done");
    });

    let _ = tokio::join!(t1, t2);

    println!("Final Parameter Value: {}", param.get());
    println!("Hardware Write History: {:?}", *hw_state.lock().await);

    // If history is [2, 1] but final value is 2, then we have:
    // T2 start (fast?), T1 start.
    // T2 write 2. T1 write 1.
    // Final hardware state is 1 (last write).
    // T2 update observable to 2. T1 update observable to 1.
    // If T2 finishes last in updating observable, observable is 2.
    // If T1 finishes last in updating observable, observable is 1.
    
    // We want hardware state and observable state to match the "last" command.
    // But "last" is ambiguous without serialization.
}

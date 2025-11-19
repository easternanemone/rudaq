// Camera triggered acquisition
print("Setting up acquisition...");

camera.arm();
print("Camera armed");

for i in 0..5 {
    let pos = i * 2.0;
    stage.move_abs(pos);
    stage.wait_settled();
    camera.trigger();
    print(`Frame ${i+1} captured at ${pos}mm`);
}

print("Acquisition complete!");

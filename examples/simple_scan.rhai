// Simple stage scan experiment
print("Starting scan...");

for i in 0..10 {
    let pos = i * 1.0;
    stage.move_abs(pos);
    print(`Moved to ${pos}mm`);
    sleep(0.1);
}

print("Scan complete!");

// Script with intentional error to test error handling
print("Testing error handling...");

// This will fail - camera not armed before trigger
camera.trigger();

print("This should not be reached");

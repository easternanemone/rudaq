use std::process::Command;

fn main() {
    // Get git commit SHA
    if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output() {
        if output.status.success() {
            if let Ok(sha) = String::from_utf8(output.stdout) {
                println!("cargo:rustc-env=VERGEN_GIT_SHA={}", sha.trim());
            }
        }
    } else {
        // Fallback if git not available
        println!("cargo:rustc-env=VERGEN_GIT_SHA=unknown");
    }

    // Check if working directory is dirty
    if let Ok(output) = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
    {
        if output.status.success() {
            let is_dirty = !output.stdout.is_empty();
            println!("cargo:rustc-env=VERGEN_GIT_DIRTY={}", is_dirty);
        }
    } else {
        println!("cargo:rustc-env=VERGEN_GIT_DIRTY=false");
    }

    // Get git commit date
    if let Ok(output) = Command::new("git")
        .args(["log", "-1", "--format=%cI"])
        .output()
    {
        if output.status.success() {
            if let Ok(date) = String::from_utf8(output.stdout) {
                println!("cargo:rustc-env=VERGEN_GIT_COMMIT_DATE={}", date.trim());
            }
        }
    }

    // Get build timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    println!("cargo:rustc-env=VERGEN_BUILD_TIMESTAMP={}", now);

    // Rerun if .git/HEAD or .git/index changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}

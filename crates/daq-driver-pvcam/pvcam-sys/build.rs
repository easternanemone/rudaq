use std::env;
use std::path::PathBuf;

// These helper functions are only used when pvcam-sdk feature is enabled
#[allow(dead_code)]
/// Print a boxed error message for visibility in cargo output
fn print_env_error(title: &str, details: &[&str], fixes: &[&str]) {
    eprintln!();
    eprintln!("╔══════════════════════════════════════════════════════════════════╗");
    eprintln!("║ PVCAM BUILD ERROR: {:<46} ║", title);
    eprintln!("╠══════════════════════════════════════════════════════════════════╣");
    for detail in details {
        eprintln!("║ {:<66} ║", detail);
    }
    eprintln!("╠══════════════════════════════════════════════════════════════════╣");
    eprintln!("║ HOW TO FIX:                                                      ║");
    for fix in fixes {
        eprintln!("║   {:<64} ║", fix);
    }
    eprintln!("╠══════════════════════════════════════════════════════════════════╣");
    eprintln!("║ Quick setup on maitai:                                           ║");
    eprintln!("║   source scripts/env-check.sh                                    ║");
    eprintln!("╚══════════════════════════════════════════════════════════════════╝");
    eprintln!();
}

#[allow(dead_code)]
/// Check for common PVCAM installation paths
fn find_pvcam_sdk() -> Option<PathBuf> {
    let candidates = [
        "/opt/pvcam/sdk",
        "/usr/local/pvcam/sdk",
        "/opt/photometrics/pvcam/sdk",
    ];

    for path in &candidates {
        let p = PathBuf::from(path);
        if p.join("include").exists() {
            return Some(p);
        }
    }
    None
}

#[allow(dead_code)]
/// Check for PVCAM library paths
fn find_pvcam_lib() -> Option<PathBuf> {
    let candidates = [
        "/opt/pvcam/library/x86_64",
        "/opt/pvcam/lib",
        "/usr/local/lib",
        "/usr/lib/x86_64-linux-gnu",
    ];

    for path in &candidates {
        let p = PathBuf::from(path);
        if p.join("libpvcam.so").exists() {
            return Some(p);
        }
    }
    None
}

#[allow(dead_code)]
/// Print diagnostic information about the environment
fn print_env_diagnostics() {
    eprintln!();
    eprintln!("=== PVCAM Build Diagnostics ===");
    eprintln!("PVCAM_SDK_DIR: {:?}", env::var("PVCAM_SDK_DIR").ok());
    eprintln!("PVCAM_LIB_DIR: {:?}", env::var("PVCAM_LIB_DIR").ok());
    eprintln!("LIBRARY_PATH: {:?}", env::var("LIBRARY_PATH").ok());
    eprintln!("LD_LIBRARY_PATH: {:?}", env::var("LD_LIBRARY_PATH").ok());

    if let Some(found) = find_pvcam_sdk() {
        eprintln!("Auto-detected SDK at: {:?}", found);
    }
    if let Some(found) = find_pvcam_lib() {
        eprintln!("Auto-detected lib at: {:?}", found);
    }
    eprintln!("===============================");
    eprintln!();
}

fn main() {
    // Only run bindgen and linking logic if the `pvcam-sdk` feature is enabled.
    // This allows the crate to compile without the SDK if the feature is not active.
    #[cfg(feature = "pvcam-sdk")]
    {
        println!("cargo:rerun-if-env-changed=PVCAM_SDK_DIR");
        println!("cargo:rerun-if-env-changed=PVCAM_LIB_DIR");
        println!("cargo:rerun-if-env-changed=LIBRARY_PATH");
        println!("cargo:rerun-if-changed=wrapper.h"); // For bindgen to re-run if wrapper changes

        // Try to get SDK directory from environment, with helpful auto-detection
        let sdk_dir = match env::var("PVCAM_SDK_DIR") {
            Ok(dir) => PathBuf::from(dir),
            Err(_) => {
                // Try auto-detection before failing
                if let Some(found) = find_pvcam_sdk() {
                    println!(
                        "cargo:warning=PVCAM_SDK_DIR not set, auto-detected: {}",
                        found.display()
                    );
                    found
                } else {
                    print_env_diagnostics();
                    print_env_error(
                        "PVCAM_SDK_DIR not set",
                        &[
                            "The pvcam-sdk feature requires the PVCAM SDK.",
                            "This environment variable tells the build where to find headers.",
                        ],
                        &[
                            "export PVCAM_SDK_DIR=/opt/pvcam/sdk",
                            "Or run: source scripts/env-check.sh",
                        ],
                    );
                    panic!("PVCAM_SDK_DIR environment variable must be set when `pvcam-sdk` feature is enabled.");
                }
            }
        };

        let sdk_include_path = sdk_dir.join("include");

        // Allow PVCAM_LIB_DIR to override the default lib path
        let sdk_lib_path = match env::var("PVCAM_LIB_DIR") {
            Ok(lib_dir) => PathBuf::from(lib_dir),
            Err(_) => {
                // Try common locations
                if let Some(found) = find_pvcam_lib() {
                    println!(
                        "cargo:warning=PVCAM_LIB_DIR not set, auto-detected: {}",
                        found.display()
                    );
                    found
                } else {
                    // Fall back to SDK default
                    sdk_dir.join("lib")
                }
            }
        };

        if !sdk_include_path.exists() {
            print_env_diagnostics();
            print_env_error(
                "SDK include path not found",
                &[
                    &format!("Expected headers at: {}", sdk_include_path.display()),
                    "The master.h header file is required for bindgen.",
                ],
                &[
                    "Verify PVCAM SDK is installed: ls /opt/pvcam/sdk/include",
                    "Set correct path: export PVCAM_SDK_DIR=/path/to/sdk",
                ],
            );
            panic!(
                "PVCAM SDK include path does not exist: {:?}",
                sdk_include_path
            );
        }

        // The lib path might not exist if libraries are installed globally,
        // but it's a common place. Warn rather than panic.
        if !sdk_lib_path.exists() {
            println!(
                "cargo:warning=PVCAM SDK lib path does not exist: {}",
                sdk_lib_path.display()
            );
            println!("cargo:warning=Linker will search LIBRARY_PATH and standard paths");
        }

        // Generate bindings
        let bindings = bindgen::Builder::default()
            // The input header we would like to generate bindings for.
            .header("wrapper.h")
            // Tell cargo to invalidate the built crate whenever any of the
            // included header files changed.
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            // Add include path for PVCAM headers
            .clang_arg(format!("-I{}", sdk_include_path.display()))
            // Allowlist functions starting with `pl_`
            .allowlist_function("pl_.*")
            // Allowlist types used by PVCAM. Bindgen often pulls in types if they are
            // part of an allowlisted function's signature, but explicit allowlisting
            // is safer for constants and standalone types.
            .allowlist_type("rs_bool")
            .allowlist_type("uns8|uns16|uns32|uns64") // Common PVCAM integer types
            .allowlist_type("int8|int16|int32|int64") // Common PVCAM integer types
            .allowlist_type("flt32|flt64") // Common PVCAM float types
            .allowlist_type("char_ptr") // If char_ptr is a typedef
            .allowlist_type("PV_.*") // General allowlist for PVCAM specific types (e.g., PV_ERROR, PV_CAMERA_TYPE)
            .allowlist_type("pvc_.*") // Some types might start with pvc_
            // Convert PARAM_* constants to a Rust enum for type safety.
            // Bindgen will attempt to group related constants into an enum.
            .constified_enum("PARAM_.*")
            .default_enum_style(bindgen::EnumVariation::Rust {
                non_exhaustive: false,
            })
            // Allowlist additional types and variables
            .allowlist_type("rgn_type")
            .allowlist_var("PARAM_.*")
            .allowlist_var("ATTR_.*")
            .allowlist_var("TIMED_MODE")
            .allowlist_var("READOUT_.*")
            // Finish the builder and generate the bindings.
            .generate()
            .expect("Unable to generate bindings");

        // Write the bindings to the $OUT_DIR/bindings.rs file.
        let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
        bindings
            .write_to_file(out_path.join("bindings.rs"))
            .expect("Couldn't write bindings!");

        // Link to the PVCAM library
        println!("cargo:rustc-link-search=native={}", sdk_lib_path.display());

        #[cfg(target_os = "windows")]
        {
            println!("cargo:rustc-link-lib=pvcam64");
        }
        #[cfg(target_os = "macos")]
        {
            println!("cargo:rustc-link-lib=pvcam"); // Assuming libpvcam.dylib
        }
        #[cfg(target_os = "linux")]
        {
            println!("cargo:rustc-link-lib=pvcam"); // Assuming libpvcam.so
        }
    }
    #[cfg(not(feature = "pvcam-sdk"))]
    {
        // If the pvcam-sdk feature is not enabled, create a dummy bindings file
        // to allow src/lib.rs to compile without actual SDK presence.
        let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
        std::fs::write(
            out_path.join("bindings.rs"),
            "// Dummy bindings when pvcam-sdk feature is not enabled\npub mod pvcam_bindings {}\n",
        )
        .expect("Couldn't write dummy bindings!");
    }
}

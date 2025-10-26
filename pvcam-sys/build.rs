use std::env;
use std::path::PathBuf;

fn main() {
    // Only run bindgen and linking logic if the `pvcam-sdk` feature is enabled.
    // This allows the crate to compile without the SDK if the feature is not active.
    #[cfg(feature = "pvcam-sdk")]
    {
        println!("cargo:rerun-if-env-changed=PVCAM_SDK_DIR");
        println!("cargo:rerun-if-changed=wrapper.h"); // For bindgen to re-run if wrapper changes

        let sdk_dir = env::var("PVCAM_SDK_DIR")
            .expect("PVCAM_SDK_DIR environment variable must be set when `pvcam-sdk` feature is enabled.");

        let sdk_include_path = PathBuf::from(&sdk_dir).join("include");
        let sdk_lib_path = PathBuf::from(&sdk_dir).join("lib");

        if !sdk_include_path.exists() {
            panic!("PVCAM SDK include path does not exist: {:?}", sdk_include_path);
        }
        // The lib path might not exist if libraries are installed globally,
        // but it's a common place. Warn rather than panic.
        if !sdk_lib_path.exists() {
            eprintln!("Warning: PVCAM SDK lib path does not exist: {:?}", sdk_lib_path);
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
            .default_enum_style(bindgen::EnumVariation::Rust { non_exhaustive: false })
            // Map `rs_bool` to Rust's `bool`. This assumes rs_bool is 0 for false, non-zero for true.
            .type_alias("rs_bool", "bool")
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

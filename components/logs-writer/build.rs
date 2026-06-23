use std::{env, process::Command};

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let is_test_build = out_dir.contains("debug");

    if is_test_build {
        println!("cargo:warning=Building WASM component for tests…");
        println!("cargo::rerun-if-changed=test-component/src/lib.rs");
        println!("cargo::rerun-if-changed=src/lib.rs");

        let status = Command::new("cargo")
            .args(["build", "--target", "wasm32-wasip2", "--release"])
            .status()
            .expect("Failed to compile WASM component");
        assert!(status.success(), "WASM component build failed");

        let status = Command::new("cargo")
            .args([
                "build",
                "--manifest-path",
                "test-component/Cargo.toml",
                "--target",
                "wasm32-wasip2",
                "--release",
            ])
            .status()
            .expect("Failed to compile WASM component");
        assert!(status.success(), "WASM component build failed");
    } else {
        println!("cargo:warning=Skipping WASM build (not a test build)");
    }
}

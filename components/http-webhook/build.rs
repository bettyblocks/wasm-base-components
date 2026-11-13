use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let is_test_build = out_dir.contains("debug");

    if !is_test_build {
        println!("cargo:warning=Skipping WASM build (not a test build)");
        return;
    }

    if std::env::var("SKIP_WASM_BUILD").is_ok() {
        println!("cargo:warning=SKIP_WASM_BUILD set – skipping fixture builds");
        return;
    }

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let fixtures_dir = manifest_dir.join("tests/fixtures");

    std::fs::create_dir_all(&fixtures_dir).expect("Failed to create tests/fixtures directory");

    println!("cargo:warning=Building webhook-component for wasm32-wasip2...");
    let status = Command::new("cargo")
        .args(["build", "--target", "wasm32-wasip2", "--release"])
        .status()
        .expect("Failed to compile webhook-component");
    assert!(status.success(), "webhook-component WASM build failed");

    let src = manifest_dir.join("target/wasm32-wasip2/release/deps/webhook_component.wasm");
    let dest = fixtures_dir.join("webhook_component.wasm");
    std::fs::copy(&src, &dest).unwrap_or_else(|e| {
        panic!(
            "Failed to copy {} -> {}: {}",
            src.display(),
            dest.display(),
            e
        )
    });
    println!(
        "cargo:warning=Copied {} -> {}",
        src.display(),
        dest.display()
    );

    println!("cargo::rerun-if-changed=src/");
    println!("cargo::rerun-if-changed=../../crates/action-client/src/");
    println!("cargo::rerun-if-changed=wasmcloud.toml");
    println!("cargo::rerun-if-env-changed=SKIP_WASM_BUILD");
}

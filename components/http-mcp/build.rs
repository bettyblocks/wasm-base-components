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
    let auth_dir = manifest_dir.join("../utilities/auth");
    let fixtures_dir = manifest_dir.join("tests/fixtures");

    std::fs::create_dir_all(&fixtures_dir).expect("Failed to create tests/fixtures directory");

    println!("cargo:warning=Building mcp-component for wasm32-wasip2...");
    let status = Command::new("cargo")
        .args(["build", "--target", "wasm32-wasip2", "--release"])
        .status()
        .expect("Failed to compile mcp-component");
    assert!(status.success(), "mcp-component WASM build failed");

    println!("cargo:warning=Building jwt-auth-component for wasm32-wasip2...");
    let status = Command::new("cargo")
        .args([
            "build",
            "--manifest-path",
            "../utilities/auth/Cargo.toml",
            "--target",
            "wasm32-wasip2",
            "--release",
        ])
        .status()
        .expect("Failed to compile jwt-auth-component");
    assert!(status.success(), "jwt-auth-component WASM build failed");

    let artifacts: &[(&std::path::Path, &str, &str)] = &[
        (
            manifest_dir.as_path(),
            "mcp_component.wasm",
            "betty_mcp_component.wasm",
        ),
        (
            auth_dir.as_path(),
            "jwt_auth_component.wasm",
            "jwt_auth_component.wasm",
        ),
    ];
    for (crate_dir, src_name, dest_name) in artifacts {
        let src = crate_dir
            .join("target/wasm32-wasip2/release/deps")
            .join(src_name);
        let dest = fixtures_dir.join(dest_name);
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
    }

    println!("cargo::rerun-if-changed=src/");
    println!("cargo::rerun-if-changed=wasmcloud.toml");
    println!("cargo::rerun-if-changed=../utilities/auth/src/");
    println!("cargo::rerun-if-changed=../utilities/auth/wasmcloud.toml");
    println!("cargo::rerun-if-env-changed=SKIP_WASM_BUILD");
}

fn main() {
    // env!("TARGET") is not set for normal crates, but a build script gets it.
    // Without this, BuildInfo can only report the bare arch ("aarch64"), which
    // cannot distinguish a macOS build from any other aarch64 build in a bug report.
    println!(
        "cargo:rustc-env=M2M_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".into())
    );
    // The rig `.glb` embedding moved to `m2m-pipeline`'s build script along with
    // the pipeline itself.
    tauri_build::build()
}

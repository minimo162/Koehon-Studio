fn main() {
    prepare_sidecar();
    tauri_build::build()
}

fn prepare_sidecar() {
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let target = std::env::var("TARGET").unwrap_or_default();
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let exe = std::env::consts::EXE_SUFFIX;
    let source = manifest_dir
        .join("../native-tts/target")
        .join(&profile)
        .join(format!("koehon-tts-sidecar{exe}"));
    let sidecar_dir = manifest_dir.join("../native-tts/sidecars");
    let target_path = sidecar_dir.join(format!("koehon-tts-sidecar-{target}{exe}"));

    println!("cargo:rerun-if-changed={}", source.display());
    if source.exists() {
        if let Err(error) = std::fs::create_dir_all(&sidecar_dir) {
            println!(
                "cargo:warning=failed to create sidecar dir {}: {error}",
                sidecar_dir.display()
            );
            return;
        }
        if let Err(error) = std::fs::copy(&source, &target_path) {
            println!(
                "cargo:warning=failed to prepare sidecar {}: {error}",
                target_path.display()
            );
        }
    }
}

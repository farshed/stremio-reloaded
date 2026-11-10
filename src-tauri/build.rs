use std::process::Command;

fn main() {
    link_libmpv();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-headerpad_max_install_names");
    }
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&["shell_props", "shell_event"]),
        ),
    )
    .expect("failed to run tauri-build");
}

fn link_libmpv() {
    println!("cargo:rerun-if-env-changed=MPV_LIB_DIR");
    if let Ok(dir) = std::env::var("MPV_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        return;
    }
    let Ok(output) = Command::new("pkg-config").args(["--libs-only-L", "mpv"]).output() else {
        return;
    };
    for dir in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        if let Some(dir) = dir.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=COOKBOOK_UPDATE_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=COOKBOOK_UPDATE_ENDPOINT");
    println!(
        "cargo:rustc-env=APR_TARGET_TRIPLE={}",
        std::env::var("TARGET").unwrap()
    );
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "desktop_status",
            "scoped_call",
            "open_setup",
            "owner_request",
            "oauth_begin",
            "oauth_discover_begin",
            "oauth_status",
            "oauth_cancel",
            "update_check",
            "update_install",
            "app_preview_open",
        ]),
    ))
    .expect("Tauri desktop build failed");
}

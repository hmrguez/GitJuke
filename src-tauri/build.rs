fn main() {
    println!("cargo:rerun-if-changed=src/notifications.m");
    cc::Build::new()
        .file("src/notifications.m")
        .flag("-fobjc-arc")
        .flag("-fblocks")
        .compile("gitjuke_notifications");
    println!("cargo:rustc-link-lib=framework=UserNotifications");
    println!("cargo:rustc-link-lib=framework=Foundation");
    tauri_build::build()
}

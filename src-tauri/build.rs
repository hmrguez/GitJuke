mod build_metadata;

fn main() {
    let root = std::path::Path::new("..");
    // Watch source directories as well as Git pointers: an unstaged edit must not
    // leave a cached clean stamp in a newly compiled app. Never watch target/.
    for path in [
        "../src",
        "../public",
        "../scripts",
        "../crates",
        "../Cargo.toml",
        "../Cargo.lock",
        "../package.json",
        "../bun.lock",
        "../index.html",
        "../tsconfig.json",
        "src",
        "icons",
        "capabilities",
        "tauri.conf.json",
        "Cargo.toml",
        "build.rs",
        "build_metadata.rs",
    ] {
        if std::path::Path::new(path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    for name in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = build_metadata::git(root, &["rev-parse", "--git-path", name]) {
            let path = root.join(path.trim());
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    if let Some(reference) = build_metadata::git(root, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) =
            build_metadata::git(root, &["rev-parse", "--git-path", reference.trim()])
    {
        let path = root.join(path.trim());
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    println!("cargo:rerun-if-env-changed=GITJUKE_BUILD_ID");
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("build clock precedes Unix epoch")
        .as_secs();
    let override_id = std::env::var_os("GITJUKE_BUILD_ID")
        .map(|value| value.into_string().expect("GITJUKE_BUILD_ID must be ASCII"));
    let id =
        build_metadata::build_id(override_id.as_deref(), seconds).expect("invalid build identity");
    let (revision, state) = build_metadata::source(root);
    for (key, value) in [
        ("ID", id),
        ("REVISION", revision),
        ("SOURCE_STATE", state.into()),
        ("TARGET", std::env::var("TARGET").unwrap()),
        ("PROFILE", std::env::var("PROFILE").unwrap()),
    ] {
        println!("cargo:rustc-env=GITJUKE_BUILD_{key}={value}");
    }
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

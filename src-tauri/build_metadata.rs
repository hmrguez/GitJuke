//! Build-time source identity only. Never query Git or the environment at runtime.
use std::{path::Path, process::Command};

pub fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

pub fn source(root: &Path) -> (String, &'static str) {
    let repository = git(root, &["rev-parse", "--show-toplevel"])
        .and_then(|path| Path::new(path.trim()).canonicalize().ok());
    if repository.is_none() || repository != root.canonicalize().ok() {
        return ("unavailable".into(), "unavailable");
    }
    let revision = git(root, &["rev-parse", "--verify", "HEAD"])
        .map(|s| s.trim().to_owned())
        .filter(|s| [40, 64].contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()));
    let state = git(root, &["status", "--porcelain", "--untracked-files=normal"]);
    match (revision, state) {
        (Some(revision), Some(state)) => (
            revision,
            if state.is_empty() {
                "clean"
            } else {
                "modified"
            },
        ),
        _ => ("unavailable".into(), "unavailable"),
    }
}

pub fn build_id(override_id: Option<&str>, seconds: u64) -> Result<String, &'static str> {
    match override_id {
        Some(id)
            if !id.is_empty()
                && id.len() <= 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)) =>
        {
            Ok(id.into())
        }
        Some(_) => Err(
            "GITJUKE_BUILD_ID must contain 1–64 ASCII letters, digits, dots, underscores or hyphens",
        ),
        None => Ok(format!("local-{seconds}")),
    }
}

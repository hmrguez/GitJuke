use gitjuke_github::transport::{Limits, ReadOp, Transport};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tempfile::{TempDir, tempdir};
use tokio_util::sync::CancellationToken;
fn fake(body: &str, limits: Limits) -> (TempDir, Transport) {
    let d = tempdir().unwrap();
    let p = d.path().join("gh");
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
    let t = Transport::new(p, d.path().join("config"), d.path().into(), limits).unwrap();
    (d, t)
}
fn limits() -> Limits {
    Limits {
        stdout: 128,
        stderr: 64,
        deadline: Duration::from_secs(2),
        grace: Duration::from_millis(40),
    }
}
#[tokio::test]
async fn inclusive_bounds_and_stdin_closed() {
    let (_d, t) = fake("cat >/dev/null\n/usr/bin/printf '%0128d' 0", limits());
    let o = t
        .read(ReadOp::Version, &CancellationToken::new())
        .await
        .unwrap();
    assert!(o.success);
    assert_eq!(o.stdout.len(), 128);
}
#[tokio::test]
async fn stdout_and_stderr_overflow_rejected() {
    for body in [
        "/usr/bin/printf '%0129d' 0",
        "/usr/bin/printf '%065d' 0 >&2",
    ] {
        let (_d, t) = fake(body, limits());
        assert!(
            t.read(ReadOp::Version, &CancellationToken::new())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("output limit")
        );
    }
}
#[tokio::test]
async fn timeout_kills_helpers_and_next_child_can_start() {
    let (d, t) = fake("/bin/sleep 20 &\necho $! > \"$HOME/child\"\nwait", limits());
    let started = std::time::Instant::now();
    assert!(
        t.read(ReadOp::Version, &CancellationToken::new())
            .await
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(4));
    let pid = std::fs::read_to_string(d.path().join("child"))
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "helper must be gone");
    assert!(
        t.read(ReadOp::Version, &CancellationToken::new())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn cancellation_reaps_before_serialized_successor() {
    let (d, t) = fake("echo $$ > \"$HOME/parent\"\n/bin/sleep 20", limits());
    let token = CancellationToken::new();
    let tc = token.clone();
    let transport = t.clone();
    let task = tokio::spawn(async move { transport.read(ReadOp::Version, &tc).await });
    let pid = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            // Creation precedes writing: wait for a complete PID before
            // cancelling the shell that publishes it.
            if let Some(pid) = std::fs::read_to_string(d.path().join("parent"))
                .ok()
                .and_then(|s| s.strip_suffix('\n')?.parse::<i32>().ok())
            {
                break pid;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fake gh must start");
    token.cancel();
    assert!(task.await.unwrap().is_err());
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}
#[tokio::test]
async fn deliberate_environment_no_tokens_or_injection() {
    let (_d, t) = fake(
        "test -z \"$GH_TOKEN$GITHUB_TOKEN$GH_HOST$GH_REPO$DYLD_INSERT_LIBRARIES$NODE_OPTIONS\" || exit 2\ntest \"$GH_PROMPT_DISABLED\" = 1 || exit 3\ntest \"$PATH\" = /usr/bin:/bin:/usr/sbin:/sbin || exit 4\nprintf safe",
        limits(),
    );
    let o = t
        .read(ReadOp::Identity, &CancellationToken::new())
        .await
        .unwrap();
    assert!(o.success);
    assert_eq!(o.stdout, b"safe");
}

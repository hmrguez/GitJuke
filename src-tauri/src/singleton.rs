//! Atomic ownership precedes SQLite/runtime initialization. Fail closed on forwarding errors.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub struct Owner {
    _lock: File,
    pub listener: UnixListener,
    socket: PathBuf,
}
impl Drop for Owner {
    fn drop(&mut self) {
        // Cleanup while the ownership lock is still held.
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_dir(self.socket.parent().unwrap());
    }
}
fn socket_path(directory: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let hash = directory
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
    let uid = unsafe { libc::getuid() };
    let parent = PathBuf::from(format!("/private/tmp/gitjuke-{uid}-{hash:016x}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&parent) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(&parent)?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(std::io::Error::other(
            "Unsafe single-instance socket directory; no monitor was started.",
        ));
    }
    Ok(parent.join("instance.sock"))
}
pub enum Instance {
    Owner(Owner),
    Forwarded,
}
/// One owner per OS account + installed application identity, independent of HOME/config overrides.
/// Reuse the private short namespace validator; the lock inode remains persistent.
pub fn directory_for(identifier: &str) -> std::io::Result<PathBuf> {
    Ok(socket_path(Path::new(identifier))?
        .parent()
        .unwrap()
        .to_path_buf())
}
pub fn acquire(directory: &Path, background: bool) -> std::io::Result<Instance> {
    std::fs::create_dir_all(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("instance.lock"))?;
    let socket = socket_path(directory)?;
    // flock is released by the kernel on exit/crash; never unlink a live lock inode.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::WouldBlock {
            return Err(error);
        }
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(mut stream) = UnixStream::connect(&socket) {
                stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                stream.set_write_timeout(Some(Duration::from_secs(1)))?;
                stream.write_all(&[u8::from(background)])?;
                let mut ack = [0];
                stream.read_exact(&mut ack)?;
                if ack == [1] {
                    return Ok(Instance::Forwarded);
                }
                return Err(std::io::Error::other(
                    "Existing GitJuke could not accept activation. Quit it explicitly and retry.",
                ));
            }
            if Instant::now() >= until {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "GitJuke is starting or quitting. Retry after it finishes; no second monitor was started.",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    match std::fs::remove_file(&socket) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(&socket)?;
    Ok(Instance::Owner(Owner {
        _lock: lock,
        listener,
        socket,
    }))
}
pub fn receive(mut stream: UnixStream) -> std::io::Result<bool> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut mode = [0];
    stream.read_exact(&mut mode)?;
    if mode[0] > 1 {
        return Err(std::io::Error::other("Invalid activation"));
    }
    stream.write_all(&[1])?;
    Ok(mode[0] == 1)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simultaneous_launches_forward_and_crash_releases_ownership() {
        let dir =
            std::env::temp_dir().join(format!("gitjuke-instance-test-{}", std::process::id()));
        let barrier = std::sync::Barrier::new(16);
        std::thread::scope(|scope| {
            let contenders: Vec<_> = (0..16)
                .map(|n| {
                    let d = &dir;
                    let start = &barrier;
                    scope.spawn(move || {
                        start.wait();
                        match acquire(d, n % 2 == 0).unwrap() {
                            Instance::Forwarded => (0, 0),
                            Instance::Owner(owner) => {
                                owner.listener.set_nonblocking(true).unwrap();
                                let until = Instant::now() + Duration::from_secs(5);
                                let mut received = 0;
                                let mut background = usize::from(n % 2 == 0);
                                while received < 15 {
                                    match owner.listener.accept() {
                                        Ok((stream, _)) => {
                                            background += usize::from(receive(stream).unwrap());
                                            received += 1;
                                        }
                                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                            assert!(Instant::now() < until);
                                            std::thread::sleep(Duration::from_millis(1));
                                        }
                                        Err(e) => panic!("{e}"),
                                    }
                                }
                                (1, background)
                            }
                        }
                    })
                })
                .collect();
            let results: Vec<_> = contenders.into_iter().map(|t| t.join().unwrap()).collect();
            assert_eq!(
                results.iter().map(|v| v.0).sum::<usize>(),
                1,
                "one cold-start owner"
            );
            assert_eq!(
                results.iter().map(|v| v.1).sum::<usize>(),
                8,
                "all manual/background launch intents preserved"
            );
        });
        assert!(matches!(acquire(&dir, false).unwrap(), Instance::Owner(_)));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn symlink_lock_is_rejected() {
        let dir = std::env::temp_dir().join(format!("gitjuke-lock-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink("/dev/null", dir.join("instance.lock")).unwrap();
        assert!(acquire(&dir, false).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn long_configuration_paths_have_short_identity_isolated_socket_paths() {
        let root = std::env::temp_dir().join(format!("gitjuke-long-{}", std::process::id()));
        let directory = root.join("x".repeat(200));
        let Instance::Owner(first) = acquire(&directory, false).unwrap() else {
            panic!()
        };
        assert!(first.socket.as_os_str().len() < 104);
        let Instance::Owner(second) = acquire(&root.join("other"), true).unwrap() else {
            panic!()
        };
        assert_ne!(first.socket, second.socket);
        drop(first);
        drop(second);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn child() {
        let Some(dir) = std::env::var_os("GITJUKE_SINGLETON_TEST_DIR") else {
            return;
        };
        let namespace = std::env::var("GITJUKE_SINGLETON_TEST_ID")
            .ok()
            .map(|id| directory_for(&id).unwrap())
            .unwrap_or_else(|| PathBuf::from(&dir));
        match acquire(&namespace, true).unwrap() {
            Instance::Owner(owner) => {
                std::fs::write(Path::new(&dir).join("ready"), b"ready").unwrap();
                loop {
                    receive(owner.listener.accept().unwrap().0).unwrap();
                }
            }
            Instance::Forwarded => {}
        }
    }
    #[test]
    fn separate_processes_serialize_and_sigkill_releases_lock() {
        use std::process::{Command, Stdio};
        let dir = std::env::temp_dir().join(format!("gitjuke-process-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let identity = format!("gitjuke-process-contexts-test-{}", std::process::id());
        let spawn = |home: &str| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "singleton::tests::child", "--nocapture"])
                .env("GITJUKE_SINGLETON_TEST_DIR", &dir)
                .env("GITJUKE_SINGLETON_TEST_ID", &identity)
                .env("HOME", home)
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        };
        let mut owner = spawn("/private/tmp/gitjuke-owner-home-a");
        let until = Instant::now() + Duration::from_secs(5);
        while !dir.join("ready").exists() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
        let children: Vec<_> = (0..12)
            .map(|_| spawn("/private/tmp/gitjuke-owner-home-b"))
            .collect();
        for mut child in children {
            assert!(child.wait().unwrap().success());
        }
        owner.kill().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(owner.wait().unwrap().signal(), Some(libc::SIGKILL));
        assert!(matches!(
            acquire(&directory_for(&identity).unwrap(), false).unwrap(),
            Instance::Owner(_)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

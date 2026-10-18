//! The only GitHub process boundary. Never expose argument vectors through IPC.
use crate::{Error, Result};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

pub const STDOUT_LIMIT: usize = 16 * 1024 * 1024;
pub const STDERR_LIMIT: usize = 64 * 1024;
#[derive(Clone)]
pub struct Transport {
    pub path: PathBuf,
    pub config_dir: PathBuf,
    env: BTreeMap<String, String>,
    gate: Arc<Mutex<()>>,
    limits: Limits,
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub stdout: usize,
    pub stderr: usize,
    pub deadline: Duration,
    pub grace: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            stdout: STDOUT_LIMIT,
            stderr: STDERR_LIMIT,
            deadline: Duration::from_secs(30),
            grace: Duration::from_secs(2),
        }
    }
}

pub enum ReadOp {
    Activity(crate::activity::ActivityRead),
    Graph(crate::sync::GraphRead),
    Teams { page: u32 },
    Search { query: String, page: u32, size: u16 },
    Version,
    AuthStatus,
    Identity,
    Authored { login: String, page: u32 },
    Overview { repo: String, number: u64 },
    Stacks { repo: String, number: u64 },
    StackDetail { repo: String, number: u64 },
}
impl ReadOp {
    fn args(self) -> Vec<String> {
        if let Self::Activity(read) = self {
            let mut args = ReadOp::Identity.args();
            args.pop();
            args.push(read.url);
            args.extend([
                "-H".into(),
                "Accept: application/vnd.github.full+json".into(),
            ]);
            for (name, value) in [
                ("If-None-Match", read.etag),
                ("If-Modified-Since", read.modified),
            ] {
                if let Some(value) = value.filter(|v| !v.contains(['\r', '\n'])) {
                    args.extend(["-H".into(), format!("{name}: {value}")]);
                }
            }
            return args;
        }
        let args: Vec<String> = match self {
            Self::Activity(_) => unreachable!(),
            Self::Graph(_) => unreachable!("GraphQL is sent on stdin"),
            Self::Teams { page } => vec![format!("user/teams?per_page=100&page={page}")],
            Self::Search { query, page, size } => vec![format!(
                "search/issues?q={}&per_page={size}&page={page}",
                encode(&query)
            )],
            Self::Version => return vec!["--version".into()],
            Self::AuthStatus => {
                return [
                    "auth",
                    "status",
                    "--active",
                    "--hostname",
                    "github.com",
                    "--json",
                    "hosts",
                ]
                .map(String::from)
                .to_vec();
            }
            Self::Identity => vec!["user".into()],
            Self::Authored { login, page } => vec![format!(
                "search/issues?q=is%3Apr%20is%3Aopen%20author%3A{}&per_page=100&page={page}",
                encode(&login)
            )],
            Self::Overview { repo, number } => vec![format!("repos/{repo}/pulls/{number}")],
            Self::Stacks { repo, number } => vec![format!(
                "repos/{repo}/stacks?pull_request={number}&per_page=100"
            )],
            Self::StackDetail { repo, number } => vec![format!("repos/{repo}/stacks/{number}")],
        };
        [
            vec![
                "api".into(),
                "--hostname".into(),
                "github.com".into(),
                "--method".into(),
                "GET".into(),
                "--include".into(),
                "-H".into(),
                "X-GitHub-Api-Version: 2026-03-10".into(),
            ],
            args,
        ]
        .concat()
    }
}
fn encode(v: &str) -> String {
    v.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
pub fn ignored_overrides() -> Vec<String> {
    [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "GH_HOST",
        "GH_REPO",
        "GH_CONFIG_DIR",
        "XDG_CONFIG_HOME",
    ]
    .iter()
    .filter(|key| std::env::var_os(key).is_some())
    .map(|s| (*s).into())
    .collect()
}
pub fn os_home() -> Result<PathBuf> {
    // passwd is independent of inherited HOME/XDG shell overrides.
    unsafe {
        let p = libc::getpwuid(libc::getuid());
        if p.is_null() {
            return Err(Error::Safe(
                "Cannot resolve the OS account home directory.".into(),
            ));
        }
        Ok(PathBuf::from(
            std::ffi::CStr::from_ptr((*p).pw_dir)
                .to_string_lossy()
                .into_owned(),
        ))
    }
}
impl Transport {
    pub fn discover(saved: Option<&str>, config: Option<&str>) -> Result<Self> {
        let path = if let Some(saved) = saved {
            PathBuf::from(saved)
        } else {
            ["/opt/homebrew/bin/gh","/usr/local/bin/gh","/usr/bin/gh"].iter().map(PathBuf::from).find(|p|p.is_file()).ok_or_else(|| Error::Safe("Installed gh was not found. Select an absolute executable path in Preferences.".into()))?
        };
        let home = os_home()?;
        let config = config
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config/gh"));
        Self::new(path, config, home, Limits::default())
    }
    pub fn new(path: PathBuf, config_dir: PathBuf, home: PathBuf, limits: Limits) -> Result<Self> {
        if !path.is_absolute() || !config_dir.is_absolute() {
            return Err(Error::Safe(
                "gh and configuration paths must be absolute.".into(),
            ));
        }
        let path = std::fs::canonicalize(path).map_err(|_| {
            Error::Safe("The saved gh executable is missing or inaccessible.".into())
        })?;
        use std::os::unix::fs::PermissionsExt;
        if !path.is_file() || std::fs::metadata(&path)?.permissions().mode() & 0o111 == 0 {
            return Err(Error::Safe(
                "The selected gh path is not executable.".into(),
            ));
        }
        let mut env = BTreeMap::new();
        for key in [
            "USER",
            "LOGNAME",
            "TMPDIR",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
        ] {
            if let Ok(value) = std::env::var(key) {
                env.insert(key.into(), value);
            }
        }
        env.insert("HOME".into(), home.to_string_lossy().into_owned());
        env.insert(
            "GH_CONFIG_DIR".into(),
            config_dir.to_string_lossy().into_owned(),
        );
        for (k, v) in [
            ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin"),
            ("GH_PROMPT_DISABLED", "1"),
            ("GH_PAGER", "cat"),
            ("GH_BROWSER", "false"),
            ("NO_COLOR", "1"),
            ("GH_COLOR_LABELS", "false"),
        ] {
            env.insert(k.into(), v.into());
        }
        Ok(Self {
            path,
            config_dir,
            env,
            gate: Arc::new(Mutex::new(())),
            limits,
        })
    }
    pub async fn read(&self, op: ReadOp, cancel: &CancellationToken) -> Result<Output> {
        let _guard = tokio::select! { _=cancel.cancelled()=>return Err(Error::Safe("Read cancelled.".into())), g=self.gate.lock()=>g };
        if cancel.is_cancelled() {
            return Err(Error::Safe("Read cancelled.".into()));
        }
        let (args, input) = match op {
            ReadOp::Graph(read) => (
                [
                    "api",
                    "graphql",
                    "--hostname",
                    "github.com",
                    "--method",
                    "POST",
                    "--include",
                    "--input",
                    "-",
                ]
                .map(String::from)
                .to_vec(),
                Some(serde_json::to_vec(&read.body())?),
            ),
            op => (op.args(), None),
        };
        let mut command = Command::new(&self.path);
        command
            .args(args)
            .env_clear()
            .envs(&self.env)
            .current_dir(Path::new("/"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let mut child = command.spawn().map_err(|_| {
            Error::Safe("gh could not start. Check the selected executable.".into())
        })?;
        let pid = child
            .id()
            .ok_or_else(|| Error::Safe("gh process ID unavailable.".into()))?;
        let mut stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let read = async {
            let (out, err, status, _) = tokio::try_join!(
                bounded(stdout, self.limits.stdout),
                bounded(stderr, self.limits.stderr),
                async { child.wait().await.map_err(Error::from) },
                async {
                    if let Some(mut stdin) = stdin.take() {
                        if let Some(bytes) = input {
                            stdin.write_all(&bytes).await?;
                        }
                        stdin.shutdown().await?;
                    }
                    Ok::<_, Error>(())
                }
            )?;
            Ok::<_, Error>(Output {
                stdout: out,
                stderr: err,
                success: status.success(),
            })
        };
        let result = tokio::select! {
            _=cancel.cancelled()=>Err(Error::Safe("Read cancelled.".into())),
            r=tokio::time::timeout(self.limits.deadline,read)=>r.unwrap_or_else(|_|Err(Error::Safe("gh exceeded the 30-second wall deadline.".into())))
        };
        // Kill the owned group even after the leader exits: helpers may retain pipes.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
        if result.is_err() {
            tokio::time::sleep(self.limits.grace).await;
        }
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        let _ = child.wait().await;
        result
    }
}
async fn bounded(mut input: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(limit.min(8192));
    let mut buf = [0; 8192];
    loop {
        let n = input.read(&mut buf).await?;
        if n == 0 {
            return Ok(out);
        }
        if out.len() + n > limit {
            return Err(Error::Safe(
                "gh exceeded its output limit; no partial response was accepted.".into(),
            ));
        }
        out.extend_from_slice(&buf[..n]);
    }
}
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub success: bool,
}

pub fn indicates_auth_loss(output: &Output) -> bool {
    if output.success {
        return false;
    }
    let diagnostic = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    diagnostic.contains("gh auth login") || diagnostic.contains("not logged into")
}

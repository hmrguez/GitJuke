//! Explicit Actions reads. Bodies and terminal states live only in one replaceable memory slot.
use crate::{
    Error, Result, github,
    runtime::{Inner, Job, Runtime, Work},
    sync::CheckContext,
    transport::{ReadOp, Transport},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub const LOG_BYTES: usize = 128 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogJob {
    pub repo: String,
    pub number: u64,
    pub head: String,
    pub check: String,
    pub offset: usize,
    #[serde(default)]
    pub seq: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRead {
    pub state: String,
    pub text: Option<String>,
}
impl LogRead {
    fn state(state: &str) -> Self {
        Self {
            state: state.into(),
            text: None,
        }
    }
}
pub(crate) struct LogSlot {
    job: LogJob,
    generation: u64,
    epoch: i64,
    context: CheckContext,
    result: LogRead,
}
fn bad() -> Error {
    Error::Safe("Check log selection is obsolete or unavailable; refresh Checks.".into())
}
fn job_id(c: &CheckContext, repo: &str) -> Option<u64> {
    if c.kind != "CheckRun" || c.run_id? == 0 || c.attempt? == 0 {
        return None;
    }
    let prefix = format!("https://github.com/{repo}/actions/runs/{}/job/", c.run_id?);
    let id = c.url.as_ref()?.strip_prefix(&prefix)?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse::<u64>().ok().filter(|n| *n > 0)
}
fn details_url(c: &CheckContext) -> Option<String> {
    let url = c.url.as_ref()?;
    // Only absolute HTTPS URLs with a conventional authority; never credentials, controls,
    // backslashes, whitespace, scheme-relative paths or executable/custom schemes.
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if url.len() > 4096
        || host.is_empty()
        || host.contains(['@', ':', '\\'])
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        || url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return None;
    }
    Some(url.clone())
}
impl Runtime {
    pub(crate) fn cancel_logs_locked(i: &mut Inner) {
        i.log_seq = i.log_seq.saturating_add(1);
        i.log_slot = None;
        i.queue.jobs.retain(|w| !matches!(w.job, Job::Logs(_)));
        if i.log_running {
            i.current.cancel();
        }
    }
    pub fn cancel_logs(&self, seq: u64) -> Result<()> {
        let mut i = self.lock()?;
        if i.log_slot.as_ref().is_some_and(|s| s.job.seq == seq) {
            Self::cancel_logs_locked(&mut i);
        }
        Ok(())
    }
    fn log_context(i: &mut Inner, j: &LogJob) -> Result<(CheckContext, i64)> {
        if j.repo.len() > 256
            || !github::valid_repo(&j.repo)
            || j.number == 0
            || j.number > i64::MAX as u64
            || j.offset > i64::MAX as usize
            || j.check.len() > 256
            || j.head.len() > 128
            || i.locked
            || !i.reader_open
            || i.selected.as_ref() != Some(&(j.repo.clone(), j.number))
        {
            return Err(bad());
        }
        let p = i.store.check_page(&j.repo, j.number, j.offset)?;
        if p.head.as_deref() != Some(&j.head)
            || p.error.is_some()
            || !p.commits.iter().any(|c| c.complete)
        {
            return Err(bad());
        }
        let c = p
            .items
            .into_iter()
            .find(|c| {
                c.id == j.check
                    && p.commits
                        .iter()
                        .any(|s| s.resource == c.commit && s.complete && s.error.is_none())
            })
            .ok_or_else(bad)?;
        let epoch = i.store.conn.query_row(
            "SELECT epoch FROM watch_locator WHERE repo=? AND number=?",
            rusqlite::params![j.repo, j.number as i64],
            |r| r.get(0),
        )?;
        Ok((c, epoch))
    }
    pub fn check_details(&self, j: &LogJob) -> Result<String> {
        let (c, _) = Self::log_context(&mut *self.lock()?, j)?;
        details_url(&c)
            .ok_or_else(|| Error::Safe("No validated HTTPS details link was supplied.".into()))
    }
    pub fn request_logs(&self, mut j: LogJob) -> Result<u64> {
        let mut i = self.lock()?;
        Self::require_remote_storage(&i)?;
        let (context, epoch) = Self::log_context(&mut i, &j)?;
        if job_id(&context, &j.repo).is_none() {
            return Err(Error::Safe(
                "This provider exposes details only; Actions job logs are unavailable.".into(),
            ));
        }
        Self::cancel_logs_locked(&mut i);
        j.seq = i.log_seq;
        let generation = i.store.generation()?;
        if !i.queue.push(Work {
            job: Job::Logs(j.clone()),
            manual: true,
            foreground: true,
            ticket: None,
        }) {
            return Err(Error::Safe("Reader queue is full. Retry when idle.".into()));
        }
        i.log_slot = Some(LogSlot {
            job: j.clone(),
            generation,
            epoch,
            context,
            result: LogRead::state("queued"),
        });
        drop(i);
        self.notify_sync();
        Ok(j.seq)
    }
    fn valid_log(i: &mut Inner, j: &LogJob, g: u64) -> Result<bool> {
        let Some(slot) = i.log_slot.as_ref() else {
            return Ok(false);
        };
        if slot.job != *j || slot.generation != g || i.store.generation()? != g {
            return Ok(false);
        }
        let epoch = slot.epoch;
        let context = serde_json::to_vec(&slot.context)?;
        let (current, e) = match Self::log_context(i, j) {
            Ok(v) => v,
            Err(e @ (Error::Storage(_) | Error::StorageUnavailable(_))) => return Err(e),
            Err(_) => return Ok(false),
        };
        Ok(e == epoch && serde_json::to_vec(&current)? == context)
    }
    pub(crate) fn maintain_logs(i: &mut Inner) -> Result<()> {
        if let Some(s) = i.log_slot.as_ref() {
            let j = s.job.clone();
            let g = s.generation;
            match Self::valid_log(i, &j, g) {
                Ok(true) => {}
                Ok(false) => Self::cancel_logs_locked(i),
                Err(e) => {
                    Self::cancel_logs_locked(i);
                    return Err(e);
                }
            }
        }
        Ok(())
    }
    pub fn read_logs(&self, seq: u64) -> Result<LogRead> {
        let mut i = self.lock()?;
        let Some(slot) = i.log_slot.as_ref() else {
            return Ok(LogRead::state("released"));
        };
        if seq != slot.job.seq {
            return Ok(LogRead::state("released"));
        }
        let j = slot.job.clone();
        let g = slot.generation;
        match Self::valid_log(&mut i, &j, g) {
            Ok(true) => {}
            Ok(false) => {
                Self::cancel_logs_locked(&mut i);
                return Ok(LogRead::state("released"));
            }
            Err(e) => {
                Self::cancel_logs_locked(&mut i);
                return Err(e);
            }
        }
        Ok(i.log_slot.as_ref().unwrap().result.clone())
    }
    pub(crate) async fn execute_logs(
        &self,
        t: &Transport,
        c: &CancellationToken,
        g: u64,
        j: &LogJob,
    ) -> Result<()> {
        let outcome = self.load_log(t, c, g, j).await;
        let mut i = self.lock()?;
        if let Err(e @ (Error::Storage(_) | Error::StorageUnavailable(_))) = outcome {
            Self::release_log_job(&mut i, j);
            return Err(e);
        }
        if c.is_cancelled() {
            Self::release_log_job(&mut i, j);
            return Ok(());
        }
        match Self::valid_log(&mut i, j, g) {
            Ok(true) => {}
            Ok(false) => {
                Self::release_log_job(&mut i, j);
                return Ok(());
            }
            Err(e) => {
                Self::release_log_job(&mut i, j);
                return Err(e);
            }
        }
        i.log_slot.as_mut().unwrap().result = outcome.unwrap_or_else(|e| {
            LogRead::state(if e.to_string().contains("output limit") {
                "oversized"
            } else {
                "failed"
            })
        });
        Ok(()) // Never pass provider output/body/diagnostics to persistent resource errors.
    }
    fn release_log_job(i: &mut Inner, j: &LogJob) {
        if i.log_slot.as_ref().is_some_and(|s| s.job == *j) {
            Self::cancel_logs_locked(i);
        }
    }
    async fn load_log(
        &self,
        t: &Transport,
        c: &CancellationToken,
        g: u64,
        j: &LogJob,
    ) -> Result<LogRead> {
        let context = {
            let mut i = self.lock()?;
            if !Self::valid_log(&mut i, j, g)? {
                return Err(bad());
            }
            let s = i.log_slot.as_mut().unwrap();
            s.result = LogRead::state("reading");
            s.context.clone()
        };
        self.identity(t, c, g).await?;
        let job = job_id(&context, &j.repo).ok_or_else(bad)?;
        // Verify PR head, latest run attempt and selected job association before AND after bytes.
        {
            let pr = self
                .response(
                    t,
                    ReadOp::Overview {
                        repo: j.repo.clone(),
                        number: j.number,
                    },
                    c,
                )
                .await?;
            if matches!(pr.require_ok(), Err(Error::Denied)) {
                self.lock()?.store.activity_purge(&j.repo, j.number)?;
                return Ok(LogRead::state("denied"));
            }
            pr.require_ok()?;
            if pr.body["head"]["sha"].as_str() != Some(&j.head) {
                return Err(bad());
            }
            let run = self
                .response(
                    t,
                    ReadOp::ActionsRun {
                        repo: j.repo.clone(),
                        run: context.run_id.unwrap(),
                    },
                    c,
                )
                .await?;
            if !(200..300).contains(&run.status) {
                return Ok(classify(&run));
            }
            if run.body["id"].as_u64() != context.run_id
                || run.body["run_attempt"].as_u64() != context.attempt
                || run.body["head_sha"].as_str() != Some(&context.commit)
            {
                return Err(bad());
            }
            let metadata = self
                .response(
                    t,
                    ReadOp::ActionsJob {
                        repo: j.repo.clone(),
                        job,
                    },
                    c,
                )
                .await?;
            if !(200..300).contains(&metadata.status) {
                return Ok(classify(&metadata));
            }
            let check = self
                .response(
                    t,
                    ReadOp::ActionCheck {
                        repo: j.repo.clone(),
                        check: check_id(&metadata.body, &j.repo).ok_or_else(bad)?,
                    },
                    c,
                )
                .await?;
            if !(200..300).contains(&check.status) {
                return Ok(classify(&check));
            }
            if check.body["node_id"].as_str() != Some(&context.id)
                || check.body["app"]["slug"] != "github-actions"
                || check.body["head_sha"].as_str() != Some(&context.commit)
            {
                return Err(bad());
            }
            if !matches_job(&metadata.body, &context, &j.repo, job) {
                return Err(bad());
            }
        }
        // Revalidated below again after download; no logs are committed to the store.
        let response = self
            .response(
                t,
                ReadOp::ActionsLog {
                    repo: j.repo.clone(),
                    job,
                },
                c,
            )
            .await?;
        let result = classify(&response);
        let run = self
            .response(
                t,
                ReadOp::ActionsRun {
                    repo: j.repo.clone(),
                    run: context.run_id.unwrap(),
                },
                c,
            )
            .await?;
        run.require_ok()?;
        let metadata = self
            .response(
                t,
                ReadOp::ActionsJob {
                    repo: j.repo.clone(),
                    job,
                },
                c,
            )
            .await?;
        metadata.require_ok()?;
        let check = self
            .response(
                t,
                ReadOp::ActionCheck {
                    repo: j.repo.clone(),
                    check: check_id(&metadata.body, &j.repo).ok_or_else(bad)?,
                },
                c,
            )
            .await?;
        check.require_ok()?;
        if check.body["node_id"].as_str() != Some(&context.id)
            || check.body["app"]["slug"] != "github-actions"
            || check.body["head_sha"].as_str() != Some(&context.commit)
        {
            return Err(bad());
        }
        let pr = self
            .response(
                t,
                ReadOp::Overview {
                    repo: j.repo.clone(),
                    number: j.number,
                },
                c,
            )
            .await?;
        if matches!(pr.require_ok(), Err(Error::Denied)) {
            self.lock()?.store.activity_purge(&j.repo, j.number)?;
            return Err(bad());
        }
        pr.require_ok()?;
        if run.body["id"].as_u64() != context.run_id
            || run.body["head_sha"].as_str() != Some(&context.commit)
            || run.body["run_attempt"].as_u64() != context.attempt
            || !matches_job(&metadata.body, &context, &j.repo, job)
            || pr.body["head"]["sha"].as_str() != Some(&j.head)
        {
            return Err(bad());
        }
        self.identity(t, c, g).await?;
        Ok(result)
    }
}
fn check_id(v: &Value, repo: &str) -> Option<u64> {
    let prefix = format!("https://api.github.com/repos/{repo}/check-runs/");
    let id = v["check_run_url"].as_str()?.strip_prefix(&prefix)?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse::<u64>().ok().filter(|n| *n > 0)
}
fn matches_job(v: &Value, c: &CheckContext, repo: &str, job: u64) -> bool {
    v["id"].as_u64() == Some(job)
        && v["run_id"].as_u64() == c.run_id
        && v["run_attempt"].as_u64() == c.attempt
        && v["head_sha"].as_str() == Some(&c.commit)
        && v["html_url"].as_str() == c.url.as_deref()
        && check_id(v, repo).is_some()
}
fn classify(r: &github::Response) -> LogRead {
    match r.status {
        200 if r.body["limited"] == true => LogRead::state("oversized"),
        200 => match r.body["text"].as_str() {
            Some("") => LogRead::state("empty"),
            Some(s) => LogRead {
                state: "complete".into(),
                text: Some(s.into()),
            },
            None => LogRead::state("failed"),
        },
        401 | 403 if r.next_deadline(crate::now()) == 0 => LogRead::state("denied"),
        404 => LogRead::state("missing"),
        410 => LogRead::state("expired"),
        _ => LogRead::state("failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Output;
    fn output(status: u16, raw: &[u8], success: bool) -> Output {
        let mut stdout = format!("HTTP/2.0 {status} Status\r\n\r\n").into_bytes();
        stdout.extend(raw);
        Output {
            stdout,
            stderr: vec![],
            success,
        }
    }
    #[test]
    fn log_utf8_inclusive_limit_and_worst_case_ipc_encoding() {
        let r = github::Response::parse_log(output(200, &vec![0; LOG_BYTES], true)).unwrap();
        let v = classify(&r);
        assert_eq!(v.state, "complete");
        assert!(serde_json::to_vec(&v).unwrap().len() < 1024 * 1024);
        let raw = "é".repeat(LOG_BYTES / 2);
        assert_eq!(
            classify(&github::Response::parse_log(output(200, raw.as_bytes(), true)).unwrap())
                .text
                .unwrap()
                .len(),
            LOG_BYTES
        );
        assert_eq!(
            classify(
                &github::Response::parse_log(output(200, &vec![b'x'; LOG_BYTES + 1], true))
                    .unwrap()
            )
            .state,
            "oversized"
        );
        assert!(github::Response::parse_log(output(200, &[255], true)).is_err());
        assert!(github::Response::parse_log(output(200, b"incomplete", false)).is_err());
        assert!(github::Response::parse_log(output(302, b"signed private URL", true)).is_err());
    }
    #[test]
    fn quota_errors_and_arbitrary_provider_diagnostics_are_never_empty_or_retained() {
        for (status, expected) in [
            (403, "denied"),
            (404, "missing"),
            (410, "expired"),
            (500, "failed"),
        ] {
            let r =
                github::Response::parse_log(output(status, b"PRIVATE DIAGNOSTIC", false)).unwrap();
            assert_eq!(classify(&r).state, expected);
            assert!(!r.body.to_string().contains("PRIVATE"));
        }
        let r = github::Response::parse_log(output(403, b"secondary rate limit", false)).unwrap();
        assert_eq!(classify(&r).state, "failed");
        assert!(r.next_deadline(crate::now()) > 0);
    }
    #[test]
    fn external_details_reject_unsafe_schemes_authorities_and_controls() {
        let mut c:CheckContext=serde_json::from_value(serde_json::json!({"id":"x","kind":"StatusContext","name":"ci","status":"SUCCESS","conclusion":null,"url":null,"suiteId":null,"suiteStatus":null,"suiteConclusion":null,"suiteUpdatedAt":null,"runId":null,"attempt":null,"commit":"a"})).unwrap();
        for url in [
            "javascript:alert(1)",
            "file:///tmp/x",
            "https://good.test@evil.test/",
            "https://ci.test\\evil/",
            "https://ci.test/\n",
            "https:///path",
        ] {
            c.url = Some(url.into());
            assert!(details_url(&c).is_none());
        }
        c.url = Some("https://ci.example/results?job=1#log".into());
        assert!(details_url(&c).is_some());
        assert!(job_id(&c, "a/b").is_none());
    }
}

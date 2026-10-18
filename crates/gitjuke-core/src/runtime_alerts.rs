use crate::{Error, Result, alerts::*, now, runtime::Runtime};
use std::sync::Arc;
pub(crate) struct Probe {
    reservation: Reservation,
    after: String,
    guards: std::collections::VecDeque<CheckGuard>,
    done: bool,
}
impl Runtime {
    pub fn install_alert_sink(&self, sink: Arc<dyn AlertSink>) {
        *self.alert_sink.lock().unwrap() = Some(sink);
        self.notify_sync();
    }
    pub fn report_notification_error(&self) {
        if let Ok(mut i) = self.lock() {
            i.health="macOS did not confirm the alert request. It will not be replayed; Activity remains available.".into();
        }
    }
    pub fn set_alert_choices(&self, choices: AlertChoices) -> Result<()> {
        let mut i = self.lock()?;
        Self::store_operation(&mut i, |s| s.alert_choices(choices))?;
        drop(i);
        self.notify_sync();
        Ok(())
    }
    pub fn notification_destination(&self, id: i64) -> Result<NotificationDestination> {
        let i = self.lock()?;
        if i.locked {
            return Err(Error::Safe(i.health.clone()));
        }
        i.store.notification_route(id)
    }
    /// Executed by the sole worker, between GitHub jobs. Identity probes share its
    /// transport gate, core budget, deadlines, cancellation and account binding.
    pub(crate) async fn dispatch_alert(&self) -> Result<()> {
        let sink = self.alert_sink.lock().unwrap().clone();
        let Some(sink) = sink else { return Ok(()) };
        let permission = sink.permission();
        if permission == Permission::Unknown {
            return Ok(());
        }
        let (t, token, epoch, mut probe) = {
            let mut i = self.lock()?;
            if i.storage_stopped || !i.store.cache_writes {
                return Ok(());
            }
            Self::store_operation(&mut i, |s| s.alert_permission(permission))?;
            if i.locked
                || i.store.settings()?.paused
                || now() < i.store.due("remote")?
                || !i.store.budget_eligible("core", now())?
            {
                return Ok(());
            }
            let Some(t) = i.transport.clone() else {
                return Ok(());
            };
            let old = self.alert_probe.lock().unwrap().take();
            let probe = match old {
                Some(p) if i.store.reservation_current(&p.reservation)? => p,
                _ => {
                    let Some(reservation) = Self::store_operation(&mut i, |s| s.reserve_alert())?
                    else {
                        return Ok(());
                    };
                    Probe {
                        reservation,
                        after: String::new(),
                        guards: std::collections::VecDeque::new(),
                        done: false,
                    }
                }
            };
            let token = self.work_token();
            i.current = token.clone();
            (t, token, i.store.generation()?, probe)
        };
        if probe.guards.is_empty() && !probe.done {
            let (after, guards, done) = {
                self.lock()?
                    .store
                    .alert_check_window(probe.reservation.intent, &probe.after)?
            };
            probe.after = after;
            probe.guards = guards.into();
            probe.done = done;
        }
        if let Some(guard) = probe.guards.pop_front() {
            let identity = self.identity(&t, &token, epoch).await?;
            let h = self
                .response(
                    &t,
                    crate::transport::ReadOp::Graph(crate::sync::GraphRead::AlertHead {
                        repo: guard.repo.clone(),
                        number: guard.number,
                    }),
                    &token,
                )
                .await?;
            if alert_head_denied(&h) {
                let mut i = self.lock()?;
                if !token.is_cancelled() && !i.locked && i.store.generation()? == epoch {
                    Self::store_operation(&mut i, |s| s.activity_purge(&guard.repo, guard.number))?;
                }
                return Err(Error::Denied);
            }
            let h = crate::sync::graph_data(&h)?;
            if h["viewer"]["databaseId"].as_u64() != Some(identity.id) {
                let mut i = self.lock()?;
                i.locked = true;
                Self::store_operation(&mut i, |s| s.invalidate())?;
                return Err(Error::Safe(
                    "Alert head identity changed; no native attempt was made.".into(),
                ));
            }
            let pr = &h["repository"]["pullRequest"];
            let response = self
                .response(
                    &t,
                    crate::transport::ReadOp::WorkflowRun {
                        repo: guard.repo.clone(),
                        run: guard.run,
                    },
                    &token,
                )
                .await?;
            response.require_ok()?;
            if !guard.matches(
                pr["headRefOid"].as_str(),
                pr["potentialMergeCommit"]["oid"].as_str(),
                &response.body,
            ) {
                let mut i = self.lock()?;
                if !i.locked && i.store.generation()? == epoch {
                    Self::store_operation(&mut i, |s| {
                        s.conn.execute("UPDATE watch_locator SET due=0 WHERE rowid IN(SELECT rowid FROM watch_locator WHERE repo=? AND number=? ORDER BY rowid LIMIT 128)",rusqlite::params![guard.repo,guard.number as i64])?;
                        Ok(())
                    })?;
                }
                return Ok(());
            }
        }
        if !probe.guards.is_empty() || !probe.done {
            // One 128-ID window and at most one workflow guard per worker turn.
            // Ordinary selected/background scheduling runs before the next step.
            *self.alert_probe.lock().unwrap() = Some(probe);
            return Ok(());
        }
        self.identity(&t, &token, epoch).await?;
        let mut i = self.lock()?;
        if self.stop.is_cancelled()
            || token.is_cancelled()
            || i.locked
            || sink.permission() != Permission::Granted
        {
            return Ok(());
        }
        Self::require_remote_storage(&i)?;
        if Self::store_operation(&mut i, |s| s.attempt_alert(&probe.reservation))? {
            sink.dispatch(&probe.reservation.alert);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        activity_crash::{identity, record},
        store::Store,
    };
    use std::{
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };
    struct Sink {
        path: PathBuf,
        sends: AtomicUsize,
    }
    impl AlertSink for Sink {
        fn permission(&self) -> Permission {
            Permission::Granted
        }
        fn dispatch(&self, a: &Alert) {
            let c = rusqlite::Connection::open(&self.path).unwrap();
            let committed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM activity_intent i JOIN alert_attempt a ON a.id='native:'||i.id WHERE i.id=? AND state='attempted')",[a.intent],|r|r.get(0)).unwrap();
            assert!(
                committed,
                "dispatch sees durable attempt from a separate connection"
            );
            self.sends.fetch_add(1, Ordering::SeqCst);
        }
    }
    async fn wait(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(40), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("worker state");
    }
    #[tokio::test]
    async fn worker_revalidates_identity_and_commits_attempt_before_platform_dispatch() {
        for mode in ["ok", "mismatch"] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let gh = d.path().join("gh");
            std::fs::write(&gh, include_str!("../tests/support/gh.py")).unwrap();
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut s = Store::open(&path).unwrap();
            let mut settings = s.settings().unwrap();
            settings.gh_path = Some(gh.display().to_string());
            settings.config_dir = Some(d.path().join("config").display().to_string());
            s.save_settings(&settings).unwrap();
            drop(s);
            let r = Runtime::start(path.clone(), |_| {}).unwrap();
            let sink = Arc::new(Sink {
                path: path.clone(),
                sends: AtomicUsize::new(0),
            });
            r.install_alert_sink(sink.clone());
            wait(|| r.activity(0).is_ok_and(|p| p.baseline)).await;
            r.set_alert_choices(AlertChoices {
                enabled: true,
                previews: false,
                categories: [true; 5],
            })
            .unwrap();
            {
                let mut i = r.lock().unwrap();
                i.selected = Some(("a/b".into(), 1));
                i.reader_open = true;
                i.reader_visible = true;
                let mut settings = i.store.settings().unwrap();
                settings.alerts.since = [now() - 60; 5];
                i.store.save_settings(&settings).unwrap();
                if mode == "mismatch" {
                    std::fs::write(d.path().join("mode"), "mismatch").unwrap();
                }
                let item = record("worker-alert", now() + 1);
                i.store.activity_stage("a/b", 1, &[item], true).unwrap();
                i.store
                    .activity_publish("a/b", 1, false, &identity())
                    .unwrap();
            }
            if mode == "ok" {
                wait(|| sink.sends.load(Ordering::SeqCst) == 1).await;
                r.set_pause(true).unwrap();
                assert!(
                    !r.activity(0)
                        .unwrap()
                        .items
                        .iter()
                        .find(|a| a.id == "worker-alert")
                        .unwrap()
                        .seen
                );
            } else {
                wait(|| r.status().unwrap().locked).await;
                assert_eq!(sink.sends.load(Ordering::SeqCst), 0);
            }
            r.shutdown().await;
            let c = rusqlite::Connection::open(&path).unwrap();
            let attempts: i64 = c
                .query_row(
                    "SELECT count(*) FROM alert_attempt WHERE id LIKE 'native:%'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(attempts, if mode == "ok" { 1 } else { 0 });
        }
    }
}

impl CheckGuard {
    fn matches(&self, head: Option<&str>, merge: Option<&str>, run: &serde_json::Value) -> bool {
        run["repository"]["full_name"]
            .as_str()
            .is_some_and(|v| v.eq_ignore_ascii_case(&self.repo))
            && head == Some(self.head.as_str())
            && (head == Some(self.commit.as_str()) || merge == Some(self.commit.as_str()))
            && run["id"].as_u64() == Some(self.run)
            && run["run_attempt"].as_u64() == Some(self.attempt)
            && run["head_sha"].as_str() == Some(self.commit.as_str())
            && run["status"] == "completed"
            && run["conclusion"].as_str().is_some_and(|v| !v.is_empty())
    }
}
#[cfg(test)]
mod guard_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn authoritative_source_guard_never_guesses_missing_attempt_or_superseded_head() {
        let response = |body| crate::github::Response {
            status: 200,
            headers: Default::default(),
            body,
        };
        assert!(alert_head_denied(&response(
            json!({"data":{"repository":{"pullRequest":null}}})
        )));
        assert!(alert_head_denied(&response(
            json!({"errors":[{"type":"FORBIDDEN","path":["repository"]}]})
        )));
        assert!(!alert_head_denied(&response(
            json!({"errors":[{"type":"RATE_LIMITED"}]})
        )));
        assert!(!alert_head_denied(&response(json!({"data":{}}))));
        let mut limited = response(json!({"message":"secondary rate limit"}));
        limited.status = 403;
        limited.headers.insert("retry-after".into(), "60".into());
        assert!(!alert_head_denied(&limited));
        let g = CheckGuard {
            repo: "a/b".into(),
            number: 1,
            head: "head".into(),
            commit: "head".into(),
            run: 7,
            attempt: 2,
        };
        let source = json!({"repository":{"full_name":"a/b"},"id":7,"run_attempt":2,"head_sha":"head","status":"completed","conclusion":"success"});
        assert!(g.matches(Some("head"), None, &source));
        for field in [
            "id",
            "run_attempt",
            "head_sha",
            "status",
            "conclusion",
            "repository",
        ] {
            let mut absent = source.clone();
            absent.as_object_mut().unwrap().remove(field);
            assert!(!g.matches(Some("head"), None, &absent), "{field}");
        }
        let mut retry = source.clone();
        retry["run_attempt"] = json!(3);
        assert!(!g.matches(Some("head"), None, &retry));
        assert!(!g.matches(Some("new-head"), Some("head"), &source));
        let merge = CheckGuard {
            commit: "merge".into(),
            ..g
        };
        let mut merged = source;
        merged["head_sha"] = json!("merge");
        assert!(merge.matches(Some("head"), Some("merge"), &merged));
        assert!(!merge.matches(Some("head"), Some("old-merge"), &merged));
    }
}

fn alert_head_denied(r: &crate::github::Response) -> bool {
    matches!(r.require_ok(), Err(Error::Denied))
        || r.body["errors"].as_array().is_some_and(|errors| {
            errors.iter().any(|e| {
                ["FORBIDDEN", "NOT_FOUND"].contains(&e["type"].as_str().unwrap_or_default())
                    && e["path"][0] == "repository"
            })
        })
        || r.body
            .get("data")
            .and_then(|d| d.get("repository"))
            .is_some_and(|repo| {
                repo.is_null() || repo.get("pullRequest").is_some_and(|pr| pr.is_null())
            })
}

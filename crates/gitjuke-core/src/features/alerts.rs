//! Durable, at-most-once OS interruption policy. Delivery is never a seen receipt.
use crate::{
    Error, Result,
    activity::ActivityRecord,
    now,
    store::{Store, bump},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub use gitjuke_model::{AlertSettings, Permission};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertChoices {
    pub enabled: bool,
    pub previews: bool,
    pub categories: [bool; 5],
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub intent: i64,
    pub title: String,
    pub body: String,
    pub count: u64,
}
#[derive(Clone, Debug)]
pub struct Reservation {
    pub(crate) intent: i64,
    generation: u64,
    policy: String,
    digest: String,
    destination: Option<String>,
    pub alert: Alert,
}
/// Platform adapter must return promptly. An OS enqueue error remains ambiguous/terminal.
pub trait AlertSink: Send + Sync {
    fn permission(&self) -> Permission;
    fn dispatch(&self, alert: &Alert);
}
fn category(s: &str) -> Option<usize> {
    ["comment", "review", "request", "mention", "check"]
        .iter()
        .position(|k| *k == s)
}
pub(crate) fn intent_state(c: &rusqlite::Connection) -> Result<&'static str> {
    let json: Option<String> = c
        .query_row("SELECT json FROM settings WHERE id=1", [], |r| r.get(0))
        .optional()?;
    let s: crate::model::Settings = json
        .map(|j| serde_json::from_str(&j))
        .transpose()?
        .unwrap_or_default();
    Ok(
        if s.alerts.enabled && s.alerts.permission == Permission::Granted {
            "pending"
        } else {
            "unavailable"
        },
    )
}
impl Store {
    pub fn alert_choices(&mut self, choices: AlertChoices) -> Result<()> {
        self.publication_ready()?;
        let mut s = self.settings()?;
        let floor: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(id),0) FROM activity_intent", [], |r| {
                    r.get(0)
                })?;
        for k in 0..5 {
            if (!s.alerts.enabled && choices.enabled)
                || (!s.alerts.categories[k] && choices.categories[k])
            {
                s.alerts.since[k] = now();
            }
        }
        if !s.alerts.enabled && choices.enabled {
            s.alerts.floor = floor;
        }
        s.alerts.enabled = choices.enabled;
        s.alerts.previews = choices.previews;
        s.alerts.categories = choices.categories;
        s.alerts.onboarded = true;
        self.save_settings(&s)
    }
    pub(crate) fn alert_permission(&mut self, permission: Permission) -> Result<()> {
        let mut s = self.settings()?;
        if s.alerts.permission == permission {
            return Ok(());
        }
        self.publication_ready()?;
        if permission == Permission::Granted {
            s.alerts.floor = self.conn.query_row(
                "SELECT COALESCE(MAX(id),0) FROM activity_intent",
                [],
                |r| r.get(0),
            )?;
            s.alerts.since = [now(); 5];
        }
        s.alerts.permission = permission;
        self.save_settings(&s)
    }
    /// Rebuild only never-attempted membership into the existing single recovery summary.
    /// One cursor and <=60 identity-only catchup writes per transaction; restart is idempotent.
    pub(crate) fn reconstruct_alerts(&mut self) -> Result<bool> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM activity_intent WHERE state='recovery' ORDER BY id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else { return Ok(true) };
        let key = format!("alerts:reconstruct:{id}");
        let after: String = self
            .conn
            .query_row("SELECT value FROM progress WHERE key=?", [&key], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or_default();
        let tx = self.conn.transaction()?;
        let before = tx.total_changes();
        let ids=tx.prepare("SELECT activity FROM intent_member WHERE intent=? AND activity>? ORDER BY activity LIMIT 60")?.query_map(params![id,after],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for a in &ids {
            tx.execute(
                "INSERT OR IGNORE INTO progress VALUES(?,?)",
                params![format!("catchup:{a}"), a],
            )?;
        }
        if ids.len() < 60 {
            tx.execute(
                "UPDATE activity_intent SET state='reconstructed' WHERE id=? AND state='recovery'",
                [id],
            )?;
            tx.execute("DELETE FROM progress WHERE key=?", [key])?;
        } else {
            tx.execute("INSERT INTO progress VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,ids.last()])?;
        }
        crate::shared_storage::bounded(&tx, before)?;
        tx.commit()?;
        Ok(false)
    }
    fn eligible_alert(&self, intent: i64) -> Result<(String, u64, Option<ActivityRecord>)> {
        use sha2::{Digest, Sha256};
        let s = self.settings()?;
        let mut digest = Sha256::new();
        let mut count = 0;
        let mut first = None;
        let mut after = String::new();
        loop {
            let rows=self.conn.prepare("SELECT activity FROM intent_member WHERE intent=? AND activity>? ORDER BY activity LIMIT 128")?.query_map(params![intent,after],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for id in &rows {
                after = id.clone();
                let row:Option<(String,u64,String,u64,bool,bool)>=self.conn.query_row("SELECT a.repo,a.number,a.kind,a.occurred,a.eligible,s.seen FROM activity a JOIN seen s ON s.source_id=a.id WHERE a.id=?",[id],|r|Ok((r.get(0)?,r.get::<_,i64>(1)? as u64,r.get(2)?,r.get::<_,i64>(3)? as u64,r.get(4)?,r.get(5)?))).optional()?;
                let Some((repo, n, kind, time, eligible, seen)) = row else {
                    continue;
                };
                let Some(k) = category(&kind) else { continue };
                if !eligible
                    || seen
                    || !s.alerts.categories[k]
                    || time <= s.alerts.since[k]
                    || time < now().saturating_sub(crate::retention::ACTIVITY_AGE)
                {
                    continue;
                }
                let denied:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM pr_purge WHERE repo=? COLLATE NOCASE AND number=?) OR EXISTS(SELECT 1 FROM resource_state WHERE key='conversation-denied:'||lower(?)||':'||? AND ?!='check')",params![repo,n as i64,repo,n as i64,kind],|r|r.get(0))?;
                if denied || self.alert_pr_unavailable(&repo, n)? {
                    continue;
                }
                let payload = self.get_alert_payload(id)?;
                if kind == "check" {
                    let Some(head) = payload.as_ref().and_then(|p| p.head.as_ref()) else {
                        continue;
                    };
                    let current:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM watch_locator l WHERE l.repo=? AND l.number=? AND l.head=? AND NOT EXISTS(SELECT 1 FROM check_purge p WHERE p.pr_id=l.id))",params![repo,n as i64,head],|r|r.get(0))?;
                    if !current {
                        continue;
                    }
                    // Retry data can change without changing the PR head. Require the exact
                    // authoritative terminal identity still to exist in the complete snapshot.
                    if !self.current_workflow(id, &repo, n, head)? {
                        continue;
                    }
                }
                count += 1;
                digest.update(id.as_bytes());
                digest.update([0]);
                if first.is_none() {
                    first = Some(payload.unwrap_or(ActivityRecord {
                        id: id.clone(),
                        repo,
                        number: n,
                        kind,
                        occurred: time,
                        actor: None,
                        url: String::new(),
                        text: None,
                        eligible: true,
                        reason: String::new(),
                        seen: false,
                        head: None,
                    }));
                }
            }
            if rows.len() < 128 {
                break;
            }
        }
        Ok((format!("{:x}", digest.finalize()), count, first))
    }
    fn get_alert_payload(&self, id: &str) -> Result<Option<ActivityRecord>> {
        let raw:Option<String>=self.conn.query_row("SELECT json FROM content WHERE key=? AND accessed>=? AND NOT EXISTS(SELECT 1 FROM content_clear)",params![format!("activity-payload:{id}"),now().saturating_sub(crate::retention::CONTENT_AGE) as i64],|r|r.get(0)).optional()?;
        raw.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    fn alerts_ready(&self) -> Result<bool> {
        let s = self.settings()?;
        Ok(s.alerts.enabled
            && s.alerts.permission == Permission::Granted
            && !s.paused
            && !crate::publication::pending(&self.conn)?
            && !crate::invalidation::pending(&self.conn)?
            && !crate::clear_content::pending(&self.conn)?
            && !crate::control_sweep::pending(&self.conn)?)
    }
    pub(crate) fn reserve_alert(&mut self) -> Result<Option<Reservation>> {
        if !self.alerts_ready()? {
            return Ok(None);
        }
        crate::store::storage_healthy(
            std::path::Path::new(self.conn.path().unwrap())
                .parent()
                .unwrap(),
        )?;
        let s = self.settings()?;
        let row: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id,kind FROM activity_intent WHERE state='pending' ORDER BY id LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, kind)) = row else {
            return Ok(None);
        };
        let (digest, count, first) = self.eligible_alert(id)?;
        if id <= s.alerts.floor || count == 0 {
            self.conn.execute(
                "UPDATE activity_intent SET state='unavailable' WHERE id=? AND state='pending'",
                [id],
            )?;
            return Ok(None);
        }
        let mut title = if kind == "summary" {
            "Missed activity"
        } else {
            match first.as_ref().map(|a| a.kind.as_str()) {
                Some("comment") if count == 1 => "New comment",
                Some("review") if count == 1 => "New review",
                Some("request") if count == 1 => "Review requested",
                Some("mention") if count == 1 => "You were mentioned",
                Some("check") if count == 1 => "Checks completed",
                _ => "Personal activity",
            }
        }
        .to_owned();
        let mut body = format!(
            "{count} new {}",
            if count == 1 {
                "activity item"
            } else {
                "activity items"
            }
        );
        if kind != "summary"
            && let Some(a) = &first
        {
            body.push_str(&format!(
                " · {}",
                if count == 1 {
                    a.kind.as_str()
                } else {
                    "personal activity"
                }
            ));
        }
        let destination = first.as_ref().map(|a| a.id.clone());
        if s.alerts.previews
            && kind != "summary"
            && let Some(a) = first
        {
            let action = match a.kind.as_str() {
                "comment" => "New comment",
                "review" => "New review",
                "request" => "Review requested",
                "mention" => "You were mentioned",
                "check" => "Checks completed",
                _ => "New activity",
            };
            title = preview_text(
                &format!(
                    "{action}{} · {} #{}",
                    a.actor
                        .as_ref()
                        .map(|v| format!(
                            " {} {v}",
                            if matches!(a.kind.as_str(), "request" | "mention") {
                                "by"
                            } else {
                                "from"
                            }
                        ))
                        .unwrap_or_default(),
                    a.repo,
                    a.number
                ),
                160,
            );
            let pr_title: Option<String> = self
                .conn
                .query_row(
                    "SELECT json_extract(json,'$.title') FROM content WHERE key=? AND accessed>=? AND NOT EXISTS(SELECT 1 FROM content_clear)",
                    params![
                        format!("pr:{}:{}", a.repo, a.number),
                        now().saturating_sub(crate::retention::CONTENT_AGE) as i64
                    ],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            let excerpt = preview_text(a.text.as_deref().unwrap_or_default(), 180);
            let context = preview_text(pr_title.as_deref().unwrap_or_default(), 60);
            body = match (context.is_empty(), excerpt.is_empty()) {
                (false, false) => format!("{context}\n{excerpt}"),
                (false, true) => context,
                (true, false) => excerpt,
                (true, true) => match a.kind.as_str() {
                    "request" => "Your review was requested.".into(),
                    "check" => "Checks finished. Open the pull request for results.".into(),
                    _ => "No message preview available.".into(),
                },
            };
            if count > 1 {
                body.push_str(&format!(
                    "\nAnd {} more {}",
                    count - 1,
                    if count == 2 { "update" } else { "updates" }
                ));
            }
        }
        Ok(Some(Reservation {
            intent: id,
            generation: self.generation()?,
            policy: serde_json::to_string(&s.alerts)?,
            digest,
            destination,
            alert: Alert {
                intent: id,
                title,
                body,
                count,
            },
        }))
    }
    pub(crate) fn attempt_alert(&mut self, r: &Reservation) -> Result<bool> {
        crate::store::storage_healthy(
            std::path::Path::new(self.conn.path().unwrap())
                .parent()
                .unwrap(),
        )?;
        if !self.alerts_ready()?
            || self.generation()? != r.generation
            || serde_json::to_string(&self.settings()?.alerts)? != r.policy
        {
            return Ok(false);
        }
        let (digest, count, _) = self.eligible_alert(r.intent)?;
        if digest != r.digest || count != r.alert.count {
            return Ok(false);
        }
        let tx = self.conn.transaction()?;
        let before = tx.total_changes();
        if tx.execute(
            "UPDATE activity_intent SET state='attempted' WHERE id=? AND state='pending'",
            [r.intent],
        )? == 0
        {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO alert_attempt VALUES(?,?)",
            params![format!("native:{}", r.intent), now() as i64],
        )?;
        if let Some(id) = &r.destination {
            tx.execute(
                "INSERT INTO progress VALUES(?,?)",
                params![format!("native:destination:{}", r.intent), id],
            )?;
        }
        bump(&tx)?;
        #[cfg(test)]
        crate::activity_crash::point("attempt_before_commit");
        crate::shared_storage::bounded(&tx, before)?;
        tx.commit()?;
        #[cfg(test)]
        crate::activity_crash::point("attempt_after_commit");
        Ok(true)
    }
    /// Only opaque committed attempted-intent IDs arrive from the OS. Re-resolve locally.
    pub fn alert_destination(&self, intent: i64) -> Result<Option<ActivityRecord>> {
        let kind: Option<String> = self
            .conn
            .query_row(
                "SELECT kind FROM activity_intent WHERE id=? AND state='attempted'",
                [intent],
                |r| r.get(0),
            )
            .optional()?;
        if kind.as_deref() != Some("pr") {
            return Ok(None);
        }
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM progress WHERE key=?",
                [format!("native:destination:{intent}")],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else { return Ok(None) };
        let exists:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM activity a WHERE id=? AND occurred>=? AND NOT EXISTS(SELECT 1 FROM pr_purge p WHERE p.repo=a.repo COLLATE NOCASE AND p.number=a.number))",params![id,now().saturating_sub(crate::retention::ACTIVITY_AGE) as i64],|r|r.get(0))?;
        if !exists {
            return Ok(None);
        }
        let denied:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM activity a WHERE a.id=? AND ((a.kind!='check' AND EXISTS(SELECT 1 FROM resource_state d WHERE d.key='conversation-denied:'||lower(a.repo)||':'||a.number)) OR (a.kind='check' AND EXISTS(SELECT 1 FROM check_purge p JOIN watch_locator l ON l.id=p.pr_id WHERE l.repo=a.repo AND l.number=a.number))))",[&id],|r|r.get(0))?;
        if denied {
            return Ok(None);
        }
        let (repo, number): (String, u64) =
            self.conn
                .query_row("SELECT repo,number FROM activity WHERE id=?", [&id], |r| {
                    Ok((r.get(0)?, r.get::<_, i64>(1)? as u64))
                })?;
        if self.alert_pr_unavailable(&repo, number)? {
            return Ok(None);
        }
        if let Some(record) = self.get_alert_payload(&id)? {
            return Ok(Some(record));
        }
        // Eviction/Clear removes disposable bodies, not the retained exact source
        // identity. Explicit opening may refetch the destination through its normal
        // reader path; no old payload or successful render is manufactured.
        let record=self.conn.query_row("SELECT a.repo,a.number,a.kind,a.occurred,a.eligible,a.reason,s.seen FROM activity a JOIN seen s ON s.source_id=a.id WHERE a.id=?",[&id],|r|Ok(ActivityRecord{id:id.clone(),repo:r.get(0)?,number:r.get::<_,i64>(1)? as u64,kind:r.get(2)?,occurred:r.get::<_,i64>(3)? as u64,eligible:r.get(4)?,reason:r.get(5)?,seen:r.get(6)?,actor:None,url:String::new(),text:None,head:None}))?;
        Ok(Some(record))
    }
}
/// Collapse source whitespace and bound native notification text without splitting Unicode.
fn preview_text(text: &str, limit: usize) -> String {
    let mut chars = text.split_whitespace().enumerate().flat_map(|(i, word)| {
        (i > 0)
            .then_some(' ')
            .into_iter()
            .chain(word.chars().filter(|c| !c.is_control()))
    });
    let mut result: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        result.pop();
        result.push('…');
    }
    result
}

#[cfg(test)]
#[path = "alerts_tests.rs"]
mod tests;

#[derive(Debug)]
pub(crate) struct CheckGuard {
    pub repo: String,
    pub number: u64,
    pub head: String,
    pub commit: String,
    pub run: u64,
    pub attempt: u64,
}
impl Store {
    pub(crate) fn alert_check_window(
        &self,
        intent: i64,
        after: &str,
    ) -> Result<(String, Vec<CheckGuard>, bool)> {
        let s = self.settings()?;
        if !s.alerts.categories[4] {
            return Ok((after.into(), vec![], true));
        }
        let ids=self.conn.prepare("SELECT m.activity FROM intent_member m WHERE m.intent=? AND m.activity>? ORDER BY m.activity LIMIT 128")?.query_map(params![intent,after],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut guards = vec![];
        for id in &ids {
            let valid:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM activity a JOIN seen s ON s.source_id=a.id WHERE a.id=? AND a.kind='check' AND a.eligible=1 AND s.seen=0 AND a.occurred>? AND a.occurred>=?)",params![id,s.alerts.since[4] as i64,now().saturating_sub(crate::retention::ACTIVITY_AGE) as i64],|r|r.get(0))?;
            if !valid {
                continue;
            }
            let Some(a) = self.get_alert_payload(id)? else {
                continue;
            };
            let Some(head) = a.head else { continue };
            let prefix = format!("github.com:{}:{}:workflow:", a.repo, a.number);
            let Some(tail) = id.strip_prefix(&prefix) else {
                continue;
            };
            let fields = tail.split(':').collect::<Vec<_>>();
            if fields.len() != 3 {
                continue;
            }
            let (Ok(run), Ok(attempt)) = (fields[0].parse(), fields[1].parse()) else {
                continue;
            };
            guards.push(CheckGuard {
                repo: a.repo,
                number: a.number,
                head,
                commit: fields[2].into(),
                run,
                attempt,
            });
        }
        Ok((
            ids.last().cloned().unwrap_or_else(|| after.into()),
            guards,
            ids.len() < 128,
        ))
    }
}

impl Store {
    fn current_workflow(&self, id: &str, repo: &str, number: u64, head: &str) -> Result<bool> {
        let prefix = format!("github.com:{repo}:{number}:workflow:");
        let Some(tail) = id.strip_prefix(&prefix) else {
            return Ok(false);
        };
        let fields = tail.split(':').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Ok(false);
        }
        let (Ok(run), Ok(attempt)) = (fields[0].parse::<u64>(), fields[1].parse::<u64>()) else {
            return Ok(false);
        };
        let locator:Option<(String,Option<String>)>=self.conn.query_row("SELECT id,merge_sha FROM watch_locator WHERE repo=? AND number=? AND head=? LIMIT 1",params![repo,number as i64,head],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((pr, merge)) = locator else {
            return Ok(false);
        };
        if fields[2] != head && merge.as_deref() != Some(fields[2]) {
            return Ok(false);
        }
        let complete:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM check_state WHERE pr_id=? AND commit_sha=? AND complete=1 AND error IS NULL AND accessed>=?)",params![pr,fields[2],now().saturating_sub(crate::retention::CONTENT_AGE) as i64],|r|r.get(0))?;
        if !complete {
            return Ok(false);
        }
        let mut after = String::new();
        let mut found = false;
        loop {
            let rows=self.conn.prepare("SELECT id,json FROM check_context WHERE pr_id=? AND commit_sha=? AND staged=0 AND id>? ORDER BY id LIMIT 128")?.query_map(params![pr,fields[2],after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, json) in &rows {
                after = id.clone();
                let c: crate::sync::CheckContext = serde_json::from_str(json)?;
                if c.run_id != Some(run) {
                    continue;
                }
                if c.attempt != Some(attempt)
                    || c.suite_status.as_deref() != Some("COMPLETED")
                    || c.suite_conclusion.is_none()
                    || c.suite_id.is_none()
                {
                    return Ok(false);
                }
                found = true;
            }
            if rows.len() < 128 {
                return Ok(found);
            }
        }
    }
}

impl Store {
    pub(crate) fn reservation_current(&self, r: &Reservation) -> Result<bool> {
        let pending: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM activity_intent WHERE id=? AND state='pending')",
            [r.intent],
            |r| r.get(0),
        )?;
        Ok(pending
            && self.generation()? == r.generation
            && serde_json::to_string(&self.settings()?.alerts)? == r.policy)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationDestination {
    pub record: Option<ActivityRecord>,
    pub unavailable: bool,
}
impl Store {
    pub(crate) fn notification_route(&self, intent: i64) -> Result<NotificationDestination> {
        let record = self.alert_destination(intent)?;
        let summary:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM activity_intent WHERE id=? AND kind='summary' AND state='attempted')",[intent],|r|r.get(0))?;
        let unavailable = record.is_none() && !summary;
        Ok(NotificationDestination {
            record,
            unavailable,
        })
    }
}

impl Store {
    fn alert_pr_unavailable(&self, repo: &str, number: u64) -> Result<bool> {
        // The purge journal is temporary; its durable denial and reset target
        // remain authoritative until a complete authorized hydration succeeds.
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM resource_state WHERE key='pr:'||lower(?1)||':'||?2 AND error='Resource unavailable') AND NOT EXISTS(SELECT 1 FROM activity_target WHERE repo=?1 COLLATE NOCASE AND number=?2 AND done=1)",params![repo,number as i64],|r|r.get(0))?)
    }
}

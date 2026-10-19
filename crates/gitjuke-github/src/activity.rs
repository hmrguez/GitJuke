//! Notification threads discover PRs; only authoritative individual records become Activity.
use crate::{Error, Result, github, model::Identity};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityJob {
    Notifications {
        page: u32,
        size: u16,
        since: u64,
        before: u64,
    },
    Hydrate {
        repo: String,
        number: u64,
        source: u8,
        page: u32,
        size: u16,
    },
}
impl ActivityJob {
    pub fn key(&self) -> String {
        match self {
            Self::Notifications { .. } => "activity:notifications".into(),
            Self::Hydrate { repo, number, .. } => format!("activity:pr:{repo}:{number}"),
        }
    }
    pub fn url(&self) -> String {
        match self {
            Self::Notifications {
                page,
                size,
                since,
                before,
            } => format!(
                "notifications?all=true&participating=false&per_page={size}&page={page}&before={}{}",
                crate::sync::date(*before),
                if *since == 0 {
                    String::new()
                } else {
                    format!("&since={}", crate::sync::date(since.saturating_sub(1)))
                }
            ),
            Self::Hydrate {
                repo,
                number,
                source,
                page,
                size,
            } => match source {
                0 => format!("repos/{repo}/pulls/{number}"),
                1 => format!("repos/{repo}/issues/{number}/comments?per_page={size}&page={page}"),
                2 => format!("repos/{repo}/pulls/{number}/reviews?per_page={size}&page={page}"),
                3 => format!("repos/{repo}/pulls/{number}/comments?per_page={size}&page={page}"),
                _ => format!("repos/{repo}/issues/{number}/timeline?per_page={size}&page={page}"),
            },
        }
    }
}
pub struct ActivityRead {
    pub url: String,
    pub etag: Option<String>,
    pub modified: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityRecord {
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub kind: String,
    pub occurred: u64,
    pub actor: Option<String>,
    pub url: String,
    pub text: Option<String>,
    pub eligible: bool,
    pub reason: String,
    pub seen: bool,
    #[serde(default)]
    pub head: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub items: Vec<ActivityRecord>,
    pub next_offset: Option<usize>,
    pub revision: u64,
    pub baseline: bool,
    pub recovery: Option<String>,
    pub pending: u64,
    pub error: Option<String>,
    pub next_poll_at: u64,
}
fn schema() -> Error {
    Error::Safe("Individual activity source is incomplete or unsupported; successful progress was not advanced.".into())
}
pub fn timestamp(s: &str) -> Result<u64> {
    // Strict UTC source timestamps, independent of the machine's local timezone.
    if s.len() != 20 || !s.ends_with('Z') {
        return Err(schema());
    }
    let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
    let value = std::ffi::CString::new(s).map_err(|_| schema())?;
    unsafe {
        let end = libc::strptime(
            value.as_ptr(),
            c"%Y-%m-%dT%H:%M:%SZ".as_ptr(),
            tm.as_mut_ptr(),
        );
        if end.is_null() || *end != 0 {
            return Err(schema());
        }
        let raw = libc::timegm(tm.as_mut_ptr());
        if raw < 0 || crate::sync::date(raw as u64) != s {
            return Err(schema());
        }
        Ok(raw as u64)
    }
}
/// Requires a GitHub-rendered mention link. Plain strings in code/URLs are not mentions.
fn mentions(html: &str, login: &str, teams: &[String]) -> bool {
    html.split('<')
        .filter_map(|s| s.strip_prefix("a "))
        .any(|tag| {
            let tag = tag.split('>').next().unwrap_or_default();
            let class = tag
                .split("class=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or_default();
            let href = tag
                .split("href=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or_default();
            (class.split_whitespace().any(|c| c == "user-mention")
                && href.eq_ignore_ascii_case(&format!("https://github.com/{login}")))
                || (class.split_whitespace().any(|c| c == "team-mention")
                    && teams.iter().any(|t| {
                        href.eq_ignore_ascii_case(&format!(
                            "https://github.com/orgs/{}/teams/{}",
                            t.split('/').next().unwrap_or_default(),
                            t.split('/').nth(1).unwrap_or_default()
                        ))
                    }))
        })
}
pub fn notification_targets(body: &Value) -> Result<Vec<(String, u64)>> {
    let mut result = Vec::new();
    for thread in body.as_array().ok_or_else(schema)? {
        if thread["subject"]["type"] != "PullRequest" {
            continue;
        }
        let repo = github::string(&thread["repository"], "full_name")?;
        if !github::valid_repo(&repo) {
            return Err(schema());
        }
        let prefix = format!("https://api.github.com/repos/{repo}/pulls/");
        let n = thread["subject"]["url"]
            .as_str()
            .and_then(|s| s.strip_prefix(&prefix))
            .and_then(|n| n.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .ok_or_else(schema)?;
        result.push((repo, n));
    }
    Ok(result)
}
pub fn records(
    repo: &str,
    number: u64,
    source: u8,
    body: &Value,
    identity: &Identity,
    teams: &[(String, String)],
    authored: bool,
) -> Result<Vec<ActivityRecord>> {
    let single = vec![body.clone()];
    let values = if source == 0 {
        &single
    } else {
        body.as_array().ok_or_else(schema)?
    };
    let mut out = Vec::new();
    for v in values {
        let kind = match source {
            0 => "mention",
            1 | 3 => "comment",
            2 => {
                if v["state"] == "PENDING" {
                    continue;
                }
                "review"
            }
            _ => {
                if v["event"] != "review_requested" {
                    continue;
                }
                "request"
            }
        };
        let actor = if source == 4 { &v["actor"] } else { &v["user"] };
        let raw = v["body"].as_str().unwrap_or_default();
        let html = v["body_html"].as_str();
        // Missing rendered content cannot safely establish or exclude individual mentions.
        if !raw.is_empty() && html.is_none() && source != 4 {
            return Err(schema());
        }
        let mention = html.is_some_and(|h| {
            mentions(
                h,
                &identity.login,
                &teams.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>(),
            )
        });
        let requested = source == 4
            && (v["requested_reviewer"]["id"].as_u64() == Some(identity.id)
                || v["requested_team"]["node_id"]
                    .as_str()
                    .is_some_and(|id| teams.iter().any(|(node, _)| node == id)));
        let kind = if source == 4 {
            if !requested {
                continue;
            }
            "request"
        } else if mention {
            "mention"
        } else if source == 0 || !authored {
            continue;
        } else {
            kind
        };
        let id = v["node_id"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| v["id"].as_u64().map(|id| format!("rest:{source}:{id}")))
            .ok_or_else(schema)?;
        if id.len() > 256 {
            return Err(schema());
        }
        let time = if source == 2 {
            v["submitted_at"].as_str()
        } else if kind == "mention" {
            v["updated_at"]
                .as_str()
                .or_else(|| v["created_at"].as_str())
        } else {
            v["created_at"].as_str()
        }
        .ok_or_else(schema)?;
        let own = actor["id"].as_u64() == Some(identity.id);
        let bot = actor["type"] == "Bot"
            || actor["login"]
                .as_str()
                .is_some_and(|s| s.ends_with("[bot]"));
        let routine_bot = bot && kind == "comment";
        let url = v["html_url"]
            .as_str()
            .filter(|s| s.starts_with(&format!("https://github.com/{repo}/pull/{number}")))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("https://github.com/{repo}/pull/{number}"));
        out.push(ActivityRecord {
            id: format!("github.com:{repo}:{number}:{id}"),
            repo: repo.into(),
            number,
            kind: kind.into(),
            occurred: timestamp(time)?,
            actor: actor["login"].as_str().map(str::to_owned),
            url,
            text: if raw.len() <= 8192 {
                Some(raw.into())
            } else {
                None
            },
            eligible: !own && !routine_bot,
            reason: if own {
                "own action"
            } else if routine_bot {
                "routine bot comment"
            } else {
                "eligible"
            }
            .into(),
            seen: false,
            head: None,
        });
    }
    Ok(out)
}

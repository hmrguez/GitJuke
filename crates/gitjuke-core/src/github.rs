use crate::{Error, Result, model::*, transport::Output};
use serde_json::Value;
use std::collections::BTreeMap;

pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}
impl Response {
    pub fn parse(output: Output) -> Result<Self> {
        let split=output.stdout.windows(4).position(|p|p==b"\r\n\r\n").map(|i|(i,4))
            .or_else(||output.stdout.windows(2).position(|p|p==b"\n\n").map(|i|(i,2)))
            .ok_or_else(||Error::Safe("gh API response headers unavailable; command/schema compatibility is unverified.".into()))?;
        let head = std::str::from_utf8(&output.stdout[..split.0])
            .map_err(|_| Error::Safe("Invalid API headers.".into()))?;
        let status = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| Error::Safe("Invalid API status.".into()))?;
        let headers = head
            .lines()
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().into()))
            .collect();
        let bytes = &output.stdout[split.0 + split.1..];
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(bytes)?
        };
        if !output.success && status < 400 {
            return Err(Error::Safe("gh failed; response was not committed.".into()));
        }
        Ok(Self {
            status,
            headers,
            body,
        })
    }
    /// Raw job logs never enter JSON parsing or persistent content. Keep only bounded UTF-8.
    pub fn parse_log(output: Output) -> Result<Self> {
        let bytes = &output.stdout;
        let (end, sep) = bytes
            .windows(4)
            .position(|p| p == b"\r\n\r\n")
            .map(|n| (n, 4))
            .or_else(|| bytes.windows(2).position(|p| p == b"\n\n").map(|n| (n, 2)))
            .ok_or_else(|| Error::Safe("Missing log response headers.".into()))?;
        let head = std::str::from_utf8(&bytes[..end])
            .map_err(|_| Error::Safe("Invalid log headers.".into()))?;
        let status = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::Safe("Invalid log status.".into()))?;
        let headers = head
            .lines()
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        let raw = &bytes[end + sep..];
        let body = if status == 200 && output.success {
            if raw.len() > crate::logs::LOG_BYTES {
                serde_json::json!({"limited":true})
            } else {
                serde_json::json!({"text":std::str::from_utf8(raw).map_err(|_|Error::Safe("Log is not UTF-8.".into()))?})
            }
        } else if status >= 400 {
            // Keep only structured quota signals, never a provider body.
            let rate = raw
                .windows(10)
                .any(|w| w.eq_ignore_ascii_case(b"rate limit"));
            serde_json::json!({"message":if rate {"rate limit"} else {""}})
        } else {
            return Err(Error::Safe(
                "Job log download failed or redirect was not followed.".into(),
            ));
        };
        Ok(Self {
            status,
            headers,
            body,
        })
    }
    pub fn next_deadline(&self, now: u64) -> u64 {
        if let Some(value) = self.headers.get("retry-after") {
            if let Ok(seconds) = value.parse::<u64>() {
                return now.saturating_add(seconds);
            }
            if let Ok(date) = httpdate::parse_http_date(value) {
                return date
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs().max(now))
                    .unwrap_or(now);
            }
        }
        if self
            .headers
            .get("x-ratelimit-remaining")
            .is_some_and(|v| v == "0")
        {
            return self
                .headers
                .get("x-ratelimit-reset")
                .and_then(|s| s.parse().ok())
                .unwrap_or(now + 60)
                .max(now + 1);
        }
        if self.status == 429
            || (self.status == 403
                && self.body["message"]
                    .as_str()
                    .is_some_and(|m| m.to_ascii_lowercase().contains("rate limit")))
        {
            now + 60
        } else {
            0
        }
    }
    pub fn require_ok(&self) -> Result<()> {
        match self.status {200..=299=>Ok(()),401=>Err(Error::Safe("Stored gh authentication is unavailable or locked.".into())),403 if self.next_deadline(crate::now())==0=>Err(Error::Denied),403|429=>Err(Error::Safe("GitHub denied or delayed this read; cached data is retained unless resource access was denied.".into())),404=>Err(Error::Denied),_=>Err(Error::Safe(format!("GitHub read failed (HTTP {}).",self.status)))}
    }
}
pub fn parse_auth(output: Output) -> Result<String> {
    let v: Value = serde_json::from_slice(&output.stdout)?;
    let accounts = v["hosts"]["github.com"].as_array().ok_or_else(|| {
        Error::Safe(
            "No stored GitHub.com login exists in the selected configuration context.".into(),
        )
    })?;
    let active = accounts
        .iter()
        .find(|a| a["active"] == true)
        .ok_or_else(|| Error::Safe("No active stored GitHub.com account.".into()))?;
    if !output.success || active["state"] != "success" || active["tokenSource"] == "environment" {
        return Err(Error::Safe("Stored gh credentials could not be validated. Check connectivity, the selected configuration and Keychain access externally.".into()));
    }
    string(active, "login")
}
pub fn parse_identity(response: &Response) -> Result<Identity> {
    response.require_ok()?;
    Ok(Identity {
        id: number(&response.body, "id")?,
        login: string(&response.body, "login")?,
    })
}
pub struct SearchPage {
    pub prs: Vec<PullRequest>,
    pub total: u64,
    pub incomplete: bool,
}
pub fn parse_search(r: &Response) -> Result<SearchPage> {
    r.require_ok()?;
    let items = r.body["items"].as_array().ok_or_else(bad)?;
    let prs = items
        .iter()
        .map(|v| {
            let repo_url = string(v, "repository_url")?;
            let repo = repo_url
                .strip_prefix("https://api.github.com/repos/")
                .filter(|s| valid_repo(s))
                .ok_or_else(bad)?
                .to_owned();
            let url = string(v, "html_url")?;
            let number = number(v, "number")?;
            if url != format!("https://github.com/{repo}/pull/{number}") {
                return Err(bad());
            }
            Ok(PullRequest {
                id: string(v, "node_id")?,
                repo_id: None,
                repo,
                number,
                title: string(v, "title")?,
                url,
                state: if v["pull_request"]["merged_at"].as_str().is_some() {
                    "merged".into()
                } else {
                    string(v, "state")?
                },
                draft: v["draft"].as_bool().unwrap_or(false),
                author: v["user"]["login"].as_str().map(String::from),
                updated_at: string(v, "updated_at")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if prs.len() > 100 {
        return Err(bad());
    }
    Ok(SearchPage {
        prs,
        total: number(&r.body, "total_count")?,
        incomplete: r.body["incomplete_results"].as_bool().ok_or_else(bad)?,
    })
}
pub fn parse_overview(r: &Response, repo: &str, number: u64) -> Result<Overview> {
    r.require_ok()?;
    let v = &r.body;
    if v["number"].as_u64() != Some(number) || v["base"]["repo"]["full_name"].as_str() != Some(repo)
    {
        return Err(bad());
    }
    let url = string(v, "html_url")?;
    if url != format!("https://github.com/{repo}/pull/{number}") {
        return Err(bad());
    }
    let body = v["body"].as_str().map(String::from);
    let body_limited = body.as_ref().is_some_and(|s| s.len() > 128 * 1024);
    Ok(Overview {
        repo_id: v["base"]["repo"]["id"].as_u64(),
        id: string(v, "node_id")?,
        repo: repo.into(),
        number,
        title: string(v, "title")?,
        url,
        body: if body_limited { None } else { body },
        body_limited,
        head: string(&v["head"], "sha")?,
        base: string(&v["base"], "ref")?,
        state: if v["merged"] == true {
            "merged".into()
        } else {
            string(v, "state")?
        },
        draft: v["draft"].as_bool().unwrap_or(false),
        author: v["user"]["login"].as_str().map(String::from),
        stack: StackState::Unknown {
            reason: "Native membership has not been read.".into(),
        },
    })
}
pub fn valid_repo(s: &str) -> bool {
    let parts: Vec<_> = s.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}
pub fn string(v: &Value, key: &str) -> Result<String> {
    v[key].as_str().map(String::from).ok_or_else(bad)
}
fn number(v: &Value, key: &str) -> Result<u64> {
    v[key].as_u64().ok_or_else(bad)
}
fn bad() -> Error {
    Error::Safe("The GitHub response has an unsupported or incomplete schema; no successful empty set was committed.".into())
}

pub fn stack_number(r: &Response) -> Result<Option<u64>> {
    r.require_ok()?;
    let stacks = r.body.as_array().ok_or_else(bad)?;
    if r.headers
        .get("link")
        .is_some_and(|v| v.contains("rel=\"next\""))
        || stacks.len() > 1
    {
        return Err(bad());
    }
    stacks.first().map(|s| number(s, "number")).transpose()
}
pub fn parse_stack(r: &Response, repo: &str, selected: u64) -> Result<StackState> {
    r.require_ok()?;
    let v = &r.body;
    let number = number(v, "number")?;
    let trunk = string(&v["base"], "ref")?;
    let members = v["pull_requests"]
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|p| {
            let n = crate::github::number(p, "number")?;
            let url = string(p, "html_url")?;
            if url != format!("https://github.com/{repo}/pull/{n}") {
                return Err(bad());
            }
            Ok(StackMember {
                number: n,
                title: string(p, "title")?,
                url,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if !members.iter().any(|p| p.number == selected) {
        return Err(bad());
    }
    Ok(StackState::Published {
        number,
        trunk,
        members,
    })
}

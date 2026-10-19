//! A combined initial view followed by independent cursor pages. No rendered HTML, raw source reconstruction or progress publication.
use crate::{
    Error, Result,
    changes::{hash, valid_sha},
    github,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub const BODY_BYTES: usize = 128 * 1024;
pub const PAGE_BYTES: usize = 900 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationJob {
    pub repo: String,
    pub number: u64,
    pub head: String,
    pub source: String,
    pub thread: Option<String>,
    pub after: Option<String>,
    pub seq: u64,
}
impl ConversationJob {
    pub fn validate(&self) -> Result<()> {
        if self.source == "destination"
            && self
                .thread
                .as_deref()
                .is_some_and(|id| id.starts_with("rest:"))
        {
            return Err(Error::Safe("Exact GitHub node identity unavailable for this retained Activity record. It remains unseen.".into()));
        }
        if !github::valid_repo(&self.repo)
            || self.number == 0
            || self.number > i32::MAX as u64
            || !valid_sha(&self.head)
            || !matches!(
                self.source.as_str(),
                "initial" | "comments" | "reviews" | "threads" | "replies" | "destination"
            )
            || self.thread.is_some() != matches!(self.source.as_str(), "replies" | "destination")
            || self
                .thread
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 256)
            || (self.source == "initial" && self.after.is_some())
            || (self.source == "destination"
                && (self.after.is_some()
                    || self
                        .thread
                        .as_deref()
                        .is_some_and(|id| id.starts_with("rest:"))))
            || self
                .after
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 1024)
        {
            return Err(Error::Safe("Invalid Conversation request.".into()));
        }
        Ok(())
    }
    pub fn key(&self) -> String {
        format!(
            "api:repos/{}/pulls/{}/gitjuke-conversation/{}/{}/{}/{}",
            self.repo,
            self.number,
            self.head,
            self.source,
            hash(self.thread.as_deref().unwrap_or("")),
            hash(self.after.as_deref().unwrap_or(""))
        )
    }
    pub fn query(&self, first: usize) -> Value {
        let (owner, name) = self.repo.split_once('/').unwrap();
        let fields = "id url body createdAt author { login }";
        let root_fields = format!("{fields} commit {{ oid }} originalCommit {{ oid }}");
        let threads = format!(
            "reviewThreads(first:$first,after:$after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ id path diffSide line startLine startDiffSide originalLine originalStartLine isOutdated isResolved comments(first:1) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {root_fields} }} }} }} }}"
        );
        let connection = match self.source.as_str() {
            "comments" => format!(
                "comments(first:$first,after:$after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {fields} }} }}"
            ),
            "reviews" => format!(
                "reviews(first:$first,after:$after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {fields} state submittedAt }} }}"
            ),
            "threads" => threads.clone(),
            _ => "reviewThreads(first:1) { totalCount }".to_owned(),
        };
        let query = if self.source == "initial" {
            let threads = threads.replace(",after:$after", "");
            format!(
                "query ConversationInitial($owner:String!,$name:String!,$number:Int!,$first:Int!) {{ repository(owner:$owner,name:$name) {{ pullRequest(number:$number) {{ id headRefOid comments(first:$first) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {fields} }} }} reviews(first:$first) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {fields} state submittedAt }} }} {threads} }} }} rateLimit {{ cost remaining resetAt }} }}"
            )
        } else if self.source == "destination" {
            let ownership = "number repository { nameWithOwner }";
            format!(
                "query ConversationDestination($owner:String!,$name:String!,$number:Int!,$thread:ID!) {{ repository(owner:$owner,name:$name) {{ pullRequest(number:$number) {{ id headRefOid }} }} node(id:$thread) {{ __typename ... on IssueComment {{ {fields} pullRequest {{ {ownership} }} }} ... on PullRequestReview {{ {fields} state pullRequest {{ {ownership} }} }} ... on PullRequestReviewComment {{ {fields} fullDatabaseId pullRequest {{ {ownership} }} }} }} rateLimit {{ cost remaining resetAt }} }}"
            )
        } else if self.source == "replies" {
            format!(
                "query Conversation($owner:String!,$name:String!,$number:Int!,$first:Int!,$after:String,$thread:ID!) {{ repository(owner:$owner,name:$name) {{ pullRequest(number:$number) {{ id headRefOid }} }} node(id:$thread) {{ ... on PullRequestReviewThread {{ id pullRequest {{ number repository {{ nameWithOwner }} }} comments(first:$first,after:$after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {fields} }} }} }} }} rateLimit {{ cost remaining resetAt }} }}"
            )
        } else {
            format!(
                "query Conversation($owner:String!,$name:String!,$number:Int!,$first:Int!,$after:String) {{ repository(owner:$owner,name:$name) {{ pullRequest(number:$number) {{ id headRefOid {connection} }} }} rateLimit {{ cost remaining resetAt }} }}"
            )
        };
        let mut variables = json!({"owner":owner,"name":name,"number":self.number,"first":first,"after":self.after});
        if self.source == "destination" {
            variables.as_object_mut().unwrap().remove("first");
            variables.as_object_mut().unwrap().remove("after");
        }
        if self.source == "initial" {
            variables.as_object_mut().unwrap().remove("after");
        }
        if let Some(thread) = &self.thread {
            variables["thread"] = json!(thread);
        }
        json!({"query":query,"variables":variables})
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub kind: String,
    pub author: Option<String>,
    pub time: String,
    pub url: Option<String>,
    pub body: Option<String>,
    pub state: String,
    pub bytes: Option<usize>,
    pub digest: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub path: String,
    pub side: String,
    pub line: Option<u64>,
    pub start_line: Option<u64>,
    pub start_side: Option<String>,
    pub original_line: Option<u64>,
    pub original_start_line: Option<u64>,
    pub commit: Option<String>,
    pub original_commit: Option<String>,
    pub outdated: bool,
    pub resolved: bool,
    pub url: Option<String>,
    pub time: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPage {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seeds: Vec<ConversationSeed>,
    #[serde(default)]
    pub destination_anchor: Option<Thread>,
    pub head: String,
    pub generation: u64,
    pub source: String,
    pub messages: Vec<Message>,
    pub threads: Vec<Thread>,
    pub next: Option<String>,
    pub total: u64,
    pub complete: bool,
}
/// A source's first page, including a thread identity for seeded replies.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSeed {
    pub thread: Option<String>,
    pub page: ConversationPage,
}
impl ConversationPage {
    pub fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
        for seed in &mut self.seeds {
            seed.page.set_generation(generation);
        }
    }
    pub fn body_bytes(&self) -> usize {
        self.messages
            .iter()
            .filter_map(|m| m.body.as_ref())
            .map(String::len)
            .sum::<usize>()
            + self
                .seeds
                .iter()
                .map(|s| s.page.body_bytes())
                .sum::<usize>()
    }
}
/// Coordinates supplied by the exact REST review-comment identity, never reconstructed.
pub fn destination_anchor(v: &Value, j: &ConversationJob, message: &Message) -> Result<Thread> {
    if v["node_id"].as_str() != Some(&message.id)
        || v["pull_request_url"].as_str()
            != Some(&format!(
                "https://api.github.com/repos/{}/pulls/{}",
                j.repo, j.number
            ))
        || v["body"].as_str().map(hash) != message.digest
    {
        return Err(schema());
    }
    Ok(Thread {
        id: message.id.clone(),
        path: text(v, "path", 16 * 1024)?,
        side: text(v, "side", 5)?,
        line: v["line"].as_u64(),
        start_line: v["start_line"].as_u64(),
        start_side: v["start_side"].as_str().map(str::to_owned),
        original_line: v["original_line"].as_u64(),
        original_start_line: v["original_start_line"].as_u64(),
        commit: v["commit_id"]
            .as_str()
            .filter(|s| valid_sha(s))
            .map(str::to_owned),
        original_commit: v["original_commit_id"]
            .as_str()
            .filter(|s| valid_sha(s))
            .map(str::to_owned),
        outdated: v["position"].is_null() || v["line"].is_null(),
        resolved: false,
        url: message.url.clone(),
        time: message.time.clone(),
    })
}
fn schema() -> Error {
    Error::Safe("Incomplete Conversation source; no partial page was published.".into())
}
fn text(v: &Value, k: &str, max: usize) -> Result<String> {
    v[k].as_str()
        .filter(|s| s.len() <= max)
        .map(str::to_owned)
        .ok_or_else(schema)
}
pub fn source_url(url: &str, repo: &str, number: u64) -> Option<String> {
    let root = format!("https://github.com/{repo}/pull/{number}");
    let fragment = url.strip_prefix(&root)?.strip_prefix('#')?;
    if ["issuecomment-", "discussion_r", "pullrequestreview-"]
        .iter()
        .any(|p| {
            fragment
                .strip_prefix(p)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
    {
        Some(url.into())
    } else {
        None
    }
}
pub fn parse(body: &Value, j: &ConversationJob, first: usize) -> Result<ConversationPage> {
    if body
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return Err(schema());
    }
    let pr = &body["data"]["repository"]["pullRequest"];
    if pr["headRefOid"].as_str() != Some(&j.head) {
        return Err(Error::Safe(
            "Conversation revision changed. Update the PR before reading it.".into(),
        ));
    }
    if j.source == "initial" {
        let mut page = ConversationPage {
            seeds: vec![],
            destination_anchor: None,
            head: j.head.clone(),
            generation: 0,
            source: j.source.clone(),
            messages: vec![],
            threads: vec![],
            next: None,
            total: 0,
            complete: true,
        };
        for source in ["comments", "reviews", "threads"] {
            let job = ConversationJob {
                source: source.into(),
                ..j.clone()
            };
            let child = parse(body, &job, first)?;
            page.complete &= child.complete && child.seeds.iter().all(|seed| seed.page.complete);
            page.seeds.push(ConversationSeed {
                thread: None,
                page: child,
            });
        }
        return Ok(page);
    }
    if j.source == "destination" {
        let n = &body["data"]["node"];
        let kind = n["__typename"].as_str().ok_or_else(schema)?;
        let owner = match kind {
            "IssueComment" | "PullRequestReview" | "PullRequestReviewComment" => &n["pullRequest"],
            _ => return Err(schema()),
        };
        if n["id"].as_str() != j.thread.as_deref()
            || owner["number"].as_u64() != Some(j.number)
            || !owner["repository"]["nameWithOwner"]
                .as_str()
                .is_some_and(|r| r.eq_ignore_ascii_case(&j.repo))
            || (kind == "PullRequestReview" && n["state"] == "PENDING")
        {
            return Err(schema());
        }
        let mut single = j.clone();
        single.source = if kind == "PullRequestReview" {
            "reviews"
        } else {
            "comments"
        }
        .into();
        single.thread = None;
        let mut wrapped = body.clone();
        wrapped["data"]["repository"]["pullRequest"][&single.source] =
            json!({"nodes":[n],"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":null}});
        let mut page = parse(&wrapped, &single, 1)?;
        page.source = "destination".into();
        // Exact-node coverage says nothing about completeness of its parent source/thread.
        page.complete = false;
        page.messages[0].kind = if kind == "PullRequestReview" {
            text(n, "state", 64)?
        } else if kind == "IssueComment" {
            "Comment".into()
        } else {
            "Review comment".into()
        };
        return Ok(page);
    }
    let key = match j.source.as_str() {
        "threads" => "reviewThreads",
        s => s,
    };
    let connection = if j.source == "replies" {
        let node = &body["data"]["node"];
        if node["id"].as_str() != j.thread.as_deref()
            || node["pullRequest"]["number"].as_u64() != Some(j.number)
            || node["pullRequest"]["repository"]["nameWithOwner"].as_str() != Some(&j.repo)
        {
            return Err(schema());
        }
        &node["comments"]
    } else {
        &pr[key]
    };
    let nodes = connection["nodes"].as_array().ok_or_else(schema)?;
    if nodes.len() > first || first > 50 {
        return Err(schema());
    }
    let total = connection["totalCount"].as_u64().ok_or_else(schema)?;
    let more = connection["pageInfo"]["hasNextPage"]
        .as_bool()
        .ok_or_else(schema)?;
    if total < nodes.len() as u64 || (!more && j.after.is_none() && total != nodes.len() as u64) {
        return Err(schema());
    }
    let next = if more {
        Some(text(&connection["pageInfo"], "endCursor", 1024)?)
    } else {
        None
    };
    if more && (nodes.is_empty() || next == j.after) {
        return Err(schema());
    }
    let mut p = ConversationPage {
        seeds: vec![],
        destination_anchor: None,
        head: j.head.clone(),
        generation: 0,
        source: j.source.clone(),
        messages: vec![],
        threads: vec![],
        next,
        total,
        complete: !more && j.after.is_none(),
    };
    let mut ids = std::collections::HashSet::new();
    for n in nodes {
        let id = text(n, "id", 256)?;
        if id.is_empty() || !ids.insert(id.clone()) {
            return Err(schema());
        }
        if j.source == "threads" {
            // Parse the nested first comment with the same ownership, cursor and
            // body bounds as an independently loaded reply page.
            let reply_job = ConversationJob {
                source: "replies".into(),
                thread: Some(id.clone()),
                after: None,
                ..j.clone()
            };
            let mut wrapped = body.clone();
            wrapped["data"]["node"] = json!({"id":id,"pullRequest":{"number":j.number,"repository":{"nameWithOwner":j.repo}},"comments":n["comments"]});
            let replies = parse(&wrapped, &reply_job, 1)?;
            p.seeds.push(ConversationSeed {
                thread: Some(id.clone()),
                page: replies,
            });
            let root = &n["comments"]["nodes"][0];
            p.threads.push(Thread {
                id,
                path: text(n, "path", 16 * 1024)?,
                side: text(n, "diffSide", 5)?,
                line: n["line"].as_u64(),
                start_line: n["startLine"].as_u64(),
                start_side: n["startDiffSide"].as_str().map(str::to_owned),
                original_line: n["originalLine"].as_u64(),
                original_start_line: n["originalStartLine"].as_u64(),
                commit: root["commit"]["oid"]
                    .as_str()
                    .filter(|s| valid_sha(s))
                    .map(str::to_owned),
                original_commit: root["originalCommit"]["oid"]
                    .as_str()
                    .filter(|s| valid_sha(s))
                    .map(str::to_owned),
                outdated: n["isOutdated"].as_bool().ok_or_else(schema)?,
                resolved: n["isResolved"].as_bool().ok_or_else(schema)?,
                url: root["url"]
                    .as_str()
                    .and_then(|s| source_url(s, &j.repo, j.number)),
                time: text(root, "createdAt", 64)?,
            });
        } else {
            let body = n["body"].as_str();
            let bytes = body.map(str::len);
            let state = if bytes.is_some_and(|b| b > BODY_BYTES) {
                "oversized"
            } else if body.is_none() {
                "unavailable"
            } else {
                "complete"
            };
            p.messages.push(Message {
                id,
                kind: if j.source == "reviews" {
                    text(n, "state", 64)?
                } else {
                    j.source.clone()
                },
                author: n["author"]["login"]
                    .as_str()
                    .filter(|s| s.len() <= 256)
                    .map(str::to_owned),
                time: text(n, "createdAt", 64)?,
                url: n["url"]
                    .as_str()
                    .and_then(|s| source_url(s, &j.repo, j.number)),
                body: if state == "complete" {
                    body.map(str::to_owned)
                } else {
                    None
                },
                state: state.into(),
                digest: if state == "complete" {
                    body.map(hash)
                } else {
                    None
                },
                bytes,
            });
        }
    }
    Ok(p)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn job() -> ConversationJob {
        ConversationJob {
            repo: "a/b".into(),
            number: 1,
            head: "a".repeat(40),
            source: "comments".into(),
            thread: None,
            after: None,
            seq: 0,
        }
    }
    fn response(j: &ConversationJob, n: Value) -> Value {
        json!({"data":{"repository":{"pullRequest":{"headRefOid":j.head,"comments":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[n]}}}}})
    }
    #[test]
    fn initial_query_combines_sources_and_first_thread_bodies() {
        let mut j = job();
        j.source = "initial".into();
        assert!(j.validate().is_ok());
        let query = j.query(20);
        let text = query["query"].as_str().unwrap();
        for field in [
            "comments(first:$first",
            "reviews(first:$first",
            "reviewThreads(first:$first",
            "comments(first:1)",
            "body",
            "pageInfo",
        ] {
            assert!(text.contains(field), "missing {field}");
        }
        assert!(!text.contains("$after"));
        j.after = Some("cursor".into());
        assert!(j.validate().is_err());
    }
    #[test]
    fn initial_parse_keeps_all_cursors_and_rejects_partial_nested_sources() {
        let j = ConversationJob {
            source: "initial".into(),
            ..job()
        };
        let connection = |nodes: Value, total: u64, next: Option<&str>| json!({"nodes":nodes,"totalCount":total,"pageInfo":{"hasNextPage":next.is_some(),"endCursor":next}});
        let root = json!({"id":"root","createdAt":"now","body":"Requested file change","author":{"login":"reviewer"}});
        let thread = json!({"id":"T","path":"src/app.ts","diffSide":"RIGHT","isOutdated":false,"isResolved":false,"comments":connection(json!([root]),2,Some("reply-next"))});
        let mut body = json!({"data":{"repository":{"pullRequest":{"headRefOid":j.head,
            "comments":connection(json!([{"id":"C","createdAt":"now","body":"comment"}]),2,Some("comment-next")),
            "reviews":connection(json!([{"id":"R","createdAt":"now","body":"review","state":"CHANGES_REQUESTED"}]),1,None),
            "reviewThreads":connection(json!([thread]),2,Some("thread-next"))}}}});
        let mut page = parse(&body, &j, 20).unwrap();
        assert_eq!(page.seeds.len(), 3);
        assert_eq!(page.seeds[0].page.next.as_deref(), Some("comment-next"));
        assert!(page.seeds[1].page.next.is_none());
        assert_eq!(page.seeds[2].page.next.as_deref(), Some("thread-next"));
        let reply = &page.seeds[2].page.seeds[0];
        assert_eq!(reply.thread.as_deref(), Some("T"));
        assert_eq!(
            reply.page.messages[0].body.as_deref(),
            Some("Requested file change")
        );
        assert_eq!(reply.page.next.as_deref(), Some("reply-next"));
        assert!(!page.complete);
        assert_eq!(
            page.body_bytes(),
            "commentreviewRequested file change".len()
        );
        page.set_generation(7);
        assert_eq!(page.seeds[2].page.seeds[0].page.generation, 7);
        body["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"][0]["comments"]["pageInfo"] =
            Value::Null;
        assert!(parse(&body, &j, 20).is_err());
        body["data"]["repository"]["pullRequest"]["reviews"] = Value::Null;
        assert!(parse(&body, &j, 20).is_err());
    }
    #[test]
    fn inclusive_utf8_and_unavailable_bodies() {
        let j = job();
        for (body, state) in [
            (json!("é".repeat(BODY_BYTES / 2)), "complete"),
            (
                json!(format!("{}x", "é".repeat(BODY_BYTES / 2))),
                "oversized",
            ),
            (Value::Null, "unavailable"),
            (json!(""), "complete"),
        ] {
            let p=parse(&response(&j,json!({"id":"I","url":"https://github.com/a/b/pull/1#issuecomment-2","createdAt":"now","body":body})),&j,50).unwrap();
            assert_eq!(p.messages[0].state, state);
            assert_eq!(p.messages[0].body.is_some(), state == "complete");
        }
    }
    #[test]
    fn stale_partial_cursor_and_source_urls() {
        let mut j = job();
        let mut r = response(&j, json!({"id":"I","createdAt":"now","body":"ok"}));
        r["data"]["repository"]["pullRequest"]["headRefOid"] = json!("b".repeat(40));
        assert!(parse(&r, &j, 50).is_err());
        r["data"]["repository"]["pullRequest"]["headRefOid"] = json!(j.head);
        r["errors"] = json!([{}]);
        assert!(parse(&r, &j, 50).is_err());
        r.as_object_mut().unwrap().remove("errors");
        j.after = Some("same".into());
        r["data"]["repository"]["pullRequest"]["comments"]["pageInfo"] =
            json!({"hasNextPage":true,"endCursor":"same"});
        assert!(parse(&r, &j, 50).is_err());
        for u in [
            "https://evil/a/b/pull/1#issuecomment-2",
            "https://github.com/a/b/pull/10#issuecomment-2",
            "https://github.com/a/b/pull/1#discussion_r1?x",
            "javascript:alert(1)",
        ] {
            assert!(source_url(u, "a/b", 1).is_none());
        }
    }
    #[test]
    fn direct_identity_ownership_and_navigation_contract() {
        let mut j = job();
        j.source = "destination".into();
        j.thread = Some("target".into());
        let mut r = json!({"data":{"repository":{"pullRequest":{"headRefOid":j.head}},"node":{"id":"target","__typename":"IssueComment","pullRequest":{"number":1,"repository":{"nameWithOwner":"a/b"}},"createdAt":"now","body":"beyond excerpt"}}});
        assert!(j.validate().is_ok());
        let p = parse(&r, &j, 1).unwrap();
        assert_eq!(p.messages[0].id, "target");
        assert!(!p.complete);
        assert!(p.next.is_none());
        r["data"]["node"]["pullRequest"] = Value::Null;
        assert!(parse(&r, &j, 1).is_err());
        j.after = Some("cursor".into());
        assert!(j.validate().is_err());
        j.after = None;
        j.thread = Some("rest:3:19".into());
        assert!(j.validate().is_err());
        j.thread = Some("x".repeat(256));
        assert!(j.validate().is_ok());
        j.thread = Some("x".repeat(257));
        assert!(j.validate().is_err());
    }
    #[test]
    fn exact_coordinate_source_rejects_identity_owner_and_edited_body() {
        let j = job();
        let m = Message {
            id: "target".into(),
            kind: "reply".into(),
            author: None,
            time: "now".into(),
            url: None,
            body: Some("hello".into()),
            state: "complete".into(),
            bytes: Some(5),
            digest: Some(hash("hello")),
        };
        let v = json!({"node_id":"target","pull_request_url":"https://api.github.com/repos/a/b/pulls/1","body":"hello","path":"file.txt","side":"RIGHT","line":3,"position":4,"commit_id":j.head});
        let a = destination_anchor(&v, &j, &m).unwrap();
        assert_eq!(a.line, Some(3));
        assert!(!a.outdated);
        for (key, value) in [
            ("node_id", json!("neighbor")),
            (
                "pull_request_url",
                json!("https://api.github.com/repos/a/b/pulls/10"),
            ),
            ("body", json!("edited")),
        ] {
            let mut changed = v.clone();
            changed[key] = value;
            assert!(destination_anchor(&changed, &j, &m).is_err());
        }
        let mut old = v;
        old["position"] = Value::Null;
        assert!(destination_anchor(&old, &j, &m).unwrap().outdated);
    }
}

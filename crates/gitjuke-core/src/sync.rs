//! Read-only discovery plans and compact, revision-bound watch/check snapshots.
use crate::{
    Error, Result,
    github::{Response, string},
    model::*,
    now,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatchTarget {
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub generation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckTarget {
    pub page_size: u16,
    pub watch: WatchTarget,
    pub head: String,
    pub commit: String,
    pub cursor: String,
    pub total: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Range {
    pub start: u64,
    pub end: u64,
    pub page: u32,
    pub total: Option<u64>,
    pub received: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Discovery {
    pub page_size: u16,
    pub source: String,
    pub qualifier: String,
    pub cursor: Option<String>,
    pub page: u32,
    pub ranges: Vec<Range>,
    pub total: Option<u64>,
    pub received: u64,
}
impl Discovery {
    pub fn new(source: &str, qualifier: &str) -> Self {
        Self {
            page_size: 100,
            source: source.into(),
            qualifier: qualifier.into(),
            cursor: None,
            page: 1,
            ranges: vec![Range {
                start: 0,
                end: now(),
                page: 1,
                total: None,
                received: 0,
            }],
            total: None,
            received: 0,
        }
    }
    pub fn query(&self) -> String {
        let range = self.ranges.last().unwrap();
        format!(
            "is:pr is:open {} created:{}..{}",
            self.qualifier,
            date(range.start),
            date(range.end)
        )
    }
}
pub fn date(seconds: u64) -> String {
    // GitHub Search accepts UTC timestamps; no local timezone or locale dependence.
    let raw = seconds as libc::time_t;
    let mut time = std::mem::MaybeUninit::<libc::tm>::uninit();
    unsafe {
        libc::gmtime_r(&raw, time.as_mut_ptr());
        let t = time.assume_init();
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            t.tm_year + 1900,
            t.tm_mon + 1,
            t.tm_mday,
            t.tm_hour,
            t.tm_min,
            t.tm_sec
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadSnapshot {
    pub pr: PullRequest,
    pub head: String,
    pub merge: Option<String>,
    pub review: Option<String>,
    pub generation: u64,
    pub fetched_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckContext {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: Option<String>,
    pub suite_id: Option<String>,
    pub suite_status: Option<String>,
    pub suite_conclusion: Option<String>,
    pub suite_updated_at: Option<String>,
    #[serde(default)]
    pub source_created_at: Option<String>,
    pub run_id: Option<u64>,
    pub attempt: Option<u64>,
    pub commit: String,
}
#[derive(Clone, Debug)]
pub struct ContextPage {
    pub contexts: Vec<CheckContext>,
    pub next: Option<String>,
    pub total: u64,
    pub commit: String,
}
pub struct HeadRead {
    pub snapshot: HeadSnapshot,
    pub pages: Vec<ContextPage>,
}
pub const PR_FIELDS: &str = "id number title url state isDraft updatedAt author{login} repository{databaseId nameWithOwner}";
pub const CONTEXT_FIELDS: &str = "__typename ... on CheckRun{id name status conclusion detailsUrl checkSuite{id status conclusion updatedAt commit{oid} workflowRun{databaseId runAttempt}}} ... on StatusContext{id context state targetUrl createdAt}";
fn contexts(cursor: bool) -> String {
    format!(
        "statusCheckRollup{{contexts(first:$first{}){{totalCount pageInfo{{hasNextPage endCursor}} nodes{{{CONTEXT_FIELDS}}}}}}}",
        if cursor { ",after:$after" } else { "" }
    )
}
#[derive(Clone, Debug)]
pub enum GraphRead {
    Authored {
        cursor: Option<String>,
        size: u16,
    },
    Heads {
        ids: Vec<String>,
        size: u16,
    },
    Contexts {
        repo: String,
        commit: String,
        cursor: String,
        size: u16,
    },
}
impl GraphRead {
    pub fn body(&self) -> Value {
        let (query,variables)=match self {
            Self::Authored{cursor,size}=>("query Authored($after:String,$first:Int!){viewer{databaseId login pullRequests(first:$first,after:$after,states:OPEN,orderBy:{field:CREATED_AT,direction:ASC}){totalCount pageInfo{hasNextPage endCursor}nodes{FIELDS}}}rateLimit{cost remaining resetAt}}".replace("FIELDS",PR_FIELDS),json!({"after":cursor,"first":size})),
            Self::Heads{ids,size}=>("query WatchHeads($ids:[ID!]!,$first:Int!){viewer{databaseId login}nodes(ids:$ids){... on PullRequest{FIELDS headRefOid reviewDecision potentialMergeCommit{oid CHECKS} commits(last:1){nodes{commit{oid CHECKS}}}}}rateLimit{cost remaining resetAt}}".replace("FIELDS",PR_FIELDS).replace("CHECKS",&contexts(false)),json!({"ids":ids,"first":size})),
            Self::Contexts{repo,commit,cursor,size}=>{let(owner,name)=repo.split_once('/').unwrap_or_default();("query CheckContexts($owner:String!,$name:String!,$oid:GitObjectID!,$after:String,$first:Int!){viewer{databaseId login}repository(owner:$owner,name:$name){object(oid:$oid){... on Commit{oid CHECKS}}}rateLimit{cost remaining resetAt}}".replace("CHECKS",&contexts(true)),json!({"owner":owner,"name":name,"oid":commit,"after":cursor,"first":size}))}
        };
        json!({"query":query,"variables":variables})
    }
}
pub fn graph_data(response: &Response) -> Result<&Value> {
    response.require_ok()?;
    if response.body.get("errors").is_some() {
        return Err(Error::Safe("GitHub GraphQL returned errors; incomplete results did not advance successful progress.".into()));
    }
    response
        .body
        .get("data")
        .filter(|v| !v.is_null())
        .ok_or_else(schema)
}
pub fn pr(v: &Value) -> Result<PullRequest> {
    let repo = string(&v["repository"], "nameWithOwner")?;
    let number = num(v, "number")?;
    let url = string(v, "url")?;
    if !crate::github::valid_repo(&repo)
        || url != format!("https://github.com/{repo}/pull/{number}")
    {
        return Err(schema());
    }
    Ok(PullRequest {
        id: string(v, "id")?,
        repo_id: Some(num(&v["repository"], "databaseId")?),
        repo,
        number,
        url,
        title: string(v, "title")?,
        state: string(v, "state")?.to_ascii_lowercase(),
        draft: v["isDraft"].as_bool().ok_or_else(schema)?,
        author: v["author"]["login"].as_str().map(String::from),
        updated_at: string(v, "updatedAt")?,
    })
}
pub fn page_info(v: &Value) -> Result<Option<String>> {
    if v["pageInfo"]["hasNextPage"].as_bool().ok_or_else(schema)? {
        Ok(Some(string(&v["pageInfo"], "endCursor")?))
    } else {
        Ok(None)
    }
}
pub fn authored(r: &Response) -> Result<(Vec<PullRequest>, Option<String>, u64)> {
    let c = &graph_data(r)?["viewer"]["pullRequests"];
    let items = c["nodes"]
        .as_array()
        .ok_or_else(schema)?
        .iter()
        .map(pr)
        .collect::<Result<Vec<_>>>()?;
    if items.len() > 100 {
        return Err(schema());
    }
    Ok((items, page_info(c)?, num(c, "totalCount")?))
}
pub fn head(v: &Value, target: &WatchTarget) -> Result<HeadRead> {
    let pr = pr(v)?;
    if pr.id != target.id || pr.repo != target.repo || pr.number != target.number {
        return Err(schema());
    }
    let head = string(v, "headRefOid")?;
    let mut pages = vec![];
    let commit = &v["commits"]["nodes"][0]["commit"];
    if !commit.is_null() {
        if string(commit, "oid")? != head {
            return Err(schema());
        }
        pages.push(context_page(commit)?);
    } else if pr.state == "open" {
        return Err(Error::Safe(
            "Current head commit is unavailable; previous check history is retained as old.".into(),
        ));
    }
    let merge = v["potentialMergeCommit"]["oid"].as_str().map(String::from);
    if merge.as_ref().is_some_and(|m| m != &head) {
        pages.push(context_page(&v["potentialMergeCommit"])?);
    }
    Ok(HeadRead {
        snapshot: HeadSnapshot {
            pr,
            head,
            merge,
            review: v["reviewDecision"].as_str().map(String::from),
            generation: target.generation,
            fetched_at: now(),
        },
        pages,
    })
}
pub fn context_page(v: &Value) -> Result<ContextPage> {
    let commit = string(v, "oid")?;
    let rollup = &v["statusCheckRollup"];
    if rollup.is_null() {
        return Ok(ContextPage {
            contexts: vec![],
            next: None,
            total: 0,
            commit,
        });
    }
    let c = &rollup["contexts"];
    let total = num(c, "totalCount")?;
    let nodes = c["nodes"].as_array().ok_or_else(schema)?;
    if nodes.len() > 25 {
        return Err(schema());
    }
    let contexts=nodes.iter().map(|node|{
        let kind=string(node,"__typename")?;let is_run=kind=="CheckRun";if !is_run&&kind!="StatusContext"{return Err(schema())}
        let suite=&node["checkSuite"];let workflow=&suite["workflowRun"];
        if is_run&&suite["commit"]["oid"].as_str()!=Some(&commit){return Err(Error::Safe("Check suite commit does not match the queried commit; no branch-name inference was used.".into()))}
        Ok(CheckContext{id:string(node,"id")?,kind,name:string(node,if is_run{"name"}else{"context"})?,status:string(node,if is_run{"status"}else{"state"})?,conclusion:node["conclusion"].as_str().map(String::from),url:node[if is_run{"detailsUrl"}else{"targetUrl"}].as_str().filter(|s|valid_link(s)).map(String::from),suite_id:suite["id"].as_str().map(String::from),suite_status:suite["status"].as_str().map(String::from),suite_conclusion:suite["conclusion"].as_str().map(String::from),suite_updated_at:suite["updatedAt"].as_str().map(String::from),source_created_at:node["createdAt"].as_str().map(String::from),run_id:if workflow.is_null(){None}else{Some(num(workflow,"databaseId")?)},attempt:if workflow.is_null(){None}else{Some(num(workflow,"runAttempt")?)},commit:commit.clone()})
    }).collect::<Result<Vec<_>>>()?;
    Ok(ContextPage {
        contexts,
        next: page_info(c)?,
        total,
        commit,
    })
}
fn valid_link(v: &str) -> bool {
    v.starts_with("https://")
        && !v.chars().any(char::is_control)
        && !v[8..].split('/').next().unwrap_or_default().contains('@')
}
pub fn num(v: &Value, k: &str) -> Result<u64> {
    v[k].as_u64()
        .or_else(|| v[k].as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(schema)
}
pub fn schema() -> Error {
    Error::Safe(
        "Unsupported or inconsistent watch/check schema; successful progress was not advanced."
            .into(),
    )
}

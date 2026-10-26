use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub id: String,
    pub repo_id: Option<u64>,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub draft: bool,
    pub author: Option<String>,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    #[serde(default)]
    pub repo_id: Option<u64>,
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub body: Option<String>,
    pub body_limited: bool,
    pub head: String,
    pub base: String,
    pub state: String,
    pub draft: bool,
    pub author: Option<String>,
    pub stack: StackState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum StackState {
    Unknown {
        reason: String,
    },
    None,
    Published {
        number: u64,
        trunk: String,
        members: Vec<StackMember>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackMember {
    pub number: u64,
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot<T> {
    pub data: T,
    pub fetched_at: Option<u64>,
    pub complete: bool,
    pub error: Option<String>,
    pub revision: u64,
}
/// Published stack metadata bundled with a PR list, scoped to its repository.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestStack {
    pub repo: String,
    pub stack: StackState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<PullRequest>,
    #[serde(default)]
    pub stacks: Vec<PullRequestStack>,
    pub next_offset: Option<usize>,
    pub fetched_at: Option<u64>,
    pub complete: bool,
    pub error: Option<String>,
    pub revision: u64,
}

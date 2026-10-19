use crate::{PullRequest, ResourceHealth};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckPage {
    pub pr: Option<PullRequest>,
    pub error: Option<String>,
    pub review: Option<String>,
    pub items: Vec<CheckContext>,
    pub next_offset: Option<usize>,
    pub head: Option<String>,
    pub commits: Vec<ResourceHealth>,
    pub revision: u64,
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

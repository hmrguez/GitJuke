//! Bounded supplied change hunks, never reconstructed source files.
use crate::{
    Error, Result,
    github::{self, Response},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const AUTO_BYTES: usize = 512 * 1024;
pub const AUTO_ROWS: usize = 5_000;
pub const HARD_BYTES: usize = 2 * 1024 * 1024;
pub const HARD_ROWS: usize = 20_000;
pub const LINE_BYTES: usize = 16 * 1024;
pub const FILE_PAGE: u16 = 100;
pub const CHUNK_BYTES: usize = 96 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesJob {
    pub repo: String,
    pub number: u64,
    pub head: String,
    pub base: Option<String>,
    pub page: u32,
    pub index: Option<u32>,
    pub path: Option<String>,
    pub large: bool,
    pub seq: u64,
}
impl ChangesJob {
    pub fn validate(&self) -> Result<()> {
        if !github::valid_repo(&self.repo)
            || self.number == 0
            || self.number > i64::MAX as u64
            || !valid_sha(&self.head)
            || self.base.as_ref().is_some_and(|s| !valid_sha(s))
            || self.page == 0
            || self.page > 150
            || self.index.is_some_and(|i| i >= 3000)
            || self.index.is_some() != self.path.is_some()
            || self
                .path
                .as_ref()
                .is_some_and(|p| p.is_empty() || p.len() > 16 * 1024)
        {
            return Err(Error::Safe("Invalid Changes request.".into()));
        }
        Ok(())
    }
    pub fn key(&self) -> String {
        format!(
            "api:repos/{}/pulls/{}/gitjuke-changes/{}/{}/{}/{}",
            self.repo,
            self.number,
            self.head,
            self.base.as_deref().unwrap_or("discover"),
            self.index
                .map(|i| format!("patch-{i}-{}", self.large))
                .unwrap_or_else(|| format!("files-v2-{}", self.page)),
            self.path.as_ref().map(|s| hash(s)).unwrap_or_default()
        )
    }
}
pub fn valid_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub index: u32,
    pub path: String,
    pub previous_path: Option<String>,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub url: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Files {
    pub head: String,
    pub base: String,
    pub total: u64,
    pub items: Vec<File>,
    pub next_page: Option<u32>,
    pub complete: bool,
    pub limited: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    pub head: String,
    pub base: String,
    pub file: File,
    pub state: String,
    pub bytes: Option<usize>,
    pub rows: Option<usize>,
    pub chunks: usize,
    pub digest: Option<String>,
}
pub fn revision(r: &Response, repo: &str, number: u64) -> Result<(String, String, u64)> {
    let pr = github::parse_overview(r, repo, number)?;
    let base = github::string(&r.body["base"], "sha")?;
    let count = r.body["changed_files"].as_u64().ok_or_else(bad)?;
    if !valid_sha(&pr.head) || !valid_sha(&base) {
        return Err(bad());
    }
    Ok((pr.head, base, count))
}
fn bad() -> Error {
    Error::Safe(
        "Changes source is incomplete or has an unsupported schema. Retry or open GitHub Changes."
            .into(),
    )
}
fn path_url(path: &str) -> String {
    path.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
pub fn file(v: &serde_json::Value, index: u32, repo: &str, head: &str, base: &str) -> Result<File> {
    let path = github::string(v, "filename")?;
    if path.is_empty() || path.len() > 16 * 1024 {
        return Err(bad());
    }
    let status = github::string(v, "status")?;
    let sha = if status == "removed" { base } else { head };
    let expected = format!("https://github.com/{repo}/blob/{sha}/{}", path_url(&path));
    // Never synthesize an exact file destination; accept only a supplied, revision-matching URL.
    let url = v["blob_url"]
        .as_str()
        .filter(|s| *s == expected)
        .map(String::from);
    Ok(File {
        index,
        path,
        previous_path: v["previous_filename"].as_str().map(String::from),
        status,
        additions: v["additions"].as_u64().ok_or_else(bad)?,
        deletions: v["deletions"].as_u64().ok_or_else(bad)?,
        url,
    })
}
pub fn files(r: &Response, job: &ChangesJob, base: &str, total: u64, size: u16) -> Result<Files> {
    r.require_ok()?;
    let a = r.body.as_array().ok_or_else(bad)?;
    let start = (job.page - 1) * u32::from(size);
    if a.len() > size as usize || u64::from(start) + a.len() as u64 > total {
        return Err(bad());
    }
    let end = u64::from(start) + a.len() as u64;
    if a.is_empty() && end < total.min(3000) {
        return Err(bad());
    }
    let next = end < total.min(3000);
    if next && a.len() < size as usize {
        return Err(bad());
    }
    let items = a
        .iter()
        .enumerate()
        .map(|(i, v)| file(v, start + i as u32, &job.repo, &job.head, base))
        .collect::<Result<_>>()?;
    Ok(Files {
        head: job.head.clone(),
        base: base.into(),
        total,
        items,
        next_page: next.then_some(job.page + 1),
        complete: start == 0 && end == total && total <= 3000,
        limited: total > 3000,
    })
}
/// Prove that every supplied hunk is closed and that change counts match metadata.
/// This establishes complete supplied change hunks, never whole-file source/context.
pub fn complete_patch(s: &str, additions: u64, deletions: u64) -> bool {
    let (mut old_left, mut new_left, mut adds, mut dels) = (0u64, 0u64, 0u64, 0u64);
    let mut hunks = 0;
    for line in s.split_terminator('\n') {
        if line.starts_with("@@ ") {
            if old_left != 0 || new_left != 0 {
                return false;
            }
            let parts: Vec<_> = line.splitn(5, ' ').collect();
            if parts.len() < 4 || parts[3] != "@@" {
                return false;
            }
            let range = |s: &str, sign: char| -> Option<u64> {
                let s = s.strip_prefix(sign)?;
                if let Some((start, count)) = s.split_once(',') {
                    start.parse::<u64>().ok()?;
                    count.parse().ok()
                } else {
                    s.parse::<u64>().ok()?;
                    Some(1)
                }
            };
            let (Some(o), Some(n)) = (range(parts[1], '-'), range(parts[2], '+')) else {
                return false;
            };
            old_left = o;
            new_left = n;
            hunks += 1;
        } else if line == "\\ No newline at end of file" {
            if hunks == 0 {
                return false;
            }
        } else {
            let (o, n) = match line.as_bytes().first() {
                Some(b' ') => (1, 1),
                Some(b'+') => {
                    adds += 1;
                    (0, 1)
                }
                Some(b'-') => {
                    dels += 1;
                    (1, 0)
                }
                _ => return false,
            };
            if hunks == 0 || old_left < o || new_left < n {
                return false;
            }
            old_left -= o;
            new_left -= n;
        }
    }
    hunks > 0 && old_left == 0 && new_left == 0 && adds == additions && dels == deletions
}
pub fn patch(
    v: &serde_json::Value,
    file: File,
    head: &str,
    base: &str,
    large: bool,
) -> (Patch, Vec<String>) {
    let text = v["patch"].as_str();
    let bytes = text.map(str::len);
    let rows = text.map(|s| s.split_terminator('\n').count());
    let state = match text {
        None => "missing",
        Some(s) if s.starts_with("Binary files ") || s.starts_with("GIT binary patch") => "binary",
        Some(s)
            if s.len() > HARD_BYTES
                || rows.unwrap_or(0) > HARD_ROWS
                || s.split_terminator('\n').any(|l| l.len() > LINE_BYTES) =>
        {
            "tooLarge"
        }
        Some(s) if !complete_patch(s, file.additions, file.deletions) => "incomplete",
        Some(s) if !large && (s.len() > AUTO_BYTES || rows.unwrap_or(0) > AUTO_ROWS) => "large",
        Some(_) => "complete",
    };
    let mut chunks = Vec::new();
    if state == "complete" {
        let mut rest = text.unwrap_or_default();
        while !rest.is_empty() {
            let mut end = rest.len().min(CHUNK_BYTES);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            chunks.push(rest[..end].to_owned());
            rest = &rest[end..];
        }
    }
    (
        Patch {
            head: head.into(),
            base: base.into(),
            file,
            state: state.into(),
            bytes,
            rows,
            chunks: chunks.len(),
            digest: if state == "complete" {
                text.map(hash)
            } else {
                None
            },
        },
        chunks,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn f() -> File {
        File {
            index: 0,
            path: "a.txt".into(),
            previous_path: None,
            status: "added".into(),
            additions: 1,
            deletions: 0,
            url: None,
        }
    }
    fn state(text: Option<&str>, adds: u64, large: bool) -> Patch {
        let mut file = f();
        file.additions = adds;
        patch(
            &json!({"patch":text}),
            file,
            &"a".repeat(40),
            &"b".repeat(40),
            large,
        )
        .0
    }
    fn sized(bytes: usize, rows: usize) -> String {
        let n = rows - 1;
        let header = format!("@@ -0,0 +1,{n} @@\n");
        let mut out = header;
        let mut remaining = bytes - out.len() - n * 2 + 1;
        for i in 0..n {
            out.push('+');
            let fill = remaining.min(LINE_BYTES - 1);
            out.push_str(&"x".repeat(fill));
            remaining -= fill;
            if i + 1 < n {
                out.push('\n');
            }
        }
        assert_eq!(remaining, 0);
        assert_eq!(out.len(), bytes);
        out
    }
    #[test]
    fn inclusive_utf8_bytes_rows_and_logical_line_limits() {
        for (bytes, rows, large, expected) in [
            (AUTO_BYTES, 101, false, "complete"),
            (AUTO_BYTES + 1, 101, false, "large"),
            (HARD_BYTES, 201, true, "complete"),
            (HARD_BYTES + 1, 201, true, "tooLarge"),
        ] {
            let s = sized(bytes, rows);
            assert_eq!(state(Some(&s), (rows - 1) as u64, large).state, expected);
        }
        for (rows, large, expected) in [
            (AUTO_ROWS, false, "complete"),
            (AUTO_ROWS + 1, false, "large"),
            (HARD_ROWS, true, "complete"),
            (HARD_ROWS + 1, true, "tooLarge"),
        ] {
            let s = format!(
                "@@ -0,0 +1,{} @@\n{}",
                rows - 1,
                vec!["+x"; rows - 1].join("\n")
            );
            assert_eq!(state(Some(&s), (rows - 1) as u64, large).state, expected);
        }
        for (len, expected) in [(LINE_BYTES, "complete"), (LINE_BYTES + 1, "tooLarge")] {
            let s = format!(
                "@@ -0,0 +1 @@\n+{}",
                "é".repeat((len - 1) / 2) + if len % 2 == 0 { "x" } else { "" }
            );
            assert_eq!(state(Some(&s), 1, true).state, expected);
        }
    }
    #[test]
    fn source_completeness_is_not_inferred_from_presence_or_missing_text() {
        assert_eq!(state(None, 1, false).state, "missing");
        assert_eq!(
            state(Some("Binary files a and b differ"), 0, false).state,
            "binary"
        );
        for s in [
            "@@ -0,0 +1,2 @@\n+one",
            "@@ -0,0 +1 @@\n+one\n+two",
            "@@ -1 +1 @@\n+one",
            "",
            "+one",
            "@@ -0,0 +1 @@\n+one\n...truncated",
        ] {
            assert_eq!(state(Some(s), 1, true).state, "incomplete", "{s}");
        }
        assert!(complete_patch(
            "@@ -1,2 +1,2 @@\n-a\n+b\n c\n\\ No newline at end of file",
            1,
            1
        ));
        assert!(!complete_patch("@@ -0,0 +1 @@\n+x", 2, 0));
        assert!(complete_patch("@@ -1 +0,0 @@\n-old\n@@ -3 +2 @@\n x", 0, 1));
    }
    #[test]
    fn chunks_preserve_every_utf8_byte_and_fit_worst_case_ipc_encoding() {
        let s = format!(
            "@@ -0,0 +1,200 @@\n{}",
            vec![format!("+{}", "é\t\"\\".repeat(2000)); 200].join("\n")
        );
        let mut file = f();
        file.additions = 200;
        let (p, chunks) = patch(
            &json!({"patch":s}),
            file,
            &"a".repeat(40),
            &"b".repeat(40),
            true,
        );
        assert_eq!(p.state, "complete");
        assert_eq!(chunks.concat(), s);
        assert_eq!(p.digest, Some(hash(&s)));
        for chunk in &chunks {
            assert!(chunk.len() <= CHUNK_BYTES);
            assert!(serde_json::to_vec(chunk).unwrap().len() < 1024 * 1024);
        }
        assert_eq!(p.rows, Some(201));
    }
    fn job(page: u32) -> ChangesJob {
        ChangesJob {
            repo: "a/b".into(),
            number: 1,
            head: "a".repeat(40),
            base: None,
            page,
            index: None,
            path: None,
            large: false,
            seq: 0,
        }
    }
    #[test]
    fn metadata_pages_keep_indices_totals_and_upstream_ceiling_honest() {
        for (page, total, count, next, complete, limited) in [
            (1, 245, 100, Some(2), false, false),
            (2, 245, 100, Some(3), false, false),
            (3, 245, 45, None, false, false),
            (1, 20, 20, None, true, false),
            (30, 3001, 100, None, false, true),
        ] {
            let values=(0..count).map(|i|json!({"filename":format!("f{i}"),"status":"modified","additions":1,"deletions":1})).collect();
            let r = Response {
                status: 200,
                headers: Default::default(),
                body: serde_json::Value::Array(values),
            };
            let out = files(&r, &job(page), &"b".repeat(40), total, FILE_PAGE).unwrap();
            assert_eq!(out.items.len(), count);
            assert_eq!(out.items[0].index, (page - 1) * u32::from(FILE_PAGE));
            assert_eq!(out.next_page, next);
            assert_eq!(out.complete, complete);
            assert_eq!(out.limited, limited);
        }
        let short = Response {
            status: 200,
            headers: Default::default(),
            body: json!([]),
        };
        assert!(files(&short, &job(2), &"b".repeat(40), 45, FILE_PAGE).is_err());
        assert!(job(151).validate().is_err());
    }
    #[test]
    fn fallback_rejects_untrusted_or_wrong_revision_file_links() {
        let head = "a".repeat(40);
        let base = "b".repeat(40);
        for url in [
            "https://evil.example/a",
            "https://github.com/a/b/blob/main/file.txt",
            "https://github.com/a/b/pull/1/files#invented",
            "file:///tmp/source",
        ] {
            let v = json!({"filename":"file.txt","status":"modified","additions":0,"deletions":0,"blob_url":url});
            assert!(file(&v, 0, "a/b", &head, &base).unwrap().url.is_none());
        }
        let v = json!({"filename":"dir/a b.txt","status":"removed","additions":0,"deletions":1,"blob_url":format!("https://github.com/a/b/blob/{base}/dir/a%20b.txt")});
        assert!(file(&v, 0, "a/b", &head, &base).unwrap().url.is_some());
    }
}

//! A small resource barrier hides denied Conversation while disposable rows are reclaimed.
use crate::{
    Error, Result,
    store::{Store, bump},
};
use rusqlite::{OptionalExtension, params};
pub(crate) fn root(repo: &str, number: u64) -> String {
    format!("api:repos/{repo}/pulls/{number}/gitjuke-conversation/")
}
fn marker(repo: &str, number: u64) -> String {
    format!("conversation-denied:{}:{number}", repo.to_ascii_lowercase())
}
impl Store {
    pub(crate) fn conversation_denied(&self, repo: &str, number: u64) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM resource_state WHERE key=?",
                [marker(repo, number)],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
    pub(crate) fn deny_conversation(&mut self, repo: &str, number: u64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO resource_state(key,complete,error) VALUES(?,0,'pending') ON CONFLICT(key) DO UPDATE SET error='pending'",[marker(repo,number)])?;
        tx.execute(
            "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='generation'",
            [],
        )?;
        bump(&tx)?;
        tx.commit()?;
        self.clean_conversation(repo, number)
    }
    pub(crate) fn clean_conversation(&mut self, repo: &str, number: u64) -> Result<()> {
        let prefix = root(repo, number);
        let end = format!("{}0", prefix.trim_end_matches('/'));
        loop {
            let tx = self.conn.transaction()?;
            let before = tx.total_changes();
            // Content accounting triggers share the direct mutation allowance.
            let deleted=tx.execute("DELETE FROM content WHERE key IN (SELECT key FROM content WHERE key>=? COLLATE NOCASE AND key<? COLLATE NOCASE ORDER BY key LIMIT 32)",params![prefix,end])?;
            let excerpts=tx.execute("DELETE FROM content WHERE key IN (SELECT c.key FROM content c JOIN activity a ON c.key='activity-payload:'||a.id WHERE a.repo=?1 COLLATE NOCASE AND a.number=?2 AND a.kind!='check' ORDER BY c.key LIMIT 8)",params![repo,number as i64])?;
            if deleted == 0 && excerpts == 0 {
                tx.execute(
                    "UPDATE resource_state SET error='denied' WHERE key=? AND error!='denied'",
                    [marker(repo, number)],
                )?;
            }
            bump(&tx)?;
            if tx.total_changes() - before > 128 {
                return Err(Error::StorageUnavailable(
                    "Conversation cleanup exceeded its mutation allowance.".into(),
                ));
            }
            tx.commit()?;
            if deleted == 0 && excerpts == 0 {
                break;
            }
        }
        Ok(())
    }
    pub(crate) fn restore_conversation(&mut self, repo: &str, number: u64) -> Result<()> {
        let tx = self.conn.transaction()?;
        if tx.execute(
            "DELETE FROM resource_state WHERE key=?",
            [marker(repo, number)],
        )? > 0
        {
            bump(&tx)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn resume_conversation_cleanup(&mut self) -> Result<()> {
        let mut after = "conversation-denied:".to_owned();
        loop {
            let rows: Vec<(String, String)> = {
                let mut q=self.conn.prepare("SELECT key,error FROM resource_state WHERE key>? AND key<'conversation-denied;' ORDER BY key LIMIT 32")?;
                q.query_map([&after], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?
            };
            if rows.is_empty() {
                break;
            }
            for (key, state) in &rows {
                if state != "pending" {
                    continue;
                }
                let Some((repo, number)) = key
                    .trim_start_matches("conversation-denied:")
                    .rsplit_once(':')
                else {
                    return Err(Error::Safe("Invalid Conversation cleanup marker.".into()));
                };
                self.clean_conversation(
                    repo,
                    number
                        .parse()
                        .map_err(|_| Error::Safe("Invalid Conversation cleanup number.".into()))?,
                )?;
            }
            after = rows.last().unwrap().0.clone();
            if rows.len() < 32 {
                break;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_denial_cleanup_restarts_preserving_overview_neighbors_and_history() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        let g = s.generation().unwrap();
        for n in 0..257 {
            s.put(&format!("{}{:04}", root("a/b", 1), n), &"body", g)
                .unwrap();
        }
        s.put("pr:a/b:1", &"overview", g).unwrap();
        s.put(&format!("{}keep", root("a/b", 10)), &"neighbor", g)
            .unwrap();
        s.conn.execute_batch("INSERT INTO activity VALUES('A','a/b',1,'comment',4070908800,1,'eligible',1); INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A');").unwrap();
        s.put("activity-payload:A", &"private excerpt", g).unwrap();
        s.conn.execute_batch("CREATE TRIGGER fail_conversation BEFORE DELETE ON content WHEN OLD.key LIKE '%gitjuke-conversation/0140' BEGIN SELECT RAISE(ABORT,'fixture cleanup fault'); END;").unwrap();
        assert!(s.deny_conversation("a/b", 1).is_err());
        assert!(s.conversation_denied("a/b", 1).unwrap());
        assert!(s.conversation_denied("A/B", 1).unwrap());
        assert!(s.activity_page(0).unwrap().items[0].text.is_none());
        let count: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM content WHERE instr(key,'gitjuke-conversation/')>0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 130); // 129 remaining + neighbor
        drop(s);
        assert!(Store::open(&path).is_err());
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute_batch("DROP TRIGGER fail_conversation").unwrap();
        drop(raw);
        let mut s = Store::open(&path).unwrap();
        assert!(s.conversation_denied("a/b", 1).unwrap());
        assert!(s.conversation_denied("A/B", 1).unwrap());
        assert!(
            s.get::<String>("activity-payload:A")
                .unwrap()
                .data
                .is_none()
        );
        assert_eq!(
            s.get::<String>("pr:a/b:1").unwrap().data.as_deref(),
            Some("overview")
        );
        assert_eq!(
            s.get::<String>(&format!("{}keep", root("a/b", 10)))
                .unwrap()
                .data
                .as_deref(),
            Some("neighbor")
        );
        assert_eq!(
            s.conn
                .query_row("SELECT seen FROM seen WHERE source_id='A'", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            s.conn
                .query_row("SELECT count(*) FROM dedup", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

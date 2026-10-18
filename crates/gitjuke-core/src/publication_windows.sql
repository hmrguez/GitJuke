-- Schema 14: a candidate cursor belongs to the publication's current plan step.
-- Older pending preparations restart from step zero; retained rows stay protected.
ALTER TABLE publication ADD COLUMN scan_after INTEGER;
ALTER TABLE publication ADD COLUMN scan_ceiling INTEGER;
UPDATE publication SET step=0 WHERE phase='planning';
CREATE INDEX watch_source_source_nocase ON watch_source(source COLLATE NOCASE,id);
CREATE INDEX watch_source_reason ON watch_source(id,source COLLATE NOCASE);
CREATE INDEX pending_key_nocase ON pending(key COLLATE NOCASE);
CREATE INDEX watch_source_source ON watch_source(source,id);
CREATE INDEX watch_locator_location ON watch_locator(repo,number,id);
CREATE INDEX check_staged_keys ON check_context(pr_id,commit_sha,id) WHERE staged=1;
-- The frontier already costs one mutation per published insertion/deletion.
-- Maintain its count in that same row, without a separate accounting write.
ALTER TABLE shared_check_lru ADD COLUMN contexts INTEGER NOT NULL DEFAULT 0 CHECK(typeof(contexts)='integer' AND contexts>=0);
UPDATE shared_check_lru SET contexts=(SELECT COUNT(*) FROM check_context c INDEXED BY check_published_keys WHERE c.pr_id=shared_check_lru.pr_id AND c.commit_sha=shared_check_lru.commit_sha AND c.staged=0);
DROP TRIGGER shared_context_insert;
DROP TRIGGER shared_context_delete;
DROP TRIGGER shared_context_update;
CREATE TRIGGER shared_context_insert AFTER INSERT ON check_context WHEN NEW.staged=0 BEGIN
    INSERT INTO shared_check_lru(pr_id,commit_sha,accessed,first_id,contexts)
    VALUES(NEW.pr_id,NEW.commit_sha,COALESCE((SELECT accessed FROM check_state WHERE pr_id=NEW.pr_id AND commit_sha=NEW.commit_sha),0),NEW.id,1)
    ON CONFLICT(pr_id,commit_sha) DO UPDATE SET first_id=MIN(first_id,excluded.first_id),contexts=contexts+1;
END;
CREATE TRIGGER shared_context_delete AFTER DELETE ON check_context WHEN OLD.staged=0 BEGIN
    DELETE FROM shared_check_lru WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND contexts=1;
    UPDATE shared_check_lru SET contexts=contexts-1,first_id=CASE WHEN first_id=OLD.id THEN (SELECT id FROM check_context INDEXED BY check_published_keys WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND staged=0 ORDER BY id LIMIT 1) ELSE first_id END WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha;
END;
CREATE TRIGGER shared_context_update AFTER UPDATE OF pr_id,commit_sha,id,staged ON check_context
WHEN (OLD.pr_id,OLD.commit_sha,OLD.id,OLD.staged) IS NOT (NEW.pr_id,NEW.commit_sha,NEW.id,NEW.staged) BEGIN
    DELETE FROM shared_check_lru WHERE OLD.staged=0 AND pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND contexts=1;
    UPDATE shared_check_lru SET contexts=contexts-1,first_id=CASE WHEN first_id=OLD.id THEN (SELECT id FROM check_context INDEXED BY check_published_keys WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND staged=0 ORDER BY id LIMIT 1) ELSE first_id END WHERE OLD.staged=0 AND pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha;
    INSERT INTO shared_check_lru(pr_id,commit_sha,accessed,first_id,contexts)
    SELECT NEW.pr_id,NEW.commit_sha,COALESCE((SELECT accessed FROM check_state WHERE pr_id=NEW.pr_id AND commit_sha=NEW.commit_sha),0),NEW.id,1 WHERE NEW.staged=0
    ON CONFLICT(pr_id,commit_sha) DO UPDATE SET first_id=MIN(first_id,excluded.first_id),contexts=contexts+1;
END;
PRAGMA user_version=14;

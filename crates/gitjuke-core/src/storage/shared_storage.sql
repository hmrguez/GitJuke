-- Schema 13. Migration builds exact state in the existing FULL-synchronous
-- upgrade transaction. Runtime never backfills incrementally or accepts a
-- partial counter/frontier as ready.
CREATE TABLE shared_bytes(
    id INTEGER PRIMARY KEY CHECK(id=1),
    total INTEGER NOT NULL CHECK(typeof(total)='integer' AND total>=0),
    protected INTEGER NOT NULL CHECK(typeof(protected)='integer' AND protected>=0 AND protected<=total),
    invalid INTEGER NOT NULL CHECK(typeof(invalid)='integer' AND invalid>=0)
);
INSERT INTO shared_bytes VALUES(1,0,0,0);
-- Exactly one row per nonempty published snapshot, including missing-state
-- snapshots at timestamp zero. Empty/staged-only states have no index entry.
CREATE TABLE shared_check_lru(
    pr_id TEXT NOT NULL,commit_sha TEXT NOT NULL,
    accessed INTEGER NOT NULL,first_id TEXT NOT NULL,
    PRIMARY KEY(pr_id,commit_sha)
);
CREATE INDEX shared_check_order ON shared_check_lru(accessed,pr_id,commit_sha);
INSERT INTO shared_check_lru
SELECT c.pr_id,c.commit_sha,COALESCE(s.accessed,0),MIN(c.id)
FROM check_context c LEFT JOIN check_state s USING(pr_id,commit_sha)
WHERE c.staged=0 GROUP BY c.pr_id,c.commit_sha;


-- content: zero charge deltas perform no bookkeeping mutation.
UPDATE shared_bytes SET (total,protected,invalid)=(
    SELECT shared_bytes.total+COALESCE(SUM(octet_length(x.json)),0),
           shared_bytes.protected+COALESCE(SUM(0),0),
           shared_bytes.invalid+COALESCE(SUM(0),0)
    FROM content x
);

CREATE TRIGGER shared_content_insert AFTER INSERT ON content
WHEN ((octet_length(NEW.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_content_delete AFTER DELETE ON content
WHEN (-(octet_length(OLD.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+(-(octet_length(OLD.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_content_update AFTER UPDATE OF json ON content
WHEN ((octet_length(NEW.json))-(octet_length(OLD.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))-(octet_length(OLD.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

-- stage: zero charge deltas perform no bookkeeping mutation.
UPDATE shared_bytes SET (total,protected,invalid)=(
    SELECT shared_bytes.total+COALESCE(SUM(octet_length(x.json)),0),
           shared_bytes.protected+COALESCE(SUM(octet_length(x.json)),0),
           shared_bytes.invalid+COALESCE(SUM(0),0)
    FROM stage x
);

CREATE TRIGGER shared_stage_insert AFTER INSERT ON stage
WHEN ((octet_length(NEW.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))),protected=protected+((octet_length(NEW.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_stage_delete AFTER DELETE ON stage
WHEN (-(octet_length(OLD.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+(-(octet_length(OLD.json))),protected=protected+(-(octet_length(OLD.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_stage_update AFTER UPDATE OF json ON stage
WHEN ((octet_length(NEW.json))-(octet_length(OLD.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))-(octet_length(OLD.json))),protected=protected+((octet_length(NEW.json))-(octet_length(OLD.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

-- source_stage: zero charge deltas perform no bookkeeping mutation.
UPDATE shared_bytes SET (total,protected,invalid)=(
    SELECT shared_bytes.total+COALESCE(SUM(octet_length(x.json)),0),
           shared_bytes.protected+COALESCE(SUM(octet_length(x.json)),0),
           shared_bytes.invalid+COALESCE(SUM(0),0)
    FROM source_stage x
);

CREATE TRIGGER shared_source_stage_insert AFTER INSERT ON source_stage
WHEN ((octet_length(NEW.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))),protected=protected+((octet_length(NEW.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_source_stage_delete AFTER DELETE ON source_stage
WHEN (-(octet_length(OLD.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+(-(octet_length(OLD.json))),protected=protected+(-(octet_length(OLD.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_source_stage_update AFTER UPDATE OF json ON source_stage
WHEN ((octet_length(NEW.json))-(octet_length(OLD.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))-(octet_length(OLD.json))),protected=protected+((octet_length(NEW.json))-(octet_length(OLD.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

-- activity_stage: zero charge deltas perform no bookkeeping mutation.
UPDATE shared_bytes SET (total,protected,invalid)=(
    SELECT shared_bytes.total+COALESCE(SUM(octet_length(x.json)),0),
           shared_bytes.protected+COALESCE(SUM(octet_length(x.json)),0),
           shared_bytes.invalid+COALESCE(SUM(0),0)
    FROM activity_stage x
);

CREATE TRIGGER shared_activity_stage_insert AFTER INSERT ON activity_stage
WHEN ((octet_length(NEW.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))),protected=protected+((octet_length(NEW.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_activity_stage_delete AFTER DELETE ON activity_stage
WHEN (-(octet_length(OLD.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+(-(octet_length(OLD.json))),protected=protected+(-(octet_length(OLD.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_activity_stage_update AFTER UPDATE OF json ON activity_stage
WHEN ((octet_length(NEW.json))-(octet_length(OLD.json)))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))-(octet_length(OLD.json))),protected=protected+((octet_length(NEW.json))-(octet_length(OLD.json))) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

-- check_context: zero charge deltas perform no bookkeeping mutation.
UPDATE shared_bytes SET (total,protected,invalid)=(
    SELECT shared_bytes.total+COALESCE(SUM(octet_length(x.json)),0),
           shared_bytes.protected+COALESCE(SUM(CASE WHEN x.staged=1 THEN octet_length(x.json) ELSE 0 END),0),
           shared_bytes.invalid+COALESCE(SUM(0),0)
    FROM check_context x
);

CREATE TRIGGER shared_check_context_insert AFTER INSERT ON check_context
WHEN ((octet_length(NEW.json)))!=0 OR ((CASE WHEN NEW.staged=1 THEN octet_length(NEW.json) ELSE 0 END))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))),protected=protected+((CASE WHEN NEW.staged=1 THEN octet_length(NEW.json) ELSE 0 END)) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_check_context_delete AFTER DELETE ON check_context
WHEN (-(octet_length(OLD.json)))!=0 OR (-(CASE WHEN OLD.staged=1 THEN octet_length(OLD.json) ELSE 0 END))!=0
BEGIN
    UPDATE shared_bytes SET total=total+(-(octet_length(OLD.json))),protected=protected+(-(CASE WHEN OLD.staged=1 THEN octet_length(OLD.json) ELSE 0 END)) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_check_context_update AFTER UPDATE OF json,staged ON check_context
WHEN ((octet_length(NEW.json))-(octet_length(OLD.json)))!=0 OR ((CASE WHEN NEW.staged=1 THEN octet_length(NEW.json) ELSE 0 END)-(CASE WHEN OLD.staged=1 THEN octet_length(OLD.json) ELSE 0 END))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((octet_length(NEW.json))-(octet_length(OLD.json))),protected=protected+((CASE WHEN NEW.staged=1 THEN octet_length(NEW.json) ELSE 0 END)-(CASE WHEN OLD.staged=1 THEN octet_length(OLD.json) ELSE 0 END)) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

-- publication_row: zero charge deltas perform no bookkeeping mutation.
UPDATE shared_bytes SET (total,protected,invalid)=(
    SELECT shared_bytes.total+COALESCE(SUM(CASE WHEN (x.tbl='content' AND x.old_json IS NOT NULL) OR (x.tbl='check_context' AND json_extract(x.old_json,'$[4]')=0) THEN x.old_bytes ELSE 0 END),0),
           shared_bytes.protected+COALESCE(SUM(CASE WHEN (x.tbl='content' AND x.old_json IS NOT NULL) OR (x.tbl='check_context' AND json_extract(x.old_json,'$[4]')=0) THEN x.old_bytes ELSE 0 END),0),
           shared_bytes.invalid+COALESCE(SUM(0),0)
    FROM publication_row x
);

CREATE TRIGGER shared_publication_row_insert AFTER INSERT ON publication_row
WHEN ((CASE WHEN (NEW.tbl='content' AND NEW.old_json IS NOT NULL) OR (NEW.tbl='check_context' AND json_extract(NEW.old_json,'$[4]')=0) THEN NEW.old_bytes ELSE 0 END))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((CASE WHEN (NEW.tbl='content' AND NEW.old_json IS NOT NULL) OR (NEW.tbl='check_context' AND json_extract(NEW.old_json,'$[4]')=0) THEN NEW.old_bytes ELSE 0 END)),protected=protected+((CASE WHEN (NEW.tbl='content' AND NEW.old_json IS NOT NULL) OR (NEW.tbl='check_context' AND json_extract(NEW.old_json,'$[4]')=0) THEN NEW.old_bytes ELSE 0 END)) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_publication_row_delete AFTER DELETE ON publication_row
WHEN (-(CASE WHEN (OLD.tbl='content' AND OLD.old_json IS NOT NULL) OR (OLD.tbl='check_context' AND json_extract(OLD.old_json,'$[4]')=0) THEN OLD.old_bytes ELSE 0 END))!=0
BEGIN
    UPDATE shared_bytes SET total=total+(-(CASE WHEN (OLD.tbl='content' AND OLD.old_json IS NOT NULL) OR (OLD.tbl='check_context' AND json_extract(OLD.old_json,'$[4]')=0) THEN OLD.old_bytes ELSE 0 END)),protected=protected+(-(CASE WHEN (OLD.tbl='content' AND OLD.old_json IS NOT NULL) OR (OLD.tbl='check_context' AND json_extract(OLD.old_json,'$[4]')=0) THEN OLD.old_bytes ELSE 0 END)) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_publication_row_update AFTER UPDATE OF tbl,old_json,old_bytes ON publication_row
WHEN ((CASE WHEN (NEW.tbl='content' AND NEW.old_json IS NOT NULL) OR (NEW.tbl='check_context' AND json_extract(NEW.old_json,'$[4]')=0) THEN NEW.old_bytes ELSE 0 END)-(CASE WHEN (OLD.tbl='content' AND OLD.old_json IS NOT NULL) OR (OLD.tbl='check_context' AND json_extract(OLD.old_json,'$[4]')=0) THEN OLD.old_bytes ELSE 0 END))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((CASE WHEN (NEW.tbl='content' AND NEW.old_json IS NOT NULL) OR (NEW.tbl='check_context' AND json_extract(NEW.old_json,'$[4]')=0) THEN NEW.old_bytes ELSE 0 END)-(CASE WHEN (OLD.tbl='content' AND OLD.old_json IS NOT NULL) OR (OLD.tbl='check_context' AND json_extract(OLD.old_json,'$[4]')=0) THEN OLD.old_bytes ELSE 0 END)),protected=protected+((CASE WHEN (NEW.tbl='content' AND NEW.old_json IS NOT NULL) OR (NEW.tbl='check_context' AND json_extract(NEW.old_json,'$[4]')=0) THEN NEW.old_bytes ELSE 0 END)-(CASE WHEN (OLD.tbl='content' AND OLD.old_json IS NOT NULL) OR (OLD.tbl='check_context' AND json_extract(OLD.old_json,'$[4]')=0) THEN OLD.old_bytes ELSE 0 END)) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

-- publication: zero charge deltas perform no bookkeeping mutation.
UPDATE shared_bytes SET (total,protected,invalid)=(
    SELECT shared_bytes.total+COALESCE(SUM(COALESCE(gitjuke_input_bytes(x.kind,x.args),0)),0),
           shared_bytes.protected+COALESCE(SUM(COALESCE(gitjuke_input_bytes(x.kind,x.args),0)),0),
           shared_bytes.invalid+COALESCE(SUM(gitjuke_input_bytes(x.kind,x.args) IS NULL),0)
    FROM publication x
);

CREATE TRIGGER shared_publication_insert AFTER INSERT ON publication
WHEN ((COALESCE(gitjuke_input_bytes(NEW.kind,NEW.args),0)))!=0 OR ((gitjuke_input_bytes(NEW.kind,NEW.args) IS NULL))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((COALESCE(gitjuke_input_bytes(NEW.kind,NEW.args),0))),protected=protected+((COALESCE(gitjuke_input_bytes(NEW.kind,NEW.args),0))),invalid=invalid+((gitjuke_input_bytes(NEW.kind,NEW.args) IS NULL)) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_publication_delete AFTER DELETE ON publication
WHEN (-(COALESCE(gitjuke_input_bytes(OLD.kind,OLD.args),0)))!=0 OR (-(gitjuke_input_bytes(OLD.kind,OLD.args) IS NULL))!=0
BEGIN
    UPDATE shared_bytes SET total=total+(-(COALESCE(gitjuke_input_bytes(OLD.kind,OLD.args),0))),protected=protected+(-(COALESCE(gitjuke_input_bytes(OLD.kind,OLD.args),0))),invalid=invalid+(-(gitjuke_input_bytes(OLD.kind,OLD.args) IS NULL)) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

CREATE TRIGGER shared_publication_update AFTER UPDATE OF kind,args ON publication
WHEN ((COALESCE(gitjuke_input_bytes(NEW.kind,NEW.args),0))-(COALESCE(gitjuke_input_bytes(OLD.kind,OLD.args),0)))!=0 OR ((gitjuke_input_bytes(NEW.kind,NEW.args) IS NULL)-(gitjuke_input_bytes(OLD.kind,OLD.args) IS NULL))!=0
BEGIN
    UPDATE shared_bytes SET total=total+((COALESCE(gitjuke_input_bytes(NEW.kind,NEW.args),0))-(COALESCE(gitjuke_input_bytes(OLD.kind,OLD.args),0))),protected=protected+((COALESCE(gitjuke_input_bytes(NEW.kind,NEW.args),0))-(COALESCE(gitjuke_input_bytes(OLD.kind,OLD.args),0))),invalid=invalid+((gitjuke_input_bytes(NEW.kind,NEW.args) IS NULL)-(gitjuke_input_bytes(OLD.kind,OLD.args) IS NULL)) WHERE id=1;
    SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Missing shared accounting') END;
END;

-- Frontiers use point updates and indexed successor seeks, never a
-- timestamp fan-out across contexts. Every row write counts against its caller.
CREATE TRIGGER shared_context_insert AFTER INSERT ON check_context WHEN NEW.staged=0 BEGIN INSERT INTO shared_check_lru VALUES(NEW.pr_id,NEW.commit_sha,COALESCE((SELECT accessed FROM check_state WHERE pr_id=NEW.pr_id AND commit_sha=NEW.commit_sha),0),NEW.id) ON CONFLICT(pr_id,commit_sha) DO UPDATE SET first_id=excluded.first_id WHERE excluded.first_id<first_id; END;
CREATE TRIGGER shared_context_delete AFTER DELETE ON check_context WHEN OLD.staged=0 BEGIN DELETE FROM shared_check_lru WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND first_id=OLD.id AND NOT EXISTS(SELECT 1 FROM check_context INDEXED BY check_published_keys WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND staged=0);
UPDATE shared_check_lru SET first_id=(SELECT id FROM check_context INDEXED BY check_published_keys WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND staged=0 ORDER BY id LIMIT 1) WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND first_id=OLD.id; END;
CREATE TRIGGER shared_context_update AFTER UPDATE OF pr_id,commit_sha,id,staged ON check_context
WHEN (OLD.pr_id,OLD.commit_sha,OLD.id,OLD.staged) IS NOT (NEW.pr_id,NEW.commit_sha,NEW.id,NEW.staged) BEGIN DELETE FROM shared_check_lru WHERE OLD.staged=0 AND pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND first_id=OLD.id AND NOT EXISTS(SELECT 1 FROM check_context INDEXED BY check_published_keys WHERE OLD.staged=0 AND pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND staged=0);
UPDATE shared_check_lru SET first_id=(SELECT id FROM check_context INDEXED BY check_published_keys WHERE OLD.staged=0 AND pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND staged=0 ORDER BY id LIMIT 1) WHERE OLD.staged=0 AND pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha AND first_id=OLD.id; INSERT INTO shared_check_lru SELECT NEW.pr_id,NEW.commit_sha,COALESCE((SELECT accessed FROM check_state WHERE pr_id=NEW.pr_id AND commit_sha=NEW.commit_sha),0),NEW.id WHERE NEW.staged=0 ON CONFLICT(pr_id,commit_sha) DO UPDATE SET first_id=excluded.first_id WHERE excluded.first_id<first_id; END;
CREATE TRIGGER shared_state_insert AFTER INSERT ON check_state BEGIN UPDATE shared_check_lru SET accessed=NEW.accessed WHERE pr_id=NEW.pr_id AND commit_sha=NEW.commit_sha; END;
CREATE TRIGGER shared_state_update AFTER UPDATE OF accessed,pr_id,commit_sha ON check_state
WHEN (OLD.accessed,OLD.pr_id,OLD.commit_sha) IS NOT (NEW.accessed,NEW.pr_id,NEW.commit_sha) BEGIN UPDATE shared_check_lru SET accessed=COALESCE((SELECT accessed FROM check_state WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha),0) WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha; UPDATE shared_check_lru SET accessed=NEW.accessed WHERE pr_id=NEW.pr_id AND commit_sha=NEW.commit_sha AND (pr_id!=OLD.pr_id OR commit_sha!=OLD.commit_sha); END;
CREATE TRIGGER shared_state_delete AFTER DELETE ON check_state BEGIN UPDATE shared_check_lru SET accessed=0 WHERE pr_id=OLD.pr_id AND commit_sha=OLD.commit_sha; END;
PRAGMA user_version=13;

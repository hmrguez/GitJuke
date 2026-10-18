//! Every planning pass examines twelve candidates before eligibility tests and
//! point joins. Cursor/fence and prepared old images commit in one transaction.
use super::*;
use rusqlite::Transaction;
use serde_json::json;

#[derive(Clone, Copy)]
enum Step {
    SourceInput,
    SourceOld,
    SourceMembers,
    SourceLocators,
    SourceContent,
    SelectionOld,
    SelectionNew,
    HeadStage,
    HeadStates,
    ClosedSources,
    SelectedSource,
    Locator,
    ListContent,
    TeamsOldStage,
    TeamsInput,
    TeamSources,
    TeamPending,
    TeamsOld,
    TeamsNew,
    TeamsClear,
    CheckState,
    CheckOld,
    CheckStage,
    CheckInput,
    CheckClear,
    FlagsReset,
    FlagsCanonical,
    FlagsPrepared,
    Seen,
    Dedup,
    Activity,
    Intent,
    Member,
    Catchup,
    Payload,
    ActivityClear,
    Target,
    Progress,
    SummaryIntent,
    SummaryMember,
    SummaryClear,
    Baseline,
    Normal,
    Health,
}
impl Step {
    fn driver(self) -> Option<&'static str> {
        use Step::*;
        match self {
            SourceOld | SelectionOld | ClosedSources | TeamSources | FlagsCanonical => {
                Some("watch_source")
            }
            SourceMembers | SourceLocators | SourceContent => Some("source_stage"),
            SelectionNew => Some("watch_locator"),
            HeadStage | CheckOld | CheckStage | CheckClear => Some("check_context"),
            HeadStates => Some("check_state"),
            TeamsOldStage | TeamsNew | TeamsClear => Some("teams_stage"),
            TeamPending => Some("pending"),
            TeamsOld => Some("teams"),
            FlagsReset => Some("watch"),
            FlagsPrepared => Some("publication_row"),
            Seen | Dedup | Activity | Intent | Member | Catchup | Payload | ActivityClear => {
                Some("activity_stage")
            }
            SummaryIntent | SummaryMember | SummaryClear => Some("progress"),
            _ => None,
        }
    }
    fn array(self) -> Option<&'static str> {
        match self {
            Self::SourceInput => Some("prs"),
            Self::TeamsInput => Some("rows"),
            Self::CheckInput => Some("contexts"),
            _ => None,
        }
    }
}
fn steps(kind: &str, x: &Value) -> Vec<Step> {
    use Step::*;
    let flags = [FlagsReset, FlagsCanonical, FlagsPrepared];
    let mut out = match kind {
        "source_page" => vec![SourceInput],
        "source" => vec![SourceOld, SourceMembers, SourceLocators, SourceContent],
        "selection" => vec![Progress, SelectionOld, SelectionNew],
        "selected" => vec![SelectedSource, Locator, ListContent],
        "head" => {
            let mut v = vec![];
            if x["changed"] == true {
                v.extend([HeadStage, HeadStates]);
            }
            if x["pr"]["state"] != "open" {
                v.push(ClosedSources);
            }
            v.extend([Locator, ListContent]);
            v
        }
        "teams_page" => {
            let mut v = vec![];
            if x["first"] == true {
                v.push(TeamsOldStage);
            }
            v.push(TeamsInput);
            v
        }
        "teams" => vec![TeamSources, TeamPending, TeamsOld, TeamsNew],
        "checks" => {
            let mut v = vec![CheckState];
            if x["complete"] == true {
                v.extend([CheckOld, CheckStage, CheckInput, CheckClear]);
            } else {
                if x["first"] == true {
                    v.push(CheckClear);
                }
                v.push(CheckInput);
            }
            v
        }
        "activity" => {
            let mut v = vec![Seen, Dedup, Activity, Intent, Member];
            if x["mode"] == "recovery" {
                v.push(Catchup);
            }
            v.extend([Payload, ActivityClear, Target, Progress]);
            v
        }
        "activity_summary" => vec![SummaryIntent, SummaryMember, SummaryClear, Baseline, Normal],
        _ => vec![], // validated by the caller
    };
    if matches!(kind, "source" | "selection" | "selected" | "teams")
        || kind == "head" && x["pr"]["state"] != "open"
    {
        out.extend(flags);
    }
    if matches!(kind, "source" | "teams") {
        out.push(Health);
    }
    if kind == "teams" {
        out.push(TeamsClear);
    }
    out
}

pub(super) fn finished(kind: &str, x: &Value, step: i64) -> Result<bool> {
    let length = steps(kind, x).len();
    if step < 0 || step as usize > length {
        return Err(Error::StorageUnavailable(
            "Invalid publication planning step.".into(),
        ));
    }
    Ok(step as usize == length)
}

pub(super) fn prepare(
    tx: &Transaction<'_>,
    kind: &str,
    x: &Value,
    step: i64,
    after: Option<i64>,
    saved_ceiling: Option<i64>,
) -> Result<()> {
    let all = steps(kind, x);
    if step < 0
        || step as usize > all.len()
        || after.is_some() && saved_ceiling.is_none()
        || after.zip(saved_ceiling).is_some_and(|(a, b)| a > b)
    {
        return Err(Error::StorageUnavailable(
            "Invalid publication planning cursor; original database preserved.".into(),
        ));
    }
    let Some(&stage) = all.get(step as usize) else {
        tx.execute(
            "UPDATE publication SET phase='applying',scan_after=NULL,scan_ceiling=NULL",
            [],
        )?;
        return Ok(());
    };
    let mut candidates = Vec::new();
    let ceiling;
    let done;
    if let Some(driver) = stage.driver() {
        ceiling = match saved_ceiling {
            Some(n) => Some(n),
            None => tx
                .query_row(
                    &format!("SELECT rowid FROM main.{driver} ORDER BY rowid DESC LIMIT 1"),
                    [],
                    |r| r.get(0),
                )
                .optional()?,
        };
        if let Some(upper) = ceiling {
            // The candidate limit precedes all scope, equality and journal tests.
            let sql = window_sql(driver, after.is_some())?;
            let mut q = tx.prepare(&sql)?;
            candidates = q
                .query_map(params![after.unwrap_or(i64::MIN), upper], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            observe(&q, candidates.len());
        }
        done = candidates.len() < ROWS;
    } else if let Some(field) = stage.array() {
        let values = x[field]
            .as_array()
            .ok_or_else(|| Error::StorageUnavailable("Invalid publication input array.".into()))?;
        ceiling = Some(saved_ceiling.unwrap_or(values.len() as i64 - 1));
        if after.is_some_and(|a| a < 0) || ceiling.is_some_and(|n| n != values.len() as i64 - 1) {
            return Err(Error::StorageUnavailable(
                "Invalid publication input cursor.".into(),
            ));
        }
        let start = after.map_or(0, |a| a as usize + 1);
        let end = (start + ROWS).min(values.len());
        if start > values.len() {
            return Err(Error::StorageUnavailable(
                "Invalid publication input cursor.".into(),
            ));
        }
        candidates.extend(
            values[start..end]
                .iter()
                .enumerate()
                .map(|(i, v)| ((start + i) as i64, v.to_string())),
        );
        done = end == values.len();
    } else {
        ceiling = None;
        done = true;
        candidates.push((0, "null".into()));
    }
    for (position, data) in &candidates {
        project(tx, stage, kind, x, *position, &serde_json::from_str(data)?)?;
    }
    if done {
        tx.execute(
            "UPDATE publication SET step=step+1,scan_after=NULL,scan_ceiling=NULL",
            [],
        )?;
    } else {
        tx.execute(
            "UPDATE publication SET scan_after=?,scan_ceiling=?",
            params![candidates.last().unwrap().0, ceiling],
        )?;
    }
    #[cfg(test)]
    super::point("cursor_before");
    Ok(())
}

pub(super) fn window_sql(driver: &str, after: bool) -> Result<String> {
    // publication_row is operational and intentionally absent from reader tables.
    let columns = if driver == "publication_row" {
        "seq,tbl,k,new_json".to_string()
    } else {
        table(driver)?.columns.join(",")
    };
    Ok(format!(
        "SELECT rowid,json_array({columns}) FROM main.{driver} WHERE rowid{}?1 AND rowid<=?2 ORDER BY rowid LIMIT {ROWS}",
        if after { ">" } else { ">=" }
    ))
}
fn get(tx: &Connection, name: &str, keys: &Value) -> Result<Option<Value>> {
    let t = table(name)?;
    let mut q = tx.prepare(&format!(
        "SELECT {} FROM main.{name} b WHERE {}",
        row(t, "b"),
        pred(t, "b")
    ))?;
    let data: Option<String> = q.query_row([keys.to_string()], |r| r.get(0)).optional()?;
    observe(&q, usize::from(data.is_some()));
    data.map(|s| serde_json::from_str(&s).map_err(Error::from))
        .transpose()
}
fn emit(
    tx: &Connection,
    name: &str,
    keys: Value,
    new: Option<Value>,
    reference: Option<Value>,
) -> Result<()> {
    let k = keys.to_string();
    let mut q =
        tx.prepare("SELECT EXISTS(SELECT 1 FROM main.publication_row WHERE tbl=? AND k=?)")?;
    let exists: bool = q.query_row(params![name, k], |r| r.get(0))?;
    observe(&q, 1);
    if exists {
        return Ok(());
    }
    let t = table(name)?;
    let old = get(tx, name, &keys)?;
    if reference.is_none() && old == new {
        return Ok(());
    }
    let old_bytes = if old.is_some() && matches!(name, "content" | "check_context") {
        tx.query_row(
            &format!(
                "SELECT octet_length(json) FROM main.{name} b WHERE {}",
                pred(t, "b")
            ),
            [&k],
            |r| r.get(0),
        )?
    } else {
        0i64
    };
    let old = if matches!(name, "activity_stage" | "source_stage" | "teams_stage")
        || name == "check_context" && keys[3] == 1
    {
        None
    } else {
        old
    };
    tx.execute("INSERT INTO publication_row(tbl,k,old_json,new_json,reference,old_bytes) VALUES(?,?,?,?,?,?)",params![name,k,old.map(|v|v.to_string()),new.map(|v|v.to_string()),reference.map(|v|v.to_string()),old_bytes])?;
    Ok(())
}
fn ascii_prefix(s: &str, p: &str) -> bool {
    s.get(..p.len()).is_some_and(|a| a.eq_ignore_ascii_case(p))
}
fn flag(tx: &Connection, id: &Value, source: Option<&str>) -> Result<()> {
    let k = json!([id]);
    let saved: Option<String> = tx
        .query_row(
            "SELECT new_json FROM main.publication_row WHERE tbl='watch' AND k=?",
            [k.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let mut bits: Value = match saved {
        Some(ref s) => serde_json::from_str(s)?,
        None => json!([id, 0, 0, 0]),
    };
    if let Some(source) = source {
        if source == "authored" {
            bits[1] = json!(1);
        }
        if source == "direct" || ascii_prefix(source, "team:") {
            bits[2] = json!(1);
        }
        if source == "selected" {
            bits[3] = json!(1);
        }
    } else {
        bits = json!([id, 0, 0, 0]);
    }
    let encoded = bits.to_string();
    if let Some(saved) = saved {
        if saved != encoded {
            tx.execute(
                "UPDATE publication_row SET new_json=? WHERE tbl='watch' AND k=?",
                params![encoded, k.to_string()],
            )?;
        }
    } else {
        let old = get(tx, "watch", &k)?;
        // Even unchanged zero flags seed the accumulation pass durably.
        tx.execute(
            "INSERT INTO publication_row(tbl,k,old_json,new_json) VALUES('watch',?,?,?)",
            params![k.to_string(), old.map(|v| v.to_string()), bits.to_string()],
        )?;
    }
    Ok(())
}
fn incoming(x: &Value, field: &str, id: &Value) -> Option<usize> {
    x[field].as_array()?.iter().position(|v| v["id"] == *id)
}
fn check_value(tx: &Connection, x: &Value, id: &Value) -> Result<Option<Value>> {
    if let Some(i) = incoming(x, "contexts", id) {
        return Ok(Some(json!(["args", format!("$.contexts[{i}]")])));
    }
    if x["first"] != true
        && get(tx, "check_context", &json!([x["id"], x["commit"], id, 1]))?.is_some()
    {
        return Ok(Some(json!(["check_context", x["id"], x["commit"], id, 1])));
    }
    Ok(None)
}
fn summary_member(tx: &Connection, key: &Value, cutoff: u64) -> Result<Option<Value>> {
    let Some(id) = key.as_str().and_then(|k| k.strip_prefix("catchup:")) else {
        return Ok(None);
    };
    let Some(a) = get(tx, "activity", &json!([id]))? else {
        return Ok(None);
    };
    let seen = get(tx, "seen", &json!([id]))?;
    Ok((a[5] == 1
        && a[4].as_u64().is_some_and(|t| t >= cutoff)
        && seen.as_ref().is_some_and(|s| s[1] == 0))
    .then(|| json!(id)))
}
fn project(tx: &Connection, stage: Step, kind: &str, x: &Value, pos: i64, d: &Value) -> Result<()> {
    use Step::*;
    let time = x["time"].as_u64().unwrap();
    let mut output: Option<(&str, Value, Option<Value>, Option<Value>)> = None;
    let mut set = |tbl, keys, new, rf| output = Some((tbl, keys, new, rf));
    match stage {
        SourceInput => set(
            "source_stage",
            json!([x["source"], d["id"]]),
            Some(json!([x["source"], d["id"], null])),
            Some(json!(["args", format!("$.prs[{pos}]")])),
        ),
        SourceOld if d[1] == x["source"] => {
            let present = get(tx, "source_stage", &json!([x["source"], d[0]]))?.is_some();
            set(
                "watch_source",
                json!([d[0], d[1]]),
                present.then(|| d.clone()),
                None,
            );
        }
        SourceMembers | SourceLocators | SourceContent if d[0] == x["source"] => {
            if matches!(stage, SourceMembers) {
                set(
                    "watch_source",
                    json!([d[1], d[0]]),
                    Some(json!([d[1], d[0]])),
                    None,
                );
            } else if matches!(stage, SourceContent) {
                set(
                    "content",
                    json!([format!("list:{}", d[1].as_str().unwrap())]),
                    Some(json!([
                        format!("list:{}", d[1].as_str().unwrap()),
                        null,
                        time,
                        time
                    ])),
                    Some(json!(["source_stage", d[0], d[1]])),
                );
            } else {
                let pr: Value = serde_json::from_str(d[2].as_str().unwrap())?;
                let old = get(tx, "watch_locator", &json!([d[1]]))?.unwrap_or_else(|| {
                    json!([
                        d[1], null, null, null, "open", null, null, null, 0, 0, null, null, 0
                    ])
                });
                let mut row = old.clone();
                row[1] = pr["repo"].clone();
                row[2] = pr["number"].clone();
                if !pr["repoId"].is_null() {
                    row[3] = pr["repoId"].clone();
                }
                set("watch_locator", json!([d[1]]), Some(row), None);
            }
        }
        SelectionOld if d[1] == "selected" => {
            let l = get(tx, "watch_locator", &json!([d[0]]))?;
            let keep = x["open"] == true
                && l.as_ref()
                    .is_some_and(|r| r[1] == x["repo"] && r[2] == x["number"] && r[4] == "open");
            set(
                "watch_source",
                json!([d[0], "selected"]),
                keep.then(|| d.clone()),
                None,
            );
        }
        SelectionNew
            if x["open"] == true && d[1] == x["repo"] && d[2] == x["number"] && d[4] == "open" =>
        {
            set(
                "watch_source",
                json!([d[0], "selected"]),
                Some(json!([d[0], "selected"])),
                None,
            )
        }
        HeadStage if d[0] == x["pr"]["id"] && d[4] == 1 => {
            set("check_context", json!([d[0], d[1], d[2], 1]), None, None)
        }
        HeadStates if d[0] == x["pr"]["id"] => set(
            "check_state",
            json!([d[0], d[1]]),
            Some(json!([d[0], d[1], d[2], 0, "Old revision", d[5]])),
            None,
        ),
        ClosedSources if d[0] == x["pr"]["id"] => {
            set("watch_source", json!([d[0], d[1]]), None, None)
        }
        SelectedSource => {
            let id = &x["pr"]["id"];
            set(
                "watch_source",
                json!([id, "selected"]),
                (x["open"] == true && x["pr"]["state"] == "open").then(|| json!([id, "selected"])),
                None,
            );
        }
        Locator => set(
            "watch_locator",
            json!([x["locator"][0]]),
            Some(x["locator"].clone()),
            None,
        ),
        ListContent => {
            let key = format!("list:{}", x["pr"]["id"].as_str().unwrap());
            set(
                "content",
                json!([key]),
                Some(json!([key, null, time, time])),
                Some(json!(["args", "$.pr"])),
            );
        }
        TeamsOldStage => {
            let found = x["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["node_id"] == d[0]);
            if !found {
                set("teams_stage", json!([d[0]]), None, None);
            }
        }
        TeamsInput => set(
            "teams_stage",
            json!([d["node_id"]]),
            Some(json!([
                d["node_id"],
                format!(
                    "{}/{}",
                    d["organization"]["login"].as_str().unwrap(),
                    d["slug"].as_str().unwrap()
                )
            ])),
            None,
        ),
        TeamSources
            if d[1].as_str().is_some_and(|s| ascii_prefix(s, "team:"))
                && get(tx, "teams_stage", &json!([&d[1].as_str().unwrap()[5..]]))?.is_none() =>
        {
            set("watch_source", json!([d[0], d[1]]), None, None)
        }
        TeamPending
            if d[1]
                .as_str()
                .is_some_and(|s| ascii_prefix(s, "discover:team:"))
                && get(tx, "teams_stage", &json!([&d[1].as_str().unwrap()[14..]]))?.is_none() =>
        {
            set("pending", json!([d[0]]), None, None)
        }
        TeamsOld => set(
            "teams",
            json!([d[0]]),
            get(tx, "teams_stage", &json!([d[0]]))?,
            None,
        ),
        TeamsNew => set("teams", json!([d[0]]), Some(d.clone()), None),
        TeamsClear => set("teams_stage", json!([d[0]]), None, None),
        CheckState => {
            let key = json!([x["id"], x["commit"]]);
            let new = if x["complete"] == true {
                json!([
                    x["id"],
                    x["commit"],
                    time,
                    0,
                    "Refreshing; publication incomplete",
                    time
                ])
            } else {
                let old = get(tx, "check_state", &key)?
                    .unwrap_or_else(|| json!([null, null, null, 0, null, 0]));
                json!([
                    x["id"],
                    x["commit"],
                    old[2],
                    old[3],
                    "Refreshing; traversal incomplete",
                    old[5]
                ])
            };
            set("check_state", key, Some(new), None);
        }
        CheckOld | CheckStage
            if d[0] == x["id"]
                && d[1] == x["commit"]
                && d[4] == if matches!(stage, CheckOld) { 0 } else { 1 } =>
        {
            let rf = check_value(tx, x, &d[2])?;
            set(
                "check_context",
                json!([d[0], d[1], d[2], 0]),
                rf.as_ref().map(|_| json!([d[0], d[1], d[2], null, 0])),
                rf,
            );
        }
        CheckInput => {
            let staged = if x["complete"] == true { 0 } else { 1 };
            set(
                "check_context",
                json!([x["id"], x["commit"], d["id"], staged]),
                Some(json!([x["id"], x["commit"], d["id"], null, staged])),
                Some(json!(["args", format!("$.contexts[{pos}]")])),
            );
        }
        CheckClear
            if d[0] == x["id"]
                && d[1] == x["commit"]
                && d[4] == 1
                && (x["complete"] == true || incoming(x, "contexts", &d[2]).is_none()) =>
        {
            set("check_context", json!([d[0], d[1], d[2], 1]), None, None)
        }
        FlagsReset => flag(tx, &d[0], None)?,
        FlagsCanonical => {
            let changed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM main.publication_row WHERE tbl='watch_source' AND k=?)",[json!([d[0],d[1]]).to_string()],|r|r.get(0))?;
            if !changed {
                flag(tx, &d[0], d[1].as_str())?;
            }
        }
        FlagsPrepared if d[1] == "watch_source" && !d[3].is_null() => {
            let v: Value = serde_json::from_str(d[3].as_str().unwrap())?;
            flag(tx, &v[0], v[1].as_str())?;
        }
        Seen | Dedup | Activity | Intent | Member | Catchup | Payload | ActivityClear
            if d[0] == x["repo"] && d[1] == x["number"] =>
        {
            let r: Value = serde_json::from_str(d[3].as_str().unwrap())?;
            let fresh = get(tx, "dedup", &json!([d[2]]))?.is_none();
            let occurred = r["occurred"].as_u64().unwrap_or(0);
            let retained = occurred >= time.saturating_sub(crate::retention::ACTIVITY_AGE);
            let eligible = fresh
                && retained
                && x["alerts"] == true
                && occurred > x["baseline"].as_u64().unwrap()
                && r["eligible"] == true;
            let catchup = x["mode"] == "recovery" && occurred <= x["boundary"].as_u64().unwrap();
            let normal = eligible && !catchup;
            match stage {
                Seen if fresh && get(tx, "seen", &json!([d[2]]))?.is_none() => set(
                    "seen",
                    json!([d[2]]),
                    Some(json!([
                        d[2],
                        i32::from(
                            x["historical"] == true || occurred <= x["baseline"].as_u64().unwrap()
                        )
                    ])),
                    None,
                ),
                Dedup if fresh => set("dedup", json!([d[2]]), Some(json!([d[2]])), None),
                Activity if fresh && retained => set(
                    "activity",
                    json!([d[2]]),
                    Some(json!([
                        d[2],
                        d[0],
                        d[1],
                        r["kind"],
                        occurred,
                        i32::from(r["eligible"] == true),
                        r["reason"],
                        time
                    ])),
                    None,
                ),
                Intent if normal => set(
                    "activity_intent",
                    json!([x["intent"]]),
                    Some(json!([
                        x["intent"],
                        "pr",
                        x["intentState"].as_str().unwrap_or("unavailable"),
                        time
                    ])),
                    None,
                ),
                Member if normal => set(
                    "intent_member",
                    json!([x["intent"], d[2]]),
                    Some(json!([x["intent"], d[2]])),
                    None,
                ),
                Catchup if eligible && catchup => set(
                    "progress",
                    json!([format!("catchup:{}", d[2].as_str().unwrap())]),
                    Some(json!([format!("catchup:{}", d[2].as_str().unwrap()), d[2]])),
                    None,
                ),
                Payload
                    if retained && (fresh || get(tx, "activity", &json!([d[2]]))?.is_some()) =>
                {
                    let key = format!("activity-payload:{}", d[2].as_str().unwrap());
                    set(
                        "content",
                        json!([key]),
                        Some(json!([key, null, time, time])),
                        Some(json!(["activity_stage", d[0], d[1], d[2]])),
                    );
                }
                ActivityClear => set("activity_stage", json!([d[0], d[1], d[2]]), None, None),
                _ => {}
            }
        }
        Target if get(tx, "activity_target", &json!([x["repo"], x["number"]]))?.is_some() => set(
            "activity_target",
            json!([x["repo"], x["number"]]),
            Some(json!([x["repo"], x["number"], time + 10, 1])),
            None,
        ),
        Progress => {
            if kind == "selection" {
                if x["number"].as_u64().unwrap() > 0 {
                    set(
                        "progress",
                        json!(["selection"]),
                        Some(json!(["selection", json!([x["repo"], x["number"]])])),
                        None,
                    );
                }
            } else {
                let key = format!(
                    "activity:pr:{}:{}",
                    x["repo"].as_str().unwrap(),
                    x["number"].as_u64().unwrap()
                );
                set(
                    "progress",
                    json!([key]),
                    Some(json!([key, time.to_string()])),
                    None,
                );
            }
        }
        SummaryIntent | SummaryMember => {
            if let Some(id) = summary_member(
                tx,
                &d[0],
                time.saturating_sub(crate::retention::ACTIVITY_AGE),
            )? {
                if matches!(stage, SummaryIntent) {
                    set(
                        "activity_intent",
                        json!([x["intent"]]),
                        Some(json!([
                            x["intent"],
                            "summary",
                            x["intentState"].as_str().unwrap_or("unavailable"),
                            time
                        ])),
                        None,
                    );
                } else {
                    set(
                        "intent_member",
                        json!([x["intent"], id]),
                        Some(json!([x["intent"], id])),
                        None,
                    );
                }
            }
        }
        SummaryClear if d[0].as_str().is_some_and(|k| ascii_prefix(k, "catchup:")) => {
            set("progress", json!([d[0]]), None, None)
        }
        Baseline if get(tx, "progress", &json!(["activity:baseline"]))?.is_none() => set(
            "progress",
            json!(["activity:baseline"]),
            Some(json!(["activity:baseline", time.to_string()])),
            None,
        ),
        Normal => {
            if let Some(mut row) = get(tx, "activity_cycle", &json!([1]))? {
                row[1] = json!("normal");
                set("activity_cycle", json!([1]), Some(row), None);
            }
        }
        Health => {
            let key = if kind == "teams" {
                json!("teams")
            } else {
                x["source"].clone()
            };
            set(
                "resource_state",
                json!([key]),
                Some(json!([key, time, 1, null])),
                None,
            );
        }
        _ => {}
    }
    if let Some((tbl, keys, new, rf)) = output {
        emit(tx, tbl, keys, new, rf)?;
    }
    Ok(())
}

#[cfg(not(test))]
fn observe(_: &rusqlite::Statement<'_>, _: usize) {}
#[cfg(test)]
fn observe(q: &rusqlite::Statement<'_>, rows: usize) {
    crate::publication_completion::observe(q, rows);
}

//! Activity changes use the same prepared old-image journal as watch publication.
//! All classification is fixed before physical application or staging consumption.
use super::*;
pub(super) fn plans(kind: &str, x: &Value) -> Result<Vec<Plan>> {
    let time = x["time"].as_u64().unwrap();
    let cutoff = time.saturating_sub(crate::retention::ACTIVITY_AGE);
    let intent = x["intent"].as_i64().unwrap();
    let mut out = vec![];
    if kind == "activity_summary" {
        let members = format!(
            "SELECT a.id FROM activity a JOIN progress p ON p.key='catchup:'||a.id JOIN seen s ON s.source_id=a.id WHERE s.seen=0 AND a.eligible=1 AND a.occurred>={cutoff}"
        );
        out.push(plan("activity_intent",format!("SELECT json_array({intent}),json_array({intent},'summary','unavailable',{time}),NULL WHERE EXISTS({members})")));
        out.push(plan(
            "intent_member",
            format!("SELECT json_array({intent},id),json_array({intent},id),NULL FROM ({members})"),
        ));
        out.push(plan(
            "progress",
            "SELECT json_array(key),NULL,NULL FROM progress WHERE key LIKE 'catchup:%'",
        ));
        out.push(plan("progress",format!("SELECT json_array('activity:baseline'),json_array('activity:baseline','{time}'),NULL WHERE NOT EXISTS(SELECT 1 FROM progress WHERE key='activity:baseline')")));
        out.push(plan("activity_cycle","SELECT json_array(id),json_array(id,'normal',boundary,notifications_done),NULL FROM activity_cycle WHERE id=1"));
        return Ok(out);
    }
    let repo = a(x, "repo");
    let num = a(x, "number");
    let baseline = a(x, "baseline");
    let boundary = a(x, "boundary");
    let historical = x["historical"] == true;
    let alerts = x["alerts"] == true;
    let rows = format!(
        "SELECT s.*,json_extract(json,'$.occurred') occurred,json_extract(json,'$.eligible') eligible,json_extract(json,'$.kind') kind,json_extract(json,'$.reason') reason FROM activity_stage s WHERE repo={repo} AND number={num}"
    );
    let fresh = format!(
        "SELECT * FROM ({rows}) s WHERE NOT EXISTS(SELECT 1 FROM dedup d WHERE d.source_id=s.id)"
    );
    let retained = format!("SELECT * FROM ({fresh}) WHERE occurred>={cutoff}");
    let eligible = format!(
        "SELECT * FROM ({retained}) WHERE {} AND occurred>{baseline} AND eligible=1",
        i32::from(alerts)
    );
    let recovery = x["mode"] == "recovery";
    let normal = format!(
        "SELECT * FROM ({eligible}) WHERE {} OR occurred>{boundary}",
        i32::from(!recovery)
    );
    // Plans all read the original ledgers, before any dedup row can be applied.
    out.push(plan("seen",format!("SELECT json_array(id),json_array(id,{} OR occurred<={baseline}),NULL FROM ({fresh}) s WHERE NOT EXISTS(SELECT 1 FROM seen WHERE source_id=s.id)",i32::from(historical))));
    out.push(plan(
        "dedup",
        format!("SELECT json_array(id),json_array(id),NULL FROM ({fresh})"),
    ));
    out.push(plan("activity",format!("SELECT json_array(id),json_array(id,repo,number,kind,occurred,eligible,reason,{time}),NULL FROM ({retained})")));
    out.push(plan("activity_intent",format!("SELECT json_array({intent}),json_array({intent},'pr','unavailable',{time}),NULL WHERE EXISTS({normal})")));
    out.push(plan(
        "intent_member",
        format!("SELECT json_array({intent},id),json_array({intent},id),NULL FROM ({normal})"),
    ));
    if recovery {
        out.push(plan("progress",format!("SELECT json_array('catchup:'||id),json_array('catchup:'||id,id),NULL FROM ({eligible}) WHERE occurred<={boundary}")));
    }
    out.push(plan("content",format!("SELECT json_array('activity-payload:'||id),json_array('activity-payload:'||id,NULL,{time},{time}),json_array('activity_stage',repo,number,id) FROM ({rows}) s WHERE occurred>={cutoff} AND (EXISTS(SELECT 1 FROM activity a WHERE a.id=s.id) OR NOT EXISTS(SELECT 1 FROM dedup d WHERE d.source_id=s.id))")));
    out.push(plan("activity_stage",format!("SELECT json_array(repo,number,id),NULL,NULL FROM activity_stage WHERE repo={repo} AND number={num}")));
    out.push(plan("activity_target",format!("SELECT json_array(repo,number),json_array(repo,number,{},1),NULL FROM activity_target WHERE repo={repo} AND number={num}",time+10)));
    let progress = lit(&format!(
        "activity:pr:{}:{}",
        x["repo"].as_str().unwrap(),
        x["number"].as_u64().unwrap()
    ));
    out.push(plan(
        "progress",
        format!("SELECT json_array({progress}),json_array({progress},'{time}'),NULL"),
    ));
    Ok(out)
}

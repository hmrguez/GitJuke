use gitjuke_core::{activity::*, model::Identity};
use serde_json::json;
fn id() -> Identity {
    Identity {
        id: 42,
        login: "reader".into(),
    }
}
fn comment(n: u64, login: &str, user: u64, body: &str, html: &str) -> serde_json::Value {
    json!({"node_id":format!("IC_{n}"),"id":n,"created_at":"2026-10-17T00:00:00Z","body":body,"body_html":html,"html_url":format!("https://github.com/a/b/pull/1#issuecomment-{n}"),"user":{"id":user,"login":login,"type":"User"}})
}
#[test]
fn sticky_reasons_never_invent_events_and_only_rendered_mentions_classify() {
    let thread = json!([{"reason":"mention","repository":{"full_name":"a/b"},"subject":{"type":"PullRequest","url":"https://api.github.com/repos/a/b/pulls/1"}}]);
    assert_eq!(
        notification_targets(&thread).unwrap(),
        vec![("a/b".into(), 1)]
    );
    let plain = comment(1, "other", 43, "@reader", "<pre>@reader</pre>");
    assert!(
        records("a/b", 1, 1, &json!([plain]), &id(), &[], false)
            .unwrap()
            .is_empty()
    );
    let mention = comment(
        2,
        "other",
        43,
        "@reader",
        "<a class=\"user-mention\" href=\"https://github.com/reader\">@reader</a>",
    );
    assert_eq!(
        records("a/b", 1, 1, &json!([mention]), &id(), &[], false).unwrap()[0].kind,
        "mention"
    );
    let unsafe_thread = json!([{"repository":{"full_name":"a/b"},"subject":{"type":"PullRequest","url":"https://other.example/repos/a/b/pulls/1"}}]);
    assert!(notification_targets(&unsafe_thread).is_err());
}
#[test]
fn textless_reviews_own_bot_and_stable_team_requests() {
    let own = comment(1, "renamed", 42, "x", "<p>x</p>");
    let mut bot = comment(2, "automation[bot]", 50, "x", "<p>x</p>");
    bot["user"]["type"] = json!("Bot");
    let items = records("a/b", 1, 1, &json!([own, bot]), &id(), &[], true).unwrap();
    assert!(items.iter().all(|a| !a.eligible && !a.seen));
    let review = json!([{"node_id":"R_1","state":"APPROVED","submitted_at":"2026-10-17T00:00:00Z","body":"","user":{"id":43,"login":"reviewer"}}]);
    assert_eq!(
        records("a/b", 1, 2, &review, &id(), &[], true).unwrap()[0].kind,
        "review"
    );
    let request = json!([{"node_id":"EV_1","event":"review_requested","created_at":"2026-10-17T00:00:00Z","actor":{"id":43},"requested_team":{"node_id":"T_1","slug":"renamed-team"}}]);
    assert_eq!(
        records(
            "a/b",
            1,
            4,
            &request,
            &id(),
            &[("T_1".into(), "org/old-team".into())],
            false
        )
        .unwrap()[0]
            .kind,
        "request"
    );
    assert!(
        records("a/b", 1, 4, &request, &id(), &[], false)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn missing_individual_schema_and_invalid_times_are_not_success() {
    let mut c = comment(1, "other", 43, "@reader", "");
    c.as_object_mut().unwrap().remove("body_html");
    assert!(records("a/b", 1, 1, &json!([c]), &id(), &[], false).is_err());
    assert!(timestamp("2026-02-31T00:00:00Z").is_err());
    assert!(timestamp("2026-10-17T00:00:00+02:00").is_err());
}

use gitjuke_github::{github::*, model::StackState, transport::Output};
use serde_json::json;
use std::collections::BTreeMap;
fn response(status: u16, body: serde_json::Value) -> Response {
    Response {
        status,
        headers: BTreeMap::new(),
        body,
    }
}
#[test]
fn auth_json_exit_zero_with_error_is_not_authenticated() {
    let output=Output{stdout:serde_json::to_vec(&json!({"hosts":{"github.com":[{"active":true,"state":"error","login":"reader","tokenSource":"keyring"}]}})).unwrap(),stderr:vec![],success:true};
    assert!(parse_auth(output).is_err());
}
#[test]
fn api_error_and_partial_schema_are_not_successful_empty_sets() {
    assert!(parse_search(&response(403, json!({"message":"Forbidden"}))).is_err());
    assert!(parse_search(&response(200, json!({"total_count":0,"items":[]}))).is_err());
    let page = parse_search(&response(
        200,
        json!({"total_count":0,"incomplete_results":true,"items":[]}),
    ))
    .unwrap();
    assert!(page.incomplete);
}
#[test]
fn search_preserves_merged_state_for_pull_request_readers() {
    let items: Vec<_> = [
        ("open", serde_json::Value::Null),
        ("closed", serde_json::Value::Null),
        ("closed", json!("2026-10-18T10:00:00Z")),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (state, merged_at))| {
        let number = i + 1;
        json!({
            "node_id":format!("PR_{number}"),"number":number,"title":"Change",
            "repository_url":"https://api.github.com/repos/a/b",
            "html_url":format!("https://github.com/a/b/pull/{number}"),
            "state":state,"draft":false,"user":{"login":"reader"},
            "updated_at":"2026-10-18T10:00:00Z","pull_request":{"merged_at":merged_at}
        })
    })
    .collect();
    let page = parse_search(&response(
        200,
        json!({"total_count":3,"incomplete_results":false,"items":items}),
    ))
    .unwrap();
    assert_eq!(
        page.prs
            .iter()
            .map(|p| p.state.as_str())
            .collect::<Vec<_>>(),
        vec!["open", "closed", "merged"]
    );
}
#[test]
fn stack_unknown_none_and_authoritative_order_are_distinct() {
    assert!(stack_number(&response(404, json!({}))).is_err());
    assert_eq!(stack_number(&response(200, json!([]))).unwrap(), None);
    let stack = response(
        200,
        json!({"number":7,"base":{"ref":"release"},"pull_requests":[{"number":9,"title":"Bottom","html_url":"https://github.com/a/b/pull/9"},{"number":2,"title":"Top","html_url":"https://github.com/a/b/pull/2"}]}),
    );
    match parse_stack(&stack, "a/b", 2).unwrap() {
        StackState::Published { trunk, members, .. } => {
            assert_eq!(trunk, "release");
            assert_eq!(
                members.iter().map(|p| p.number).collect::<Vec<_>>(),
                vec![9, 2]
            );
        }
        _ => panic!("published stack"),
    };
    assert!(parse_stack(&stack, "a/b", 100).is_err());
    assert!(parse_stack(&stack, "a/other", 2).is_err());
}
#[test]
fn server_deadlines_include_numeric_and_http_date_retry_after() {
    let mut r = response(429, json!({}));
    r.headers.insert("retry-after".into(), "120".into());
    assert_eq!(r.next_deadline(100), 220);
    r.headers
        .insert("retry-after".into(), "Wed, 21 Oct 2015 07:28:00 GMT".into());
    assert_eq!(r.next_deadline(100), 1445412480);
    r.headers.clear();
    r.headers.insert("x-ratelimit-remaining".into(), "0".into());
    r.headers.insert("x-ratelimit-reset".into(), "900".into());
    assert_eq!(r.next_deadline(100), 900);
    let r = response(403, json!({"message":"Forbidden"}));
    assert_eq!(r.next_deadline(100), 0);
    let r = response(403, json!({"message":"API rate limit exceeded"}));
    assert_eq!(r.next_deadline(100), 160);
}
#[test]
fn read_response_requires_headers_and_never_accepts_cli_failure() {
    assert!(
        Response::parse(Output {
            stdout: b"{}".to_vec(),
            stderr: vec![],
            success: true
        })
        .is_err()
    );
    assert!(
        Response::parse(Output {
            stdout: b"HTTP/2.0 200 OK\r\n\r\n{}".to_vec(),
            stderr: vec![],
            success: false
        })
        .is_err()
    );
    let r = Response::parse(Output {
        stdout: b"HTTP/2.0 200 OK\r\nX-Poll-Interval: 60\r\n\r\n{}".to_vec(),
        stderr: vec![],
        success: true,
    })
    .unwrap();
    assert_eq!(r.headers.get("x-poll-interval").unwrap(), "60");
}
#[test]
fn auth_loss_without_http_headers_is_recognized_without_exposing_diagnostics() {
    let output = Output {
        stdout: vec![],
        stderr: b"To get started, please run: gh auth login".to_vec(),
        success: false,
    };
    assert!(gitjuke_github::transport::indicates_auth_loss(&output));
}

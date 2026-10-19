use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BuildInfo {
    version: &'static str,
    build_id: &'static str,
    revision: &'static str,
    source_state: &'static str,
    target: &'static str,
    profile: &'static str,
    schema_version: i64,
}

// Independent of Runtime/SQLite/auth: diagnostics remain usable during recovery.
#[tauri::command]
pub(crate) fn build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        build_id: env!("GITJUKE_BUILD_ID"),
        revision: env!("GITJUKE_BUILD_REVISION"),
        source_state: env!("GITJUKE_BUILD_SOURCE_STATE"),
        target: env!("GITJUKE_BUILD_TARGET"),
        profile: env!("GITJUKE_BUILD_PROFILE"),
        schema_version: gitjuke_core::store::SCHEMA_VERSION,
    }
}

#[cfg(test)]
#[path = "../../build_metadata.rs"]
mod metadata;

#[cfg(test)]
mod tests {
    use super::metadata;

    #[test]
    fn ipc_returns_build_information_without_a_database_or_authenticated_runtime() {
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![super::build_info])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let value: serde_json::Value = tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "build_info".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            },
        )
        .unwrap()
        .deserialize()
        .unwrap();
        assert_eq!(value, serde_json::to_value(super::build_info()).unwrap());
        println!("BUILD_INFO_IPC={value}");
    }

    #[test]
    fn compiled_metadata_is_bounded_serializable_and_uses_the_binary_contract() {
        let value = serde_json::to_value(super::build_info()).unwrap();
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["schemaVersion"], gitjuke_core::store::SCHEMA_VERSION);
        assert_eq!(value["target"], env!("GITJUKE_BUILD_TARGET"));
        assert_eq!(value["buildId"], env!("GITJUKE_BUILD_ID"));
        assert!(value.to_string().len() < 1024);
        println!("compiled build info: {value}");
    }

    #[test]
    fn build_ids_reject_control_characters_and_oversized_values() {
        assert_eq!(metadata::build_id(None, 42).unwrap(), "local-42");
        assert_eq!(
            metadata::build_id(Some("ci.123.2"), 42).unwrap(),
            "ci.123.2"
        );
        for invalid in ["", "line\nbreak", "bad/path", "has space", &"x".repeat(65)] {
            assert!(metadata::build_id(Some(invalid), 42).is_err());
        }
    }

    #[test]
    fn source_archives_report_unavailable_and_git_edits_are_not_clean() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            metadata::source(dir.path()),
            ("unavailable".into(), "unavailable")
        );
        let git = |args: &[&str]| metadata::git(dir.path(), args).unwrap();
        git(&["init", "-q"]);
        git(&[
            "-c",
            "user.name=Build test",
            "-c",
            "user.email=build@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "fixture",
        ]);
        let (revision, state) = metadata::source(dir.path());
        assert_eq!(state, "clean");
        assert!([40, 64].contains(&revision.len()));
        std::fs::write(dir.path().join("source.txt"), "changed").unwrap();
        assert_eq!(metadata::source(dir.path()), (revision, "modified"));
        let archive = dir.path().join("unrelated-source-archive");
        std::fs::create_dir(&archive).unwrap();
        assert_eq!(
            metadata::source(&archive),
            ("unavailable".into(), "unavailable")
        );
    }
}

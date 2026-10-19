//! Frontend command adapters and the shared bounded-response encoder.
pub(crate) mod build_info;
pub(crate) mod changes;
pub(crate) mod checks;
pub(crate) mod conversation;
pub(crate) mod notifications;
pub(crate) mod queries;
pub(crate) mod reader;

pub(super) fn safe<T: serde::Serialize>(result: gitjuke_core::Result<T>) -> Result<T, String> {
    let value = result.map_err(|e| e.to_string())?;
    let bytes =
        serde_json::to_vec(&value).map_err(|_| "The response could not be encoded.".to_owned())?;
    if bytes.len() > 1024 * 1024 {
        return Err("This response exceeds the 1 MiB IPC limit. Use a smaller page or the GitHub source; no truncated data was returned.".into());
    }
    Ok(value)
}

#[cfg(test)]
mod ipc_tests {
    #[test]
    fn serialized_response_ceiling_is_inclusive_and_never_clips() {
        let exact = "x".repeat(1024 * 1024 - 2);
        assert!(super::safe(Ok(exact)).is_ok());
        let over = "x".repeat(1024 * 1024 - 1);
        assert!(super::safe(Ok(over)).is_err());
    }
}

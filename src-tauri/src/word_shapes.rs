//! Local dictionary inspection. The caller cannot choose files or a provider.
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

const WORKER: &str = include_str!("../../scripts/word_dictionary.py");
const REFERENCE: &str = "/Users/paulcooper/.grok/control-panel/wizard-joe/reference";
const PYTHON: &str = "/Users/paulcooper/.grok/control-panel/wizard-joe/python-env/bin/python3";
const RESPONSE_LIMIT: usize = 1024 * 1024;
static OPERATING: Mutex<()> = Mutex::const_new(());

fn request(payload: Value) -> Result<Value, String> {
    let fields = payload
        .as_object()
        .ok_or("Dictionary request must be an object")?;
    let allowed = [
        "action",
        "query",
        "language",
        "offset",
        "limit",
        "counterpartOffset",
        "counterpartLimit",
    ];
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Dictionary requests cannot substitute source paths or unknown options".into());
    }
    let action = match payload.get("action") {
        Some(value) => value.as_str().ok_or("Dictionary action must be text")?,
        None => "query",
    }
    .to_owned();
    if !["query", "coverage"].contains(&action.as_str()) {
        return Err("Unknown dictionary operation".into());
    }
    let language = payload["language"].as_str().unwrap_or("all");
    if !["all", "eng", "jpn", "ind", "zsm", "cmn"].contains(&language) {
        return Err("Language is outside the frozen dictionary inventory".into());
    }
    for (key, max) in [("query", 256), ("language", 3)] {
        if let Some(value) = payload.get(key) {
            let text = value
                .as_str()
                .ok_or("Dictionary text fields must be strings")?;
            if text.chars().count() > max || text.contains('\0') {
                return Err("Dictionary text exceeds its input budget".into());
            }
        }
    }
    for (key, max) in [
        ("offset", 100_000),
        ("limit", 20),
        ("counterpartOffset", 100_000),
        ("counterpartLimit", 20),
    ] {
        if let Some(value) = payload.get(key) {
            let number = value
                .as_u64()
                .ok_or("Dictionary paging fields must be integers")?;
            if number > max || (key.ends_with("Limit") || key == "limit") && number == 0 {
                return Err("Dictionary page exceeds its budget".into());
            }
        }
    }
    let mut result = payload;
    result["action"] = json!(action);
    result["referenceRoot"] = json!(REFERENCE);
    Ok(result)
}

pub(crate) async fn run(payload: Value) -> Result<Value, String> {
    let payload = request(payload)?;
    let _guard = OPERATING
        .try_lock()
        .map_err(|_| "Dictionary inspection is busy; wait for this lookup")?;
    let input =
        serde_json::to_vec(&payload).map_err(|_| "Dictionary request serialization failed")?;
    if input.len() > 16 * 1024 {
        return Err("Dictionary request exceeds its input budget".into());
    }
    let mut child = tokio::process::Command::new(PYTHON)
        .args(["-B", "-c", WORKER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Could not start local dictionary inspection")?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("Dictionary input is unavailable")?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("Dictionary output is unavailable")?;
    let result = tokio::time::timeout(Duration::from_secs(180), async {
        stdin
            .write_all(&input)
            .await
            .map_err(|_| "Dictionary input failed")?;
        drop(stdin);
        let mut bytes = Vec::new();
        (&mut stdout)
            .take(RESPONSE_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "Dictionary output failed")?;
        if bytes.len() > RESPONSE_LIMIT {
            let _ = child.kill().await;
            return Err("Dictionary result exceeds its response budget".to_owned());
        }
        let exit = child
            .wait()
            .await
            .map_err(|_| "Dictionary process failed")?;
        let response: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Dictionary returned an invalid result")?;
        if !exit.success()
            || response["status"] != "ready"
            || response["schema"] != "bomb-code/dictionary-shapes/v1"
        {
            return Err(response["error"]
                .as_str()
                .unwrap_or("Dictionary inspection unavailable")
                .to_owned());
        }
        Ok(response)
    })
    .await
    .map_err(|_| "Local dictionary inspection timed out")?;
    result
}

#[tauri::command]
pub async fn word_shape_dictionary(payload: Value) -> Result<Value, String> {
    run(payload).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_cannot_replace_sources_or_escape_budgets() {
        assert!(request(json!({"referenceRoot":"/tmp"})).is_err());
        assert!(request(json!({"action":"run"})).is_err());
        assert!(request(json!({"language":"fra"})).is_err());
        assert!(request(json!({"limit":21})).is_err());
        assert!(request(json!({"offset":-1})).is_err());
        assert!(request(json!({"query":"x".repeat(257)})).is_err());
        for payload in [
            json!(null),
            json!([]),
            json!({"action":null}),
            json!({"language":null}),
            json!({"query":false}),
            json!({"limit":0}),
            json!({"offset":0.5}),
            json!({"counterpartLimit":0}),
            json!({"counterpartOffset":true}),
        ] {
            assert!(request(payload).is_err());
        }
        assert_eq!(
            request(json!({"query":"月","language":"jpn"})).unwrap()["referenceRoot"],
            REFERENCE
        );
    }
}

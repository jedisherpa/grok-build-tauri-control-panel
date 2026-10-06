//! A local reference library, kept separate from live agent sessions and their approvals.
use crate::AppState;
use chrono::Utc;
use grok_persistence::{SessionRecord, TranscriptEntry};
use serde_json::{json, Value};
use tauri::State;
use uuid::Uuid;

const INDEXER: &str = include_str!("../../scripts/history_library.py");

async fn run(state: &AppState, action: &str, payload: Value) -> Result<Value, String> {
    let python = [
        "/usr/bin/python3",
        "/opt/homebrew/bin/python3",
        "/usr/local/bin/python3",
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).is_file())
    .ok_or("Python 3 is required for the local history index")?;
    let db = state.paths.panel_dir.join("history/library.sqlite");
    let output = tokio::process::Command::new(python)
        .args(["-c", INDEXER, action])
        .arg(db)
        .arg(payload.to_string())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    let result: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        format!(
            "History index failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })?;
    if !output.status.success() {
        return Err(result["error"]
            .as_str()
            .unwrap_or("History index failed")
            .to_string());
    }
    Ok(result)
}

#[tauri::command]
pub async fn history_scan(state: State<'_, AppState>) -> Result<Value, String> {
    run(&state, "scan", json!({"home":state.paths.home_dir})).await
}

#[tauri::command]
pub async fn history_stats(state: State<'_, AppState>) -> Result<Value, String> {
    run(&state, "stats", json!({})).await
}

#[tauri::command]
pub async fn history_search(
    state: State<'_, AppState>,
    query: String,
    source: String,
    offset: u64,
    include_subagents: bool,
) -> Result<Value, String> {
    run(
        &state,
        "search",
        json!({"query":query,"source":source,"offset":offset,"include_subagents":include_subagents}),
    )
    .await
}

#[tauri::command]
pub async fn history_read(
    state: State<'_, AppState>,
    id: String,
    offset: u64,
) -> Result<Value, String> {
    run(&state, "read", json!({"id":id,"offset":offset})).await
}

/// Materialize every available message, not only the pages loaded in the UI.
#[tauri::command]
pub async fn history_prepare(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    run(&state, "prepare", json!({"id":id})).await
}

fn native_identity(prepared: &Value) -> Result<(&str, &str, &str), String> {
    let t = &prepared["thread"];
    let source = t["source"].as_str().unwrap_or("");
    let backend =
        match source {
            "codex" => "codex",
            "claude_code" => "claude",
            _ => return Err(
                "This conversation uses a new coding session; native continuation is unavailable"
                    .into(),
            ),
        };
    let native_id = t["origin_id"].as_str().unwrap_or("");
    Uuid::parse_str(native_id).map_err(|_| "No valid native session ID".to_string())?;
    let cwd = t["cwd"].as_str().unwrap_or("");
    if prepared["native_candidate"] != true || !std::path::Path::new(cwd).is_absolute() {
        return Err("Native continuation requires an available main-session transcript and its original project".into());
    }
    Ok((backend, native_id, cwd))
}

/// Explicit user-selected continuation. Load the original engine's session in
/// Plan mode with no restored MCP grants; no prompt is sent by this command.
#[tauri::command]
pub async fn history_continue_native(
    state: State<'_, AppState>,
    id: String,
) -> Result<Value, String> {
    let prepared = run(&state, "prepare", json!({"id":id})).await?;
    let (backend, native_id, cwd) = native_identity(&prepared)?;
    if state
        .registry
        .list_sessions()
        .iter()
        .any(|r| r.acp_session_id.as_deref() == Some(native_id))
    {
        return Err("This native session is already open in Bomb Code. Use that thread, or start a new session from its full history.".into());
    }
    // A new local control record links to the original native engine ID. Never
    // copy grants or MCP settings from an older Bomb Code control record.
    let control_id = Uuid::new_v4();
    {
        let now = Utc::now();
        let title = prepared["thread"]["title"]
            .as_str()
            .unwrap_or("Imported conversation");
        let record = SessionRecord {
            id: control_id,
            cwd: cwd.into(),
            mode: "acp".into(),
            model: String::new(),
            status: "saved".into(),
            worktree: None,
            acp_session_id: Some(native_id.into()),
            metadata_json: json!({"metadata":{"backend":backend,"label":title,
                "approvalMode":"plan","projectRoot":cwd}})
            .to_string(),
            created_at: now,
            updated_at: now,
            message_count: 0,
        };
        state
            .persistence
            .upsert_session(&record)
            .map_err(|e| e.to_string())?;
    }
    let path = prepared["json_path"].as_str().ok_or("Prepared conversation has no file")?;
    let document: Value = serde_json::from_slice(&tokio::fs::read(path).await.map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let messages = document["messages"].as_array().ok_or("Prepared conversation has no messages")?;
    let entries: Vec<TranscriptEntry> = messages.iter().map(|m| TranscriptEntry {
        role: m["role"].as_str().unwrap_or("").into(),
        body: m["text"].as_str().unwrap_or("").into(),
        at: m["at"].as_str().filter(|a| !a.is_empty()).map(str::to_string)
            .unwrap_or_else(|| Utc::now().to_rfc3339()), seq: 0,
    }).collect();
    state.persistence.import_conversation(control_id, &entries).map_err(|e| e.to_string())?;
    let reference = format!("Complete conversation reference: {}\nStructured history: {}\n{} available messages. {}\nIf native loading is unavailable, read this full reference before continuing; prior approvals are not current authorization.",
        prepared["markdown_path"].as_str().unwrap_or(""),
        prepared["json_path"].as_str().unwrap_or(""), prepared["message_count"],
        prepared["notice"].as_str().unwrap_or(""));
    state
        .persistence
        .append_message(control_id, "system", &reference, Utc::now())
        .map_err(|e| e.to_string())?;
    // No MCP servers or historical access grants are copied into this record.
    crate::commands::resume_saved_session(
        &state,
        control_id,
        grok_config::Backend::from_key(backend),
        None,
        Some("plan".into()),
        Some(true),
        Some(false),
    )
    .await
    .map_err(|e| {
        format!(
            "Native continuation did not start: {e}. The complete reference is saved at {}",
            prepared["markdown_path"]
        )
    })?;
    let title: String = prepared["thread"]["title"].as_str().unwrap_or("Imported conversation")
        .chars().take(60).collect();
    state.registry.set_label(control_id, &title).map_err(|e| e.to_string())?;
    crate::commands::persist_session(&state, control_id).await;
    let snapshot = state
        .registry
        .get_snapshot(control_id)
        .map_err(|e| e.to_string())?;
    Ok(
        json!({"id":control_id,"brain_mode":snapshot.metadata.brain_mode.as_str(),"prepared":prepared}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_continuation_accepts_only_explicit_eligible_engine_records() {
        let id = Uuid::new_v4().to_string();
        let mut v = json!({"native_candidate":true,"thread":{"source":"codex","origin_id":id,"cwd":"/example"}});
        assert_eq!(native_identity(&v).unwrap().0, "codex");
        v["thread"]["source"] = json!("claude_code");
        assert_eq!(native_identity(&v).unwrap().0, "claude");
        v["native_candidate"] = json!(false);
        assert!(native_identity(&v).is_err());
        v["native_candidate"] = json!(true);
        v["thread"]["source"] = json!("chatgpt");
        assert!(native_identity(&v).is_err());
        v["thread"]["source"] = json!("codex");
        v["thread"]["origin_id"] = json!("parent/subagent/child");
        assert!(native_identity(&v).is_err());
    }
}

#[tauri::command]
pub async fn history_import(state: State<'_, AppState>, path: String) -> Result<Value, String> {
    let p = std::path::Path::new(&path);
    if !p.is_absolute()
        || !p.is_file()
        || !matches!(p.extension().and_then(|x| x.to_str()), Some("json" | "zip"))
    {
        return Err("Choose an existing JSON or ZIP conversation export".into());
    }
    run(&state, "import", json!({"path":path})).await
}

#[tauri::command]
pub async fn history_open_original(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let result = run(&state, "read", json!({"id":id,"offset":0})).await?;
    let url = result["thread"]["origin_url"].as_str().unwrap_or("");
    if !(url.starts_with("https://chatgpt.com/c/") || url.starts_with("https://claude.ai/chat/")) {
        return Err("This is a local transcript; its source path is shown in History".into());
    }
    let status = tokio::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .await
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Could not open the original conversation".into())
    }
}

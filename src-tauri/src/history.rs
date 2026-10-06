//! A local reference library, separate from live agent sessions and their approvals.
use crate::AppState;
use serde_json::{json, Value};
use tauri::State;

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

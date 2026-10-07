//! Provider-free source-concept candidates over the existing recall generation.
use crate::AppState;
use serde::Serialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_INPUT: usize = 16 * 1024 * 1024;
const MAX_OUTPUT: usize = 8 * 1024 * 1024;
const DICTIONARY: &str = include_str!("../../scripts/word_dictionary.py");
const RECALL: &str = include_str!("../../scripts/memory_recall.py");
const CANDIDATES: &str = include_str!("../../scripts/source_concept_candidates.py");

fn encode_bounded(value: &impl Serialize) -> Result<Vec<u8>, String> {
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_INPUT.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other(
                    "source-concept input budget exceeded",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded(Vec::new());
    serde_json::to_writer(&mut writer, value)
        .map_err(|_| "Current notes exceed source-concept input budget or cannot be serialized")?;
    Ok(writer.0)
}

fn validate_payload(payload: &Value) -> Result<(), String> {
    let fields = payload
        .as_object()
        .ok_or("Source-concept request must be an object")?;
    if fields
        .keys()
        .any(|k| !["conceptIds", "source", "scope", "limit"].contains(&k.as_str()))
    {
        return Err("Unsupported source-concept request fields".into());
    }
    let ids = payload["conceptIds"]
        .as_array()
        .ok_or("Select encoded source concepts")?;
    let mut seen = std::collections::BTreeSet::new();
    if ids.is_empty()
        || ids.len() > 8
        || ids.iter().any(|v| {
            v.as_str().is_none_or(|s| {
                s.is_empty() || s.chars().count() > 200 || s.contains('\0') || !seen.insert(s)
            })
        })
    {
        return Err("Select one to eight distinct bounded source concepts".into());
    }
    if let Some(source) = payload.get("source") {
        if !source.as_str().is_some_and(|s| {
            [
                "",
                "notes",
                "history",
                "chatgpt",
                "codex",
                "claude_code",
                "claude",
                "grok",
            ]
            .contains(&s)
        }) {
            return Err("Unsupported source-concept source filter".into());
        }
    }
    if let Some(scope) = payload.get("scope") {
        if !scope
            .as_str()
            .is_some_and(|s| s.chars().count() <= 512 && !s.contains('\0'))
        {
            return Err("Source-concept scope must be bounded text".into());
        }
    }
    if payload
        .get("limit")
        .is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=12).contains(&n)))
    {
        return Err("Source-concept limit must be one to twelve".into());
    }
    Ok(())
}

fn worker_code() -> Result<String, String> {
    let sources = serde_json::to_string(&[
        ("word_dictionary", DICTIONARY),
        ("memory_recall", RECALL),
        ("source_concept_candidates", CANDIDATES),
    ])
    .map_err(|_| "Could not serialize embedded source-concept worker")?;
    Ok(format!(
        r#"import sys, types, json
from pathlib import Path
for name, source in {sources}:
    module = types.ModuleType(name)
    module.__file__ = '<embedded-' + name + '>'
    sys.modules[name] = module
    exec(compile(source, module.__file__, 'exec'), module.__dict__)
bridge = sys.modules['source_concept_candidates']
try:
    raw = sys.stdin.buffer.read({MAX_INPUT} + 1)
    if len(raw) > {MAX_INPUT}:
        raise bridge.CandidateError('Current notes exceed source-concept input budget')
    payload = sys.modules['word_dictionary'].strict_json(raw.decode('utf-8'))
    value = bridge.run_host(Path(sys.argv[1]), payload)
    encoded = bridge.encode_result(value)
    if len(encoded) > {MAX_OUTPUT}:
        raise bridge.CandidateError('Complete source-concept result exceeds response budget')
    sys.stdout.buffer.write(encoded)
except Exception as error:
    print(json.dumps({{'schema': bridge.SCHEMA, 'status': 'unavailable', 'error': str(error)[:1600]}}))
    sys.exit(1)
"#
    ))
}

fn python(panel: &Path) -> Result<PathBuf, String> {
    [
        panel.join("wizard-joe/python-env/bin/python3"),
        PathBuf::from("/opt/homebrew/bin/python3.12"),
        PathBuf::from("/usr/local/bin/python3.12"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .ok_or_else(|| {
        "Wizard Joe's Python environment or Python 3.12 is required for source-concept recall"
            .into()
    })
}

async fn worker(panel: &Path, request: &[u8]) -> Result<Value, String> {
    if request.len() > MAX_INPUT {
        return Err("Current notes exceed source-concept input budget".into());
    }
    worker_with_python(&python(panel)?, panel, request).await
}

async fn worker_with_python(python: &Path, panel: &Path, request: &[u8]) -> Result<Value, String> {
    if request.len() > MAX_INPUT {
        return Err("Current notes exceed source-concept input budget".into());
    }
    let mut child = tokio::process::Command::new(python)
        .args(["-I", "-B", "-c", &worker_code()?])
        .arg(panel)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Could not start source-concept recall")?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("Source-concept input unavailable")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Source-concept output unavailable")?;
    let mut bytes = Vec::new();
    let operation = async {
        let write = async {
            stdin
                .write_all(request)
                .await
                .map_err(|_| "Source-concept input failed")?;
            drop(stdin);
            Ok::<_, String>(())
        };
        let read = async {
            stdout
                .take((MAX_OUTPUT + 1) as u64)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "Source-concept output failed")?;
            if bytes.len() > MAX_OUTPUT {
                return Err("Source-concept output exceeds its response budget".into());
            }
            Ok::<_, String>(())
        };
        tokio::try_join!(write, read)?;
        child
            .wait()
            .await
            .map_err(|_| "Source-concept worker failed".to_string())
    };
    let status = tokio::time::timeout(Duration::from_secs(180), operation)
        .await
        .map_err(|_| "Source-concept recall timed out; result unavailable")??;
    let result: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Source-concept worker returned invalid JSON")?;
    if !status.success() || result["status"] != "ready" {
        return Err(result["error"]
            .as_str()
            .unwrap_or("Source-concept recall unavailable")
            .into());
    }
    if result["schema"] != "bomb-code/source-concept-candidates/v1" || result["sourceFresh"] != true
    {
        return Err("Source-concept recall returned stale or incompatible evidence".into());
    }
    Ok(result)
}

pub(crate) async fn run(state: &AppState, payload: Value) -> Result<Value, String> {
    validate_payload(&payload)?;
    let _guard = crate::memory_recall::OPERATING
        .try_lock()
        .map_err(|_| "Local recall is busy; wait for its current operation")?;
    #[derive(Serialize)]
    struct HostRequest<'a> {
        #[serde(flatten)]
        options: &'a Value,
        notes: &'a [grok_memory::MemoryEntry],
    }
    let notes = state.memory.list(None).await;
    // Stream directly from native notes; no second unbounded JSON inventory.
    let request = encode_bounded(&HostRequest {
        options: &payload,
        notes: &notes,
    })?;
    let result = worker(&state.paths.panel_dir, &request).await?;
    let after = state.memory.list(None).await;
    if encode_bounded(&HostRequest {
        options: &payload,
        notes: &after,
    })? != request
    {
        return Err(
            "Current notes changed during source-concept recall; retry with the current index"
                .into(),
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn client_paths_and_invalid_options_are_rejected() {
        for payload in [
            json!({"conceptIds":["a"],"panel":"/tmp"}),
            json!({"conceptIds":["a"],"notes":[]}),
            json!({"conceptIds":["a"],"limit":13}),
            json!({"conceptIds":["a","a"]}),
            json!({"conceptIds":["a"],"source":"other"}),
        ] {
            assert!(validate_payload(&payload).is_err());
        }
        assert!(validate_payload(
            &json!({"conceptIds":["a"],"source":"notes","scope":"project","limit":12})
        )
        .is_ok());
    }
    #[tokio::test]
    #[ignore = "requires the installed pinned dictionary and current read-only archive"]
    async fn native_embedded_worker_bank_is_read_only_and_cited() {
        let panel = Path::new("/Users/paulcooper/.grok/control-panel");
        let request =
            serde_json::to_vec(&json!({"conceptIds":["pwn30:08420278-n"],"limit":5,"notes":[]}))
                .unwrap();
        let result = worker(panel, &request).await.unwrap();
        assert_eq!(result["providerCalls"], 0);
        assert_eq!(result["nativeWrites"], 0);
        assert_eq!(result["sourceIntegrity"], true);
        assert_eq!(result["referenceIntegrity"], true);
        assert!(result["scannedChunks"].as_u64().unwrap() > 0);
        assert!(!result["hits"].as_array().unwrap().is_empty());
        assert_eq!(
            result["evidenceValidatedCount"],
            result["hits"].as_array().unwrap().len()
        );
        for hit in result["hits"].as_array().unwrap() {
            assert!(hit["chunkId"].is_string());
            assert_eq!(hit["generation"], result["generation"]);
            assert_eq!(hit["sourceConcept"]["usageSelection"], "UNSELECTED");
        }
    }
    #[tokio::test]
    async fn worker_input_budget_precedes_runtime() {
        assert!(
            worker(Path::new("/unavailable"), &vec![b' '; MAX_INPUT + 1])
                .await
                .unwrap_err()
                .contains("input budget")
        );
        assert!(encode_bounded(&"x".repeat(MAX_INPUT)).is_err());
    }

    #[tokio::test]
    #[ignore = "requires the installed pinned dictionary and Python; isolated temporary index only"]
    async fn native_embedded_worker_notes_scope_and_stale_snapshot() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(
            std::env::temp_dir().join(format!("bomb-code-candidate-test-{}", uuid::Uuid::new_v4())),
        );
        std::fs::create_dir(&fixture.0).unwrap();
        // The test fixture uses a host-owned temporary panel. It never opens or
        // changes the installed recall index, archive or native notes.
        let notes = json!([
            {"id":"n1","scope":"project-a","content":"The bank approved a loan.","tags":[],"created_at":"","updated_at":""},
            {"id":"n2","scope":"project-b","content":"The bank has another loan.","tags":[],"created_at":"","updated_at":""}
        ]);
        let module_code = worker_code()
            .unwrap()
            .split("bridge =")
            .next()
            .unwrap()
            .to_owned();
        let setup = format!(
            r#"{module_code}
import sqlite3
panel = Path(sys.argv[1])
(panel/'history').mkdir()
with sqlite3.connect(panel/'history/library.sqlite') as db:
    db.executescript('CREATE TABLE threads(id TEXT PRIMARY KEY,source TEXT,origin_id TEXT,title TEXT,parent_id TEXT,coverage TEXT); CREATE TABLE messages(thread_id TEXT,message_id TEXT,role TEXT,text TEXT,at TEXT,seq INTEGER,truncated INTEGER,PRIMARY KEY(thread_id,message_id));')
    db.execute('INSERT INTO threads VALUES (?,?,?,?,?,?)', ('t','codex','t','Synthetic bank control','','complete fixture'))
    db.execute('INSERT INTO messages VALUES (?,?,?,?,?,?,?)', ('t','m','user','The bank approved a loan.','',1,0))
sys.modules['memory_recall'].dispatch('index', str(panel), {{'notes': {notes}}}, sys.modules['source_concept_candidates'].StoredModel(None))
"#,
            notes = serde_json::to_string(&notes).unwrap()
        );
        let python = python(Path::new("/Users/paulcooper/.grok/control-panel")).unwrap();
        let output = tokio::process::Command::new(&python)
            .args(["-I", "-B", "-c", &setup])
            .arg(&fixture.0)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Test helper uses the installed runtime with a temporary host panel.
        let request = json!({"conceptIds":["pwn30:08420278-n"],"source":"notes","scope":"project-a","limit":12,"notes":notes});
        let result = worker_with_python(&python, &fixture.0, &encode_bounded(&request).unwrap())
            .await
            .unwrap();
        assert_eq!(result["scannedChunks"], 3);
        assert_eq!(result["unfilteredCandidateCount"], 3);
        assert_eq!(result["candidateCount"], 1);
        assert_eq!(result["hits"][0]["kind"], "note");
        assert_eq!(result["hits"][0]["scope"], "project-a");
        assert_eq!(result["evidenceValidatedCount"], 1);
        assert_eq!(result["sourceIntegrity"], true);
        let mut stale = request;
        stale["notes"][0]["content"] = json!("A changed current note.");
        assert!(
            worker_with_python(&python, &fixture.0, &encode_bounded(&stale).unwrap())
                .await
                .unwrap_err()
                .contains("stale")
        );
    }
}

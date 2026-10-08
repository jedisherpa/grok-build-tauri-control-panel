//! Private references to immutable proposed readings; sources stay in their original stores.
use crate::state::AppState;
use grok_cdiss::retrieval::{compare_structured, RetrievalLimits, VerifiedRetrievalInput};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::State;
use tokio::sync::Mutex;
use uuid::Uuid;

const SCHEMA: &str = "bomb-code/meaning-memory/v1";
const MAX_RECEIPT: usize = 16 * 1024 * 1024;
const MAX_REFERENCE: usize = 16 * 1024;
const MAX_CATALOG: usize = 1024;
const MAX_RESPONSE: usize = 2 * 1024 * 1024;
static OPERATING: Mutex<()> = Mutex::const_new(());

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileRef {
    schema: String,
    request_id: String,
    receipt_sha256: String,
    state_hash: String,
    evidence_hash: String,
    kind: String,
    citation_ids: Vec<String>,
    created_at: String,
}
struct Validated {
    reference: ProfileRef,
    packet: Value,
    profile: VerifiedRetrievalInput,
    source_proof: Value,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn value_hash(value: &impl Serialize) -> Result<String, String> {
    Ok(hash(
        &serde_json::to_vec(value).map_err(|_| "Could not encode profile proof")?,
    ))
}
fn uuid(value: &str) -> Result<String, String> {
    let id = Uuid::parse_str(value)
        .map_err(|_| "Choose a saved receipt identifier")?
        .to_string();
    if id != value {
        return Err("Use a canonical saved receipt identifier".into());
    }
    Ok(id)
}
fn bounded_read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "Saved profile or receipt is unavailable")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return Err("Saved profile or receipt is not a bounded private file".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "Could not read saved profile")?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read saved profile")?;
    if bytes.len() > limit {
        return Err("Saved profile exceeds its byte budget".into());
    }
    Ok(bytes)
}
fn paths(panel: &Path) -> (PathBuf, PathBuf) {
    (
        panel.join("wizard-joe/receipts"),
        panel.join("wizard-joe/meaning-profiles"),
    )
}
fn catalog(dir: &Path) -> Result<Vec<String>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|_| "Could not inspect profile catalog")?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("Profile catalog must be a private directory".into());
    }
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|_| "Could not read profile catalog")? {
        let entry = entry.map_err(|_| "Could not read profile catalog")?;
        if ids.len() >= MAX_CATALOG {
            return Err("Profile catalog exceeds 1024 entries; no entries were dropped".into());
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let id = name
            .strip_suffix(".json")
            .ok_or("Unexpected file in private profile catalog")?;
        ids.push(uuid(id)?);
    }
    ids.sort();
    Ok(ids)
}
fn read_ref(dir: &Path, id: &str) -> Result<ProfileRef, String> {
    let id = uuid(id)?;
    let value: ProfileRef = serde_json::from_slice(&bounded_read(
        &dir.join(format!("{id}.json")),
        MAX_REFERENCE,
    )?)
    .map_err(|_| "Profile reference is malformed")?;
    if value.schema != "bomb-code/meaning-profile-ref/v1"
        || value.request_id != id
        || value.receipt_sha256.len() != 64
        || value.citation_ids.len() > 8
        || !["passage", "question-with-context", "question"].contains(&value.kind.as_str())
    {
        return Err("Profile reference identity is invalid".into());
    }
    Ok(value)
}
fn receipt(dir: &Path, id: &str) -> Result<(Value, String), String> {
    let id = uuid(id)?;
    let path = dir.join(format!("{id}.json"));
    let bytes = bounded_read(&path, MAX_RECEIPT)?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Original receipt is malformed")?;
    if value["schema"] != "bomb-code/joe-result/v1"
        || value["requestId"] != id
        || ["toolsDispatched", "approvalsGranted", "memoryCommitted"]
            .iter()
            .any(|k| value["authority"][k] != false)
    {
        return Err("Receipt identity or original source-memory boundary is invalid".into());
    }
    Ok((value, hash(&bytes)))
}
fn classify(packet: &Value, context: Option<&Value>) -> Result<(String, Vec<String>), String> {
    let Some(context) = context else {
        return Ok(("question".into(), Vec::new()));
    };
    let rows = context["evidence"]
        .as_array()
        .ok_or("Saved evidence has no citation rows")?;
    if rows.is_empty() || rows.len() > 8 {
        return Err("Saved evidence citation budget is invalid".into());
    }
    let ids = rows
        .iter()
        .map(|row| {
            row["citationId"]
                .as_str()
                .map(str::to_owned)
                .ok_or("Saved citation identity is missing")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let passage = rows.len() == 1
        && rows[0]["text"].as_str().is_some()
        && rows[0]["text"] == packet["sentence"];
    Ok((
        if passage {
            "passage"
        } else {
            "question-with-context"
        }
        .into(),
        ids,
    ))
}
fn same_citations(saved: &Value, current: &Value) -> Result<(), String> {
    // Exact fifteen-field rows include text/title/date/span and all source identities.
    let rows = saved["evidence"]
        .as_array()
        .ok_or("Saved citation rows are unavailable")?;
    let present = current["evidence"]
        .as_array()
        .ok_or("Current citation rows are unavailable")?;
    if rows.len() != present.len()
        || rows.is_empty()
        || rows.len() > 8
        || rows
            .iter()
            .zip(present)
            .any(|(a, b)| a.as_object().is_none_or(|m| m.len() != 15) || a != b)
        || saved["topic"] != current["topic"]
        || saved["schema"] != current["schema"]
    {
        return Err("Cited source identity/content/provenance changed; profile withheld".into());
    }
    Ok(())
}
async fn source_proof(state: &AppState, packet: &Value) -> Result<Value, String> {
    let memory_id = crate::wizard_joe::replay_memory_id(packet)?;
    let Some(id) = memory_id else {
        return Ok(
            json!({"status":"not-applicable","currentGeneration":Value::Null,"citationIds":[]}),
        );
    };
    uuid(id)?;
    let saved = &packet["memoryEvidence"]["context"];
    crate::wizard_joe::validate_replay_memory(packet, saved)?;
    let (_, ids) = classify(packet, Some(saved))?;
    let status = crate::memory_recall::run(state, "status", json!({})).await?;
    let generation = status["generation"]
        .as_str()
        .ok_or("Memory index is unavailable; cited profile withheld")?;
    let evidence = crate::memory_recall::run(
        state,
        "evidence",
        json!({"generation":generation,"chunkIds":ids}),
    )
    .await?;
    let current =
        crate::memory_recall::context_from(&evidence, saved["topic"].as_str().unwrap_or(""))?;
    same_citations(saved, &current)?;
    Ok(
        json!({"status":"current-source-validated","currentGeneration":generation,
        "savedGeneration":saved["generation"],"citationIds":ids,"citationRowsHash":value_hash(&current["evidence"])?,
        "generationRefreshed":saved["generation"] != generation}),
    )
}
async fn validate(
    state: &AppState,
    reference: ProfileRef,
    coverage: &Value,
) -> Result<Validated, String> {
    let (receipts, _) = paths(&state.paths.panel_dir);
    let (packet, bytes_hash) = receipt(&receipts, &reference.request_id)?;
    if bytes_hash != reference.receipt_sha256 {
        return Err("Original receipt bytes changed; profile withheld".into());
    }
    crate::wizard_joe::validate_replay_reference(&packet, coverage)?;
    let source_proof = source_proof(state, &packet).await?;
    let original =
        crate::wizard_joe::replay_memory_id(&packet)?.map(|_| &packet["memoryEvidence"]["context"]);
    let (kind, ids) = classify(&packet, original)?;
    let profile = VerifiedRetrievalInput::from_native(&packet)?;
    if reference.kind != kind
        || reference.citation_ids != ids
        || packet_state(&packet, &profile)?
            != (
                reference.state_hash.clone(),
                reference.evidence_hash.clone(),
            )
    {
        return Err("Profile reference no longer matches its complete native reading".into());
    }
    Ok(Validated {
        reference,
        packet,
        profile,
        source_proof,
    })
}
fn packet_state(
    _packet: &Value,
    profile: &VerifiedRetrievalInput,
) -> Result<(String, String), String> {
    // No audit inventory is duplicated in storage; the native state is rebuilt.
    let state = grok_cdiss::analyze_joe_result(
        profile.native_packet(),
        None,
        &grok_cdiss::Config::default(),
    )
    .map_err(|e| e.to_string())?;
    Ok((state.state_hash, state.evidence_hash))
}
fn private_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        let m =
            std::fs::symlink_metadata(dir).map_err(|_| "Could not inspect private directory")?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err("Profile directory is not private".into());
        }
    }
    std::fs::create_dir_all(dir).map_err(|_| "Could not create private profile directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Could not restrict profile directory")?;
    }
    Ok(())
}
fn persist_ref(dir: &Path, reference: &ProfileRef) -> Result<bool, String> {
    private_dir(dir)?;
    let path = dir.join(format!("{}.json", uuid(&reference.request_id)?));
    if path.exists() {
        let existing = read_ref(dir, &reference.request_id)?;
        if value_hash(&existing)? != value_hash(reference)? {
            return Err("Existing immutable profile differs; no replacement allowed".into());
        }
        return Ok(false);
    }
    if catalog(dir)?.len() >= MAX_CATALOG {
        return Err("Profile catalog is full (1024); source receipt was preserved".into());
    }
    let bytes = serde_json::to_vec(reference).map_err(|_| "Could not encode profile reference")?;
    if bytes.len() > MAX_REFERENCE {
        return Err("Profile reference exceeds byte budget".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Could not save immutable profile reference")?;
    file.write_all(&bytes)
        .map_err(|_| "Could not write profile reference")?;
    file.sync_all()
        .map_err(|_| "Could not sync profile reference")?;
    Ok(true)
}
async fn import_one(
    state: &AppState,
    id: &str,
    coverage: &Value,
) -> Result<(ProfileRef, bool), String> {
    let (receipts, profiles) = paths(&state.paths.panel_dir);
    if profiles.join(format!("{}.json", uuid(id)?)).exists() {
        let original = read_ref(&profiles, id)?;
        let checked = validate(state, original, coverage).await?;
        return Ok((checked.reference, false));
    }
    let (packet, receipt_sha256) = receipt(&receipts, id)?;
    crate::wizard_joe::validate_replay_reference(&packet, coverage)?;
    source_proof(state, &packet).await?;
    let memory =
        crate::wizard_joe::replay_memory_id(&packet)?.map(|_| &packet["memoryEvidence"]["context"]);
    let (kind, citation_ids) = classify(&packet, memory)?;
    let profile = VerifiedRetrievalInput::from_native(&packet)?;
    let (state_hash, evidence_hash) = packet_state(&packet, &profile)?;
    let reference = ProfileRef {
        schema: "bomb-code/meaning-profile-ref/v1".into(),
        request_id: id.into(),
        receipt_sha256,
        state_hash,
        evidence_hash,
        kind,
        citation_ids,
        created_at: packet["at"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(128)
            .collect(),
    };
    // Close the source/byte freshness window immediately before committing the reference.
    let checked = validate(state, reference.clone(), coverage).await?;
    let saved = persist_ref(&profiles, &checked.reference)?;
    Ok((reference, saved))
}
pub(crate) async fn accumulate(state: &AppState, packet: &Value) -> Result<Value, String> {
    let _guard = OPERATING
        .try_lock()
        .map_err(|_| "Meaning memory is busy; import this saved analysis later")?;
    let id = packet["requestId"]
        .as_str()
        .ok_or("Saved analysis has no identity")?;
    let coverage = crate::word_shapes::run(json!({"action":"coverage"})).await?;
    let (reference, saved) = import_one(state, id, &coverage).await?;
    Ok(
        json!({"status":"ready","profileId":reference.request_id,"kind":reference.kind,"profileSaved":saved,
        "meaning":"Immutable reference to a proposed reading; source memories were not changed"}),
    )
}
fn summary(
    reference: &ProfileRef,
    packet: Option<&Value>,
    proof: Option<&Value>,
    reason: Option<String>,
) -> Value {
    let sentence = packet.and_then(|p| p["sentence"].as_str()).unwrap_or("");
    json!({"profileId":reference.request_id,"requestId":reference.request_id,"kind":reference.kind,
        "sentence":sentence.chars().take(512).collect::<String>(),"sentenceTruncated":sentence.chars().count()>512,
        "language":packet.map(|p| &p["language"]),"threadId":packet.map(|p| &p["threadId"]),
        "citationIds":reference.citation_ids,"createdAt":reference.created_at,"available":proof.is_some(),
        "reason":reason,"sourceProof":proof,"receiptSha256":reference.receipt_sha256,
        "stateHash":reference.state_hash,"evidenceHash":reference.evidence_hash,"proposal":true})
}
fn reference_summary(coverage: &Value) -> Value {
    let reference = &coverage["reference"];
    json!({"manifestSha256":reference["manifestSha256"],"graphHash":reference["graphHash"],
        "modelSnapshotHash":reference["modelSnapshotHash"],"modelId":reference["modelId"],
        "senseSnapImplementationHashes":reference["senseSnapImplementationHashes"]})
}
fn selected_definitions(packet: &Value) -> Result<Value, String> {
    let mut output_bytes = 0usize;
    let readings = packet["interpretation"]["binding"]["readings"]
        .as_array()
        .into_iter()
        .flatten();
    let mut out = Vec::new();
    for reading in readings {
        let mut occurrences = Vec::new();
        for atom in reading["bound_usage"]["atoms"]
            .as_array()
            .into_iter()
            .flatten()
        {
            for source in atom["selected_source_bindings"]
                .as_array()
                .into_iter()
                .flatten()
            {
                output_bytes = output_bytes
                    .checked_add(
                        serde_json::to_vec(&source["sense"])
                            .map_err(|_| "Invalid source sense")?
                            .len()
                            + 1024,
                    )
                    .ok_or("Definition summary overflow")?;
                if output_bytes > MAX_RESPONSE / 2 {
                    return Err(
                        "Definition summary exceeds its budget; choose a shorter reading".into(),
                    );
                }
            }
            let definitions: Vec<_> = atom["selected_source_bindings"].as_array().into_iter().flatten().map(|s|
                json!({"senseId":s["sense"]["id"],"lemma":s["sense"]["lemma"],"language":s["sense"]["language"],
                "definition":s["sense"]["definition"],"conceptIds":s["concept_ids"]})).collect();
            occurrences.push(json!({"atomId":atom["atom"]["id"],"surface":atom["atom"]["surface"],"span":atom["atom"]["span"],
                "selectedDefinitions":definitions,"unselectedAlternativeCount":packet["interpretation"]["binding"]["source_packet"]["atoms"]
                    .as_array().into_iter().flatten().find(|v|v["atom"]["id"]==atom["atom"]["id"])
                    .and_then(|v|v["candidates"].as_array()).map(|rows| rows.iter().filter(|v|
                        !atom["selected_source_bindings"].as_array().into_iter().flatten().any(|selected|selected["sense"]["id"]==v["sense"]["id"])).count())}));
        }
        out.push(json!({"readingId":reading["id"],"occurrences":occurrences,"uncertainty":reading["selection_uncertainty"]}));
    }
    Ok(json!(out))
}
fn card(id: &str, comparison: &Value) -> (Value, Vec<Value>) {
    let mut pairs = Vec::new();
    let drafts = Vec::new();
    for pair in comparison["readingPairs"].as_array().into_iter().flatten() {
        let event = &pair["eventLocal"];
        let differences: Vec<_> = event["eventPairs"].as_array().into_iter().flatten().map(|e|
            json!({"queryEventId":e["queryEventId"],"candidateEventId":e["candidateEventId"],"predicateEqual":e["predicateEqual"],
                "rolesEqual":e["rolesEqual"],"polarityEqual":e["polarityEqual"],"modalityEqual":e["modalityEqual"],
                "query":e["query"]["canonical"],"candidate":e["candidate"]["canonical"],
                "querySourceEvent":e["query"]["sourceEvent"],"candidateSourceEvent":e["candidate"]["sourceEvent"]})).collect();
        let collisions: Vec<_> = pair["geometryCorrespondences"].as_array().into_iter().flatten().filter(|g|g["rootCollision"]==true)
            .map(|g| json!({"queryAtomId":g["queryAtomId"],"candidateAtomId":g["candidateAtomId"],"sameRoot":g["sameRoot"],
                "sameSourceIdentity":g["sameSourceIdentity"],"sameConceptId":g["sameConceptId"],"semanticEquivalenceInferred":false})).collect();
        let fine:Vec<_> = pair["geometryCorrespondences"].as_array().into_iter().flatten().map(|g|
            json!({"queryAtomId":g["queryAtomId"],"candidateAtomId":g["candidateAtomId"],"distances":g["distances"],"status":g["status"],
                "querySenseId":g["queryActivation"]["sense_id"],"candidateSenseId":g["candidateActivation"]["sense_id"],
                "queryConceptId":g["queryActivation"]["concept_id"],"candidateConceptId":g["candidateActivation"]["concept_id"],
                "queryRootId":g["queryActivation"]["placement"]["root_id"],"candidateRootId":g["candidateActivation"]["placement"]["root_id"],
                "sameRoot":g["sameRoot"],"rootCollision":g["rootCollision"],"semanticEquivalenceInferred":false})).collect();
        pairs.push(json!({"queryReadingId":pair["queryReadingId"],"candidateReadingId":pair["candidateReadingId"],
            "sourceSenseDistance":pair["sourceSense"]["distance"],"assertedConceptDistance":pair["assertedConcept"]["distance"],
            "senseSnapDictionaryDistance":pair["senseSnap"]["dictionaryDistance"],"contextPinDistance":pair["senseSnap"]["contextPinDistance"],
            "eventLocalDistance":event["conceptNormalizedDistance"],"eventDifferences":differences,
            "multiplicity":pair["multiplicity"],"coverage":pair["coverage"],"uncertainty":pair["uncertainty"],
            "geometry":{"correspondenceCount":fine.len(),"rootCollisions":collisions,"fineDistances":fine},
            "canonicalLinksAndReferencesEqual":event["canonicalLinksAndReferencesEqual"],
            "signatureCollisions":event["signatureCollisions"],"atomIdentityCollisions":event["atomIdentityCollisions"],
            "speechActEqual":event["speechActEqual"],"graphIsomorphismEstablished":false,
            "queryFrame":event["queryOriginalFrame"],"candidateFrame":event["candidateOriginalFrame"],
            "queryCanonicalGraph":event["queryCanonicalGraph"],"candidateCanonicalGraph":event["candidateCanonicalGraph"],
            "contextPolicy":pair["senseSnap"]["contextPolicy"],
            "contextCenters":pair["senseSnap"]["centerComparisons"].as_array().into_iter().flatten().map(|c|
                json!({"queryAtomId":c["queryAtomId"],"candidateAtomId":c["candidateAtomId"],
                    "queryOrigin":c["queryCenter"]["origin"],"candidateOrigin":c["candidateCenter"]["origin"],
                    "queryMeetingId":c["queryCenter"]["meeting_id"],"candidateMeetingId":c["candidateCenter"]["meeting_id"],
                    "scopeAndEvidenceCompatible":c["scopeAndEvidenceCompatible"],"exactDeclaredCenterMatch":c["exactDeclaredCenterMatch"],
                    "fineGeometry":c["fineGeometry"],"nativeGeometrySupported":c["nativeGeometrySupported"]})).collect::<Vec<_>>()}));
    }
    (
        json!({"candidateProfileId":id,"readingPairs":pairs,"scopes":comparison["scopes"],
        "originalCdissSourceDistance":comparison["originalCdissSourceDistance"],"originalCdissStructureDistance":comparison["originalCdissStructureDistance"],
        "originalPartitionChanged":comparison["originalPartitionChanged"],"overallRelevanceScore":Value::Null}),
        drafts,
    )
}
fn compare_ids(payload: &Value) -> Result<(String, Vec<String>), String> {
    let query = ["queryProfileId", "queryReceiptId", "requestId"]
        .iter()
        .find_map(|k| payload[k].as_str())
        .ok_or("Choose a saved query profile")?;
    let candidates = payload["candidateProfileIds"]
        .as_array()
        .ok_or("Choose candidate profiles")?;
    if candidates.is_empty() || candidates.len() > 3 {
        return Err("Choose one to three candidate profiles".into());
    }
    let mut unique = BTreeSet::new();
    let ids = candidates
        .iter()
        .map(|v| uuid(v.as_str().ok_or("Invalid candidate profile")?))
        .collect::<Result<Vec<_>, _>>()?;
    if ids.iter().any(|id| !unique.insert(id.clone())) {
        return Err("Candidate profile identifiers must be distinct".into());
    }
    Ok((uuid(query)?, ids))
}
async fn compared_inputs(
    state: &AppState,
    payload: &Value,
    coverage: &Value,
) -> Result<(Validated, Vec<Validated>, String), String> {
    let (query, ids) = compare_ids(payload)?;
    let (_, dir) = paths(&state.paths.panel_dir);
    let query = validate(state, read_ref(&dir, &query)?, coverage).await?;
    let mut candidates = Vec::new();
    let mut total = serde_json::to_vec(&query.packet)
        .map_err(|_| "Invalid query receipt")?
        .len();
    for id in ids {
        let reference = read_ref(&dir, &id)?;
        let meta =
            std::fs::symlink_metadata(paths(&state.paths.panel_dir).0.join(format!("{id}.json")))
                .map_err(|_| "Candidate receipt unavailable")?;
        total = total
            .checked_add(meta.len() as usize)
            .ok_or("Receipt byte overflow")?;
        if total > 32 * 1024 * 1024 {
            return Err("Combined original receipts exceed 32 MiB; choose fewer candidates".into());
        }
        candidates.push(validate(state, reference, coverage).await?);
    }
    let fingerprint = value_hash(
        &json!({"query":query.reference,"querySources":query.source_proof,
        "candidates":candidates.iter().map(|v|json!({"profile":v.reference,"sources":v.source_proof})).collect::<Vec<_>>(),
        "reference":reference_summary(coverage)}),
    )?;
    Ok((query, candidates, fingerprint))
}
fn finish(mut value: Value) -> Result<Value, String> {
    value["schema"] = json!(SCHEMA);
    value["authority"] = json!({"toolsDispatched":false,"approvalsGranted":false,"memoryCommitted":false,"providerCalled":false});
    value["notice"]=json!("Proposed interpretations with separate measured signals. Geometry, shared concepts and zero distances do not establish intended meaning, agreement or complete equivalence.");
    if serde_json::to_vec(&value)
        .map_err(|_| "Could not encode meaning response")?
        .len()
        > MAX_RESPONSE
    {
        return Err(
            "Complete explanation summary exceeds 2 MiB; choose fewer/smaller saved profiles"
                .into(),
        );
    }
    Ok(value)
}
#[tauri::command]
pub async fn meaning_memory(
    state: State<'_, AppState>,
    action: String,
    payload: Value,
) -> Result<Value, String> {
    let target=serde_json::json!({"action":action}).to_string();
    crate::operations::recorded(&state.event_bus,"meaning_memory",target,async {
    run(&state, &action, payload).await
    }).await
}
pub(crate) async fn run(state: &AppState, action: &str, payload: Value) -> Result<Value, String> {
    if !payload.is_object()
        || serde_json::to_vec(&payload)
            .map_err(|_| "Invalid meaning request")?
            .len()
            > 16 * 1024
    {
        return Err("Meaning memory accepts bounded options and receipt identifiers only".into());
    }
    if action == "source_candidates" {
        return crate::meaning_candidates::run(state, payload).await;
    }
    validate_request(action, &payload)?;
    let _guard = OPERATING
        .try_lock()
        .map_err(|_| "Meaning memory is busy; retry after the current operation")?;
    let coverage = crate::word_shapes::run(json!({"action":"coverage"})).await;
    let (receipts, profiles) = paths(&state.paths.panel_dir);
    match action {
        "status" => {
            let offset = payload["offset"].as_u64().unwrap_or(0) as usize;
            let limit = payload["limit"].as_u64().unwrap_or(20) as usize;
            if offset > MAX_CATALOG || !(1..=20).contains(&limit) {
                return Err("Profile page is outside its budget".into());
            }
            let ids = catalog(&profiles)?;
            let mut refs = Vec::new();
            let mut bad = 0usize;
            for id in &ids {
                match read_ref(&profiles, id) {
                    Ok(r) => refs.push(r),
                    Err(_) => bad += 1,
                }
            }
            refs.sort_by(|a, b| {
                b.created_at
                    .cmp(&a.created_at)
                    .then_with(|| a.request_id.cmp(&b.request_id))
            });
            let mut rows = Vec::new();
            let mut available = 0usize;
            for reference in refs.iter().skip(offset).take(limit) {
                let checked = match &coverage {
                    Ok(c) => validate(state, reference.clone(), c).await,
                    Err(e) => Err(e.clone()),
                };
                match checked {
                    Ok(v) => {
                        available += 1;
                        rows.push(summary(
                            reference,
                            Some(&v.packet),
                            Some(&v.source_proof),
                            None,
                        ));
                    }
                    Err(e) => rows.push(summary(reference, None, None, Some(e))),
                }
            }
            finish(
                json!({"status":"ready","profiles":rows,"nextOffset":if offset+limit<refs.len(){Some(offset+limit)}else{None},
                "counts":{"total":ids.len(),"passage":refs.iter().filter(|r|r.kind=="passage").count(),
                    "questionWithContext":refs.iter().filter(|r|r.kind=="question-with-context").count(),"question":refs.iter().filter(|r|r.kind=="question").count(),
                    "currentlyVerified":available,"withheldInPage":rows.len()-available,"unchecked":refs.len()-rows.len(),"malformedReferences":bad},
                "reference":coverage.as_ref().ok().map(reference_summary),"catalogLimit":MAX_CATALOG}),
            )
        }
        "import" => {
            let coverage = coverage?;
            let ids = if let Some(ids) = payload.get("requestIds") {
                let rows = ids.as_array().ok_or("Choose saved receipt identifiers")?;
                if rows.is_empty() || rows.len() > 64 {
                    return Err("Import accepts one to 64 saved receipts".into());
                }
                rows.iter()
                    .map(|v| uuid(v.as_str().ok_or("Invalid saved receipt")?))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                let pending = catalog(&receipts)?
                    .into_iter()
                    .filter(|id| !profiles.join(format!("{id}.json")).exists())
                    .collect::<Vec<_>>();
                select_import_batch(&pending, import_cursor(&state.paths.panel_dir)?.as_deref())
            };
            let attempted = ids.len();
            let last_attempted = ids.last().cloned();
            let mut imported = 0;
            let mut existing = 0;
            let mut withheld = Vec::new();
            for id in ids {
                match import_one(state, &id, &coverage).await {
                    Ok((_, true)) => imported += 1,
                    Ok((_, false)) => existing += 1,
                    Err(reason) => withheld.push(json!({"requestId":id,"reason":reason})),
                }
            }
            if payload.get("requestIds").is_none() {
                if let Some(last) = &last_attempted {
                    save_import_cursor(&state.paths.panel_dir, last)?;
                }
            }
            finish(
                json!({"status":"ready","imported":imported,"existing":existing,"withheld":withheld,"profileSaved":imported>0,
                    "attempted":attempted,"lastAttemptedReceiptId":last_attempted,
                    "importProgress":"Default batches advance a private UUID cursor even across unavailable receipts; reaching the end wraps for later retries."}),
            )
        }
        "compare" | "validate" => {
            let coverage = coverage?;
            let (query, candidates, fingerprint) =
                compared_inputs(state, &payload, &coverage).await?;
            if action == "validate" {
                if payload["comparisonFingerprint"].as_str() != Some(fingerprint.as_str()) {
                    return Err("Comparison inputs or current source evidence changed; compare again before copying".into());
                }
                return finish(json!({"status":"ready","comparisonFingerprint":fingerprint}));
            }
            let query_readings = selected_definitions(&query.packet)?;
            let candidate_readings = candidates
                .iter()
                .map(|v| {
                    Ok(json!({"profileId":v.reference.request_id,
                "readings":selected_definitions(&v.packet)?}))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut summary_budget =
                serde_json::to_vec(&json!({"q":query_readings,"c":candidate_readings}))
                    .map_err(|_| "Could not size definition summary")?
                    .len();
            let mut cards = Vec::new();
            let mut questions = Vec::new();
            let limits = RetrievalLimits {
                max_candidates: 3,
                ..RetrievalLimits::default()
            };
            for candidate in &candidates {
                let comparison = compare_structured(&query.profile, &candidate.profile, &limits)?;
                // Charge only the displayed fields, before cloning their arrays.
                // Native full-source proof fanout remains under the core 32 MiB cap.
                summary_budget = summary_budget
                    .checked_add(card_size_upper_bound(&comparison)?)
                    .ok_or("Comparison summary overflow")?;
                if summary_budget > MAX_RESPONSE {
                    return Err("Complete comparison exceeds the 2 MiB explanation budget; choose fewer/smaller profiles".into());
                }
                let (mut item, _) = card(&candidate.reference.request_id, &comparison);
                let drafts = prepare_drafts(
                    &query.packet,
                    &candidate.packet,
                    &candidate.reference.request_id,
                    &comparison,
                )?;
                if questions.len() + drafts.len() > 48 {
                    return Err(
                        "Clarification evidence exceeds 48 drafts; compare fewer/smaller profiles"
                            .into(),
                    );
                }
                item["provenance"] = json!({"queryReceiptSha256":query.reference.receipt_sha256,"candidateReceiptSha256":candidate.reference.receipt_sha256,
                    "querySources":query.source_proof,"candidateSources":candidate.source_proof});
                cards.push(item);
                questions.extend(drafts);
            }
            let current_coverage = crate::word_shapes::run(json!({"action":"coverage"})).await?;
            let (_, _, after) = compared_inputs(state, &payload, &current_coverage).await?;
            if after != fingerprint {
                return Err("Sources changed during comparison; result withheld".into());
            }
            finish(
                json!({"status":"ready","query":summary(&query.reference,Some(&query.packet),Some(&query.source_proof),None),
                "candidates":candidates.iter().map(|v|summary(&v.reference,Some(&v.packet),Some(&v.source_proof),None)).collect::<Vec<_>>(),
                "queryReadings":query_readings, "candidateReadings":candidate_readings,
                "cards":cards,"unsentQuestions":questions,"comparisonFingerprint":fingerprint,
                "reference":reference_summary(&coverage),"overallRelevanceScore":Value::Null,
                "evidencePolicy":"Summary fields refer to complete original immutable receipts. Every retained reading pair is compared; original source alternatives/centers/frames/activations remain in those receipts."}),
            )
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> Value {
        json!({"schema":"bomb-code/recalled-evidence/v1","generation":"old","topic":"loans","evidence":[{
            "citationId":"a".repeat(64),"text":"The bank approved the loan.","title":"Authored source","source":"codex","kind":"history",
            "threadId":"t","messageId":"m","noteId":"","scope":"","role":"user","at":"2026-10-06",
            "span":{"start":0,"end":27},"sourceHash":"b".repeat(64),"excerptHash":"c".repeat(64),"coverage":"local transcript"}]})
    }
    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../crates/grok_cdiss/tests/fixtures/joe-bank.json"
        ))
        .unwrap()
    }
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("bomb-code-meaning-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn passage_subject_is_exact_single_cited_text_and_questions_remain_questions() {
        let c = context();
        let mut p = fixture();
        p["sentence"] = c["evidence"][0]["text"].clone();
        assert_eq!(classify(&p, Some(&c)).unwrap().0, "passage");
        p["sentence"] = json!("What does bank mean here?");
        assert_eq!(classify(&p, Some(&c)).unwrap().0, "question-with-context");
        assert_eq!(classify(&p, None).unwrap().0, "question");
        let mut two = c.clone();
        two["evidence"]
            .as_array_mut()
            .unwrap()
            .push(c["evidence"][0].clone());
        p["sentence"] = c["evidence"][0]["text"].clone();
        assert_eq!(classify(&p, Some(&two)).unwrap().0, "question-with-context");
    }
    #[test]
    fn generation_refresh_accepts_identical_sources_but_every_changed_or_missing_field_is_withheld()
    {
        let c = context();
        let mut refreshed = c.clone();
        refreshed["generation"] = json!("new");
        assert!(same_citations(&c, &refreshed).is_ok());
        for key in c["evidence"][0].as_object().unwrap().keys() {
            let mut changed = refreshed.clone();
            changed["evidence"][0][key] = json!("changed");
            assert!(same_citations(&c, &changed).is_err(), "{key}");
            let mut missing = refreshed.clone();
            missing["evidence"][0].as_object_mut().unwrap().remove(key);
            assert!(same_citations(&c, &missing).is_err(), "{key}");
        }
        let mut extra = refreshed.clone();
        extra["evidence"][0]["approved"] = json!(true);
        assert!(same_citations(&c, &extra).is_err());
    }
    #[test]
    fn private_reference_is_immutable_and_original_receipt_tampering_changes_pin() {
        let dir = temp();
        let id = Uuid::new_v4().to_string();
        let mut packet = fixture();
        packet["requestId"] = json!(id);
        packet["authority"] =
            json!({"toolsDispatched":false,"approvalsGranted":false,"memoryCommitted":false});
        let bytes = serde_json::to_vec(&packet).unwrap();
        std::fs::write(dir.join(format!("{id}.json")), &bytes).unwrap();
        let (loaded, pin) = receipt(&dir, &id).unwrap();
        assert_eq!(loaded, packet);
        assert_eq!(pin, hash(&bytes));
        let profiles = dir.join("profiles");
        let reference = ProfileRef {
            schema: "bomb-code/meaning-profile-ref/v1".into(),
            request_id: id.clone(),
            receipt_sha256: pin.clone(),
            state_hash: "s".into(),
            evidence_hash: "e".into(),
            kind: "question".into(),
            citation_ids: vec![],
            created_at: "".into(),
        };
        assert!(persist_ref(&profiles, &reference).unwrap());
        assert!(!persist_ref(&profiles, &reference).unwrap());
        let mut changed = reference.clone();
        changed.kind = "passage".into();
        assert!(persist_ref(&profiles, &changed).is_err());
        packet["sentence"] = json!("A changed source.");
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::to_vec(&packet).unwrap(),
        )
        .unwrap();
        assert_ne!(receipt(&dir, &id).unwrap().1, pin);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&profiles).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(profiles.join(format!("{id}.json")))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn request_and_file_bounds_reject_paths_duplicates_and_oversize() {
        assert!(uuid("../receipt").is_err());
        let id = Uuid::new_v4().to_string();
        assert!(compare_ids(&json!({"queryProfileId":id,"candidateProfileIds":[id,id]})).is_err());
        assert!(
            compare_ids(&json!({"queryProfileId":id,"candidateProfileIds":[id,id,id,id]})).is_err()
        );
        let dir = temp();
        std::fs::write(dir.join("large"), [0u8; 33]).unwrap();
        assert!(bounded_read(&dir.join("large"), 32).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("large"), dir.join("link")).unwrap();
            assert!(bounded_read(&dir.join("link"), 64).is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn native_summary_preserves_all_reading_pairs_distinct_signals_and_unsent_frame_differences() {
        let q = fixture();
        let c: Value = serde_json::from_str(include_str!(
            "../../crates/grok_cdiss/tests/fixtures/joe-negative.json"
        ))
        .unwrap();
        let q = VerifiedRetrievalInput::from_native(&q).unwrap();
        let c = VerifiedRetrievalInput::from_native(&c).unwrap();
        let comparison = compare_structured(&q, &c, &RetrievalLimits::default()).unwrap();
        let (card, drafts) = card("candidate", &comparison);
        assert_eq!(
            card["readingPairs"].as_array().unwrap().len(),
            comparison["readingPairs"].as_array().unwrap().len()
        );
        assert!(card["overallRelevanceScore"].is_null());
        assert_eq!(
            card["readingPairs"][0]["graphIsomorphismEstablished"],
            false
        );
        assert!(drafts.is_empty());
        let drafts = prepare_drafts(
            q.native_packet(),
            c.native_packet(),
            "candidate",
            &comparison,
        )
        .unwrap();
        assert!(drafts
            .iter()
            .any(|d| d["kind"] == "polarity" && d["unsent"] == true));
        assert_eq!(
            card["readingPairs"][0]["sourceSenseDistance"],
            comparison["readingPairs"][0]["sourceSense"]["distance"]
        );
        assert_eq!(
            card["readingPairs"][0]["eventDifferences"][0]["querySourceEvent"],
            comparison["readingPairs"][0]["eventLocal"]["eventPairs"][0]["query"]["sourceEvent"]
        );
    }
}

fn validate_request(action: &str, payload: &Value) -> Result<(), String> {
    let map = payload
        .as_object()
        .ok_or("Meaning memory accepts an options object")?;
    let allowed: &[&str] = match action {
        "status" => &["offset", "limit"],
        "import" => &["requestIds"],
        "compare" => &[
            "queryProfileId",
            "queryReceiptId",
            "requestId",
            "candidateProfileIds",
        ],
        "validate" => &[
            "queryProfileId",
            "queryReceiptId",
            "requestId",
            "candidateProfileIds",
            "comparisonFingerprint",
        ],
        _ => return Err("Unknown meaning memory action".into()),
    };
    if map.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err("Meaning memory accepts only declared options and saved identifiers; no paths, packets or providers".into());
    }
    if action == "status" {
        for (key, min, max) in [("offset", 0, MAX_CATALOG as u64), ("limit", 1, 20)] {
            if let Some(v) = map.get(key) {
                if v.as_u64().is_none_or(|n| n < min || n > max) {
                    return Err("Profile page options must be bounded integers".into());
                }
            }
        }
    }
    if action == "import" {
        if let Some(v) = map.get("requestIds") {
            let ids = v.as_array().ok_or("Import identifiers must be an array")?;
            if ids.is_empty() || ids.len() > 64 {
                return Err("Import accepts one to 64 saved receipts".into());
            }
            let mut seen = BTreeSet::new();
            for id in ids {
                if !seen.insert(uuid(id.as_str().ok_or("Invalid receipt identifier")?)?) {
                    return Err("Import identifiers must be distinct".into());
                }
            }
        }
    }
    if action == "compare" || action == "validate" {
        if ["queryProfileId", "queryReceiptId", "requestId"]
            .iter()
            .filter(|k| map.contains_key(**k))
            .count()
            != 1
        {
            return Err("Choose exactly one query receipt identifier".into());
        }
        compare_ids(payload)?;
    }
    if action == "validate"
        && payload["comparisonFingerprint"].as_str().is_none_or(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err("Comparison fingerprint must be a lowercase SHA-256".into());
    }
    Ok(())
}
fn reading<'a>(packet: &'a Value, id: &Value) -> Result<&'a Value, String> {
    packet["interpretation"]["binding"]["readings"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["id"] == *id)
        .ok_or_else(|| "Compared reading is missing from its original receipt".into())
}
fn surface<'a>(reading: &'a Value, id: &Value) -> Result<&'a str, String> {
    reading["bound_usage"]["atoms"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["atom"]["id"] == *id)
        .and_then(|a| a["atom"]["surface"].as_str())
        .ok_or_else(|| "Event occurrence is missing from its original reading".into())
}
fn role_labels(frame: &Value, reading: &Value) -> Result<String, String> {
    frame["roles"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            Ok(format!(
                "{}={}",
                r["role"].as_str().ok_or("Role name is missing")?,
                surface(reading, &r["atom_id"])?
            ))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|rows| rows.join(", "))
}
fn add_draft(
    out: &mut Vec<Value>,
    seen: &mut BTreeSet<String>,
    kind: &str,
    text: String,
    basis: Value,
    id: &str,
) -> Result<(), String> {
    if text.encode_utf16().count() > 4000 {
        return Err(
            "Complete clarification draft exceeds 4000 UTF-16 units; choose a shorter passage"
                .into(),
        );
    }
    let size = serde_json::to_vec(&basis)
        .map_err(|_| "Could not size clarification evidence")?
        .len()
        .checked_add(text.len())
        .and_then(|n| n.checked_add(1024))
        .ok_or("Clarification byte overflow")?;
    let existing = serde_json::to_vec(out)
        .map_err(|_| "Could not size clarification drafts")?
        .len();
    if existing
        .checked_add(size)
        .is_none_or(|n| n > MAX_RESPONSE / 2)
    {
        return Err("Complete clarification evidence exceeds its byte budget; compare fewer/smaller profiles".into());
    }
    let key = value_hash(&json!({"kind":kind,"basis":basis}))?;
    if !seen.insert(key) {
        return Ok(());
    }
    if out.len() >= 48 {
        return Err("Clarification evidence exceeds 48 drafts; no drafts were truncated".into());
    }
    out.push(json!({"candidateProfileId":id,"kind":kind,"text":text,"basis":basis,"unsent":true,"sendStatus":"unsent"}));
    Ok(())
}
fn prepare_drafts(
    q: &Value,
    c: &Value,
    id: &str,
    comparison: &Value,
) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for pair in comparison["readingPairs"].as_array().into_iter().flatten() {
        let qr = reading(q, &pair["queryReadingId"])?;
        let cr = reading(c, &pair["candidateReadingId"])?;
        let basis = json!({"queryReadingId":pair["queryReadingId"],"candidateReadingId":pair["candidateReadingId"],"candidateProfileId":id});
        let events = pair["eventLocal"]["eventPairs"]
            .as_array()
            .ok_or("Native event pairs missing")?;
        let mut qc = BTreeMap::<String, usize>::new();
        let mut cc = BTreeMap::<String, usize>::new();
        for e in events.iter().filter(|e| e["predicateEqual"] == true) {
            *qc.entry(e["queryEventId"].to_string()).or_default() += 1;
            *cc.entry(e["candidateEventId"].to_string()).or_default() += 1;
        }
        for e in events {
            if e["predicateEqual"] != true
                || qc.get(&e["queryEventId"].to_string()) != Some(&1)
                || cc.get(&e["candidateEventId"].to_string()) != Some(&1)
            {
                continue;
            }
            let qe = &e["query"]["sourceEvent"];
            let ce = &e["candidate"]["sourceEvent"];
            let word = surface(qr, &qe["predicate"])?;
            let mut ev = basis.clone();
            ev["queryEventId"] = e["queryEventId"].clone();
            ev["candidateEventId"] = e["candidateEventId"].clone();
            ev["queryPredicateAtomId"] = qe["predicate"].clone();
            ev["candidatePredicateAtomId"] = ce["predicate"].clone();
            for (flag,kind,text) in [
                ("rolesEqual","event-participants",format!("For {word:?}, who performs the action and who is affected? Current roles: {}; comparison roles: {}.",role_labels(qe,qr)?,role_labels(ce,cr)?)),
                ("polarityEqual","polarity",format!("For {word:?}, do you mean that the action happens or that it does not happen? The retained event frames differ in polarity.")),
                ("modalityEqual","modality",format!("For {word:?}, is the action actual, possible, required, or conditional? The retained event frames differ in modality.")),
            ] {if e[flag]==false {let mut proof=ev.clone();proof["signal"]=json!(flag);add_draft(&mut out,&mut seen,kind,text,proof,id)?;}}
        }
        let graph = &pair["eventLocal"];
        for (field,kind,text) in [
            ("links","event-links","How are these events related in time or condition? The retained event links differ."),
            ("references","references","Which earlier occurrence does this reference point to? The retained reference links differ."),
        ] {if graph["queryCanonicalGraph"][field]!=graph["candidateCanonicalGraph"][field] {
            let mut proof=basis.clone();proof["queryGraphEvidence"]=graph["queryCanonicalGraph"][field].clone();
            proof["candidateGraphEvidence"]=graph["candidateCanonicalGraph"][field].clone();
            add_draft(&mut out,&mut seen,kind,text.into(),proof,id)?;
        }}
        if pair["senseSnap"]["contextPinDistance"]["totalVariation"]
            .as_f64()
            .is_some_and(|v| v > 0.0)
        {
            let mut proof = basis.clone();
            proof["queryPins"] = pair["senseSnap"]["queryContextPinDistribution"].clone();
            proof["candidatePins"] = pair["senseSnap"]["candidateContextPinDistribution"].clone();
            add_draft(&mut out,&mut seen,"context","Do these uses belong to the same personal or topic context? Their explicitly bound context pins differ.".into(),proof,id)?;
        }
    }
    let mut choices = BTreeMap::<String, Vec<Value>>::new();
    for r in q["interpretation"]["binding"]["readings"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for atom in r["bound_usage"]["atoms"].as_array().into_iter().flatten() {
            let senses: BTreeSet<_> = atom["selected_source_bindings"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["sense"]["id"].as_str())
                .collect();
            if senses.is_empty() {
                continue;
            }
            choices.entry(atom["atom"]["id"].as_str().ok_or("Original atom has no identifier")?.into()).or_default()
                .push(json!({"readingId":r["id"],"sourceSenseIds":senses,"surface":atom["atom"]["surface"]}));
        }
    }
    for (atom, rows) in choices {
        let distinct: BTreeSet<_> = rows
            .iter()
            .map(|r| r["sourceSenseIds"].to_string())
            .collect();
        if distinct.len() > 1 {
            let word = rows[0]["surface"].as_str().unwrap_or("");
            add_draft(&mut out,&mut seen,"source-sense",format!("Which dictionary sense of {word:?} fits what you mean here? Review the definitions attached to these retained readings."),
                json!({"queryAtomId":atom,"readingSenseChoices":rows}),id)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn undeclared_packets_paths_and_ambiguous_query_aliases_are_rejected_before_workers() {
        let id = Uuid::new_v4().to_string();
        for value in [Value::Null, json!(true), json!(-1), json!(0.5), json!("1")] {
            assert!(validate_request("status", &json!({"offset":value})).is_err());
            assert!(validate_request("status", &json!({"limit":value})).is_err());
        }
        assert!(validate_request("status", &json!({"limit":0})).is_err());
        assert!(validate_request("status", &json!({"limit":21})).is_err());
        assert!(validate_request("status", &json!({"path":"/tmp/private"})).is_err());
        assert!(validate_request(
            "compare",
            &json!({"queryProfileId":id,"requestId":id,"candidateProfileIds":[id]})
        )
        .is_err());
        assert!(validate_request(
            "compare",
            &json!({"queryProfileId":id,"candidateProfileIds":[id],"packet":{}})
        )
        .is_err());
        assert!(validate_request("validate",&json!({"queryProfileId":id,"candidateProfileIds":[id],"comparisonFingerprint":"A".repeat(64)})).is_err());
        assert!(validate_request("validate",&json!({"queryProfileId":id,"candidateProfileIds":[id],"comparisonFingerprint":"a".repeat(64)})).is_ok());
        assert!(validate_request("import", &json!({"requestIds":[id,id]})).is_err());
    }
    #[test]
    fn repeated_predicates_abstain_from_event_correspondence_but_links_context_and_senses_remain_questionable(
    ) {
        let q: Value = serde_json::from_str(include_str!(
            "../../crates/grok_cdiss/tests/fixtures/joe-bank.json"
        ))
        .unwrap();
        let c: Value = serde_json::from_str(include_str!(
            "../../crates/grok_cdiss/tests/fixtures/joe-negative.json"
        ))
        .unwrap();
        let qp = VerifiedRetrievalInput::from_native(&q).unwrap();
        let cp = VerifiedRetrievalInput::from_native(&c).unwrap();
        let mut comparison = compare_structured(&qp, &cp, &RetrievalLimits::default()).unwrap();
        let event = comparison["readingPairs"][0]["eventLocal"]["eventPairs"][0].clone();
        comparison["readingPairs"][0]["eventLocal"]["eventPairs"]
            .as_array_mut()
            .unwrap()
            .push(event);
        comparison["readingPairs"][0]["eventLocal"]["queryCanonicalGraph"]["links"] =
            json!([{"from":"e1","to":"e2","kind":"condition"}]);
        comparison["readingPairs"][0]["eventLocal"]["queryCanonicalGraph"]["references"] =
            json!([{"from":"bank","to":"loan"}]);
        comparison["readingPairs"][0]["senseSnap"]["contextPinDistance"] =
            json!({"totalVariation":1.0});
        let drafts = prepare_drafts(&q, &c, "candidate", &comparison).unwrap();
        assert!(!drafts
            .iter()
            .any(|d| ["polarity", "modality", "event-participants"]
                .contains(&d["kind"].as_str().unwrap())));
        assert!(drafts.iter().any(|d| d["kind"] == "event-links"));
        assert!(drafts.iter().any(|d| d["kind"] == "references"));
        assert!(drafts.iter().any(|d| d["kind"] == "context"));
        let mut query = q.clone();
        let mut second = query["interpretation"]["binding"]["readings"][0].clone();
        second["id"] = json!("r2");
        second["bound_usage"]["atoms"][0]["selected_source_bindings"][0]["sense"]["id"] =
            json!("another-attested-sense");
        query["interpretation"]["binding"]["readings"]
            .as_array_mut()
            .unwrap()
            .push(second);
        let drafts = prepare_drafts(&query, &c, "candidate", &comparison).unwrap();
        assert!(drafts.iter().any(|d| d["kind"] == "source-sense"
            && d["basis"]["readingSenseChoices"].as_array().unwrap().len() == 2));
    }
    #[test]
    fn draft_overflow_fails_explicitly_instead_of_dropping_traceable_questions() {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for index in 0..48 {
            add_draft(
                &mut out,
                &mut seen,
                "sense",
                "Question".into(),
                json!({"atom":index}),
                "candidate",
            )
            .unwrap();
        }
        assert!(add_draft(
            &mut out,
            &mut seen,
            "sense",
            "Question".into(),
            json!({"atom":48}),
            "candidate"
        )
        .is_err());
        assert_eq!(out.len(), 48);
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;
    /// Actual embedded source validator, with an isolated archive/index; no installed mutation.
    #[tokio::test]
    #[ignore = "requires installed Python; creates only a temporary authored source/index fixture"]
    async fn native_host_citations_survive_generation_refresh_but_source_edit_is_withheld() {
        let dir = std::env::temp_dir().join(format!("bomb-code-meaning-host-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let source = serde_json::to_string(include_str!("../../scripts/memory_recall.py")).unwrap();
        let code = format!(
            r#"import sys, types, json, sqlite3
from pathlib import Path
m=types.ModuleType('memory_recall')
m.__file__='<embedded-memory-recall>'
sys.modules[m.__name__]=m
exec(compile({source},m.__file__,'exec'),m.__dict__)
class NoEmbedding:
 def identity(self): return None
 def embed(self,texts): raise RuntimeError('No embeddings or provider calls in this test')
panel=Path(sys.argv[1])
(panel/'history').mkdir()
with sqlite3.connect(panel/'history/library.sqlite') as db:
 db.executescript('CREATE TABLE threads(id TEXT PRIMARY KEY,source TEXT,origin_id TEXT,title TEXT,parent_id TEXT,coverage TEXT); CREATE TABLE messages(thread_id TEXT,message_id TEXT,role TEXT,text TEXT,at TEXT,seq INTEGER,truncated INTEGER,PRIMARY KEY(thread_id,message_id));')
 db.execute('INSERT INTO threads VALUES (?,?,?,?,?,?)',('t','codex','t','Authored loan source','','complete authored fixture'))
 db.execute('INSERT INTO messages VALUES (?,?,?,?,?,?,?)',('t','m','user','The bank approved the loan.','2026-10-06',1,0))
payload={{'notes':[]}}
first=m.dispatch('index',str(panel),payload,NoEmbedding())
with sqlite3.connect(panel/'memory-recall/recall.sqlite') as db:
 cid=db.execute('SELECT chunk_id FROM chunks LIMIT 1').fetchone()[0]
before=m.dispatch('evidence',str(panel),dict(payload,generation=first['generation'],chunkIds=[cid]),NoEmbedding())
second=m.dispatch('index',str(panel),payload,NoEmbedding())
after=m.dispatch('evidence',str(panel),dict(payload,generation=second['generation'],chunkIds=[cid]),NoEmbedding())
with sqlite3.connect(panel/'history/library.sqlite') as db:
 db.execute('UPDATE messages SET text=? WHERE message_id=?',('The bank did not approve the loan.','m'))
try:
 m.dispatch('evidence',str(panel),dict(payload,generation=second['generation'],chunkIds=[cid]),NoEmbedding())
 raise RuntimeError('Changed source was incorrectly accepted')
except m.RecallError as e:
 stale=str(e)
print(json.dumps({{'before':before,'after':after,'stale':stale}}))
"#
        );
        let python =
            Path::new("/Users/paulcooper/.grok/control-panel/wizard-joe/python-env/bin/python3");
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            tokio::process::Command::new(python)
                .args(["-I", "-B", "-c", &code])
                .arg(&dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.len() < 128 * 1024);
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        let before = crate::memory_recall::context_from(&result["before"], "loans").unwrap();
        let after = crate::memory_recall::context_from(&result["after"], "loans").unwrap();
        assert_ne!(before["generation"], after["generation"]);
        same_citations(&before, &after).unwrap();
        let packet = json!({"sentence":"The bank approved the loan."});
        assert_eq!(classify(&packet, Some(&after)).unwrap().0, "passage");
        assert!(result["stale"]
            .as_str()
            .is_some_and(|s| s.contains("stale") || s.contains("changed")));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Conservative size of only the displayed fields; complete proof arrays are
/// deliberately referenced through their immutable original receipt.
fn card_size_upper_bound(comparison: &Value) -> Result<usize, String> {
    fn charge(total: &mut usize, value: &Value) -> Result<(), String> {
        *total = total
            .checked_add(
                serde_json::to_vec(value)
                    .map_err(|_| "Could not size explanation evidence")?
                    .len(),
            )
            .ok_or("Explanation byte overflow")?;
        if *total > MAX_RESPONSE {
            return Err(
                "Complete explanation fields exceed 2 MiB; choose fewer/smaller profiles".into(),
            );
        }
        Ok(())
    }
    fn overhead(total: &mut usize, n: usize) -> Result<(), String> {
        *total = total.checked_add(n).ok_or("Explanation byte overflow")?;
        if *total > MAX_RESPONSE {
            return Err(
                "Complete explanation fields exceed 2 MiB; choose fewer/smaller profiles".into(),
            );
        }
        Ok(())
    }
    let mut total = 16384usize;
    charge(&mut total, &comparison["scopes"])?;
    for pair in comparison["readingPairs"].as_array().into_iter().flatten() {
        overhead(&mut total, 8192)?;
        for value in [
            &pair["sourceSense"]["distance"],
            &pair["assertedConcept"]["distance"],
            &pair["senseSnap"]["dictionaryDistance"],
            &pair["senseSnap"]["contextPinDistance"],
            &pair["multiplicity"],
            &pair["coverage"],
            &pair["uncertainty"],
            &pair["eventLocal"]["queryOriginalFrame"],
            &pair["eventLocal"]["candidateOriginalFrame"],
            &pair["eventLocal"]["queryCanonicalGraph"],
            &pair["eventLocal"]["candidateCanonicalGraph"],
        ] {
            charge(&mut total, value)?;
        }
        for event in pair["eventLocal"]["eventPairs"]
            .as_array()
            .into_iter()
            .flatten()
        {
            overhead(&mut total, 1024)?;
            for value in [
                &event["queryEventId"],
                &event["candidateEventId"],
                &event["query"]["canonical"],
                &event["candidate"]["canonical"],
                &event["query"]["sourceEvent"],
                &event["candidate"]["sourceEvent"],
            ] {
                charge(&mut total, value)?;
            }
        }
        for g in pair["geometryCorrespondences"]
            .as_array()
            .into_iter()
            .flatten()
        {
            overhead(&mut total, 2048)?; // includes the separate collision diagnostic
            for value in [
                &g["queryAtomId"],
                &g["candidateAtomId"],
                &g["distances"],
                &g["queryActivation"]["sense_id"],
                &g["candidateActivation"]["sense_id"],
                &g["queryActivation"]["concept_id"],
                &g["candidateActivation"]["concept_id"],
                &g["queryActivation"]["placement"]["root_id"],
                &g["candidateActivation"]["placement"]["root_id"],
            ] {
                charge(&mut total, value)?;
                charge(&mut total, value)?;
            }
        }
        for center in pair["senseSnap"]["centerComparisons"]
            .as_array()
            .into_iter()
            .flatten()
        {
            overhead(&mut total, 1024)?;
            for value in [
                &center["queryAtomId"],
                &center["candidateAtomId"],
                &center["queryCenter"]["origin"],
                &center["candidateCenter"]["origin"],
                &center["queryCenter"]["meeting_id"],
                &center["candidateCenter"]["meeting_id"],
                &center["fineGeometry"],
            ] {
                charge(&mut total, value)?;
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod response_tests {
    use super::*;
    #[test]
    fn compact_estimate_bounds_actual_cards_without_charging_unshown_source_proof_repetition() {
        let q: Value = serde_json::from_str(include_str!(
            "../../crates/grok_cdiss/tests/fixtures/joe-bank.json"
        ))
        .unwrap();
        let q = VerifiedRetrievalInput::from_native(&q).unwrap();
        let mut comparison = compare_structured(&q, &q, &RetrievalLimits::default()).unwrap();
        let before = card_size_upper_bound(&comparison).unwrap();
        comparison["readingPairs"][0]["occurrenceMatches"] =
            json!([{"querySourceBindings":"x".repeat(3*1024*1024)}]);
        assert_eq!(card_size_upper_bound(&comparison).unwrap(), before);
        let (card, _) = card("candidate", &comparison);
        assert!(serde_json::to_vec(&card).unwrap().len() <= before);
        comparison["readingPairs"][0]["eventLocal"]["queryOriginalFrame"] =
            json!({"unexpectedHugeFrame":"x".repeat(MAX_RESPONSE)});
        assert!(card_size_upper_bound(&comparison).is_err());
    }
    #[test]
    fn complete_draft_length_budget_matches_frontend_utf16_and_never_truncates() {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        add_draft(
            &mut out,
            &mut seen,
            "test",
            "🙂".repeat(2000),
            json!({}),
            "candidate",
        )
        .unwrap();
        assert!(add_draft(
            &mut out,
            &mut seen,
            "test",
            "🙂".repeat(2001),
            json!({"different":true}),
            "candidate"
        )
        .is_err());
        assert_eq!(
            out[0]["text"].as_str().unwrap().encode_utf16().count(),
            4000
        );
        assert_eq!(out.len(), 1);
    }
    #[tokio::test]
    #[ignore = "reads only prior private originals/results; no source/index/catalog/provider mutation"]
    async fn prior_saved_original_profiles_fit_complete_compact_card_budget() {
        let dir=PathBuf::from(std::env::var("BOMB_CODE_PRIVATE_RETRIEVAL_FIXTURES").expect("Set BOMB_CODE_PRIVATE_RETRIEVAL_FIXTURES to the host-owned prior private result directory"));
        let mut count = 0;
        for id in [2, 3, 4, 5, 6] {
            let result: Value = serde_json::from_slice(
                &bounded_read(&dir.join(format!("saved-{id}.json")), 32 * 1024 * 1024).unwrap(),
            )
            .unwrap();
            assert_eq!(result["status"], "ready");
            let comparison = &result["retrieval"]["candidateComparisons"][0]["comparison"];
            let estimate = card_size_upper_bound(comparison).unwrap();
            let (card, _) = card("readonly-original", comparison);
            assert!(serde_json::to_vec(&card).unwrap().len() <= estimate);
            count += 1;
        }
        assert_eq!(count, 5);
    }
}

fn select_import_batch(pending: &[String], cursor: Option<&str>) -> Vec<String> {
    let start = cursor
        .and_then(|last| pending.iter().position(|id| id.as_str() > last))
        .unwrap_or(0);
    pending.iter().skip(start).take(64).cloned().collect()
}
fn import_cursor(panel: &Path) -> Result<Option<String>, String> {
    let path = panel.join("wizard-joe/meaning-import-progress.json");
    if !path.exists() {
        return Ok(None);
    }
    let value: Value = serde_json::from_slice(&bounded_read(&path, 4096)?)
        .map_err(|_| "Import progress is malformed; select explicit receipt identifiers")?;
    if value["schema"] != "bomb-code/meaning-import-progress/v1" {
        return Err("Import progress schema is invalid".into());
    }
    Ok(Some(uuid(
        value["lastReceiptId"]
            .as_str()
            .ok_or("Import progress identifier is missing")?,
    )?))
}
fn save_import_cursor(panel: &Path, last: &str) -> Result<(), String> {
    let dir = panel.join("wizard-joe");
    private_dir(&dir)?;
    let staged = dir.join(format!(".meaning-import-{}.tmp", Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let bytes = serde_json::to_vec(
        &json!({"schema":"bomb-code/meaning-import-progress/v1","lastReceiptId":uuid(last)?}),
    )
    .map_err(|_| "Could not encode import progress")?;
    let mut file = options
        .open(&staged)
        .map_err(|_| "Could not save private import progress")?;
    file.write_all(&bytes)
        .map_err(|_| "Could not write import progress")?;
    file.sync_all()
        .map_err(|_| "Could not sync import progress")?;
    std::fs::rename(&staged, dir.join("meaning-import-progress.json"))
        .map_err(|_| "Could not advance import progress")?;
    Ok(())
}
#[cfg(test)]
mod import_tests {
    use super::*;
    #[test]
    fn failed_first_batch_cannot_starve_later_saved_receipts_and_cursor_wraps_for_retry() {
        let ids = (1..=130)
            .map(|n| format!("00000000-0000-0000-0000-{n:012x}"))
            .collect::<Vec<_>>();
        let first = select_import_batch(&ids, None);
        assert_eq!(first.len(), 64);
        let second = select_import_batch(&ids, first.last().map(String::as_str));
        assert_eq!(second[0], ids[64]);
        assert_eq!(second.len(), 64);
        let third = select_import_batch(&ids, second.last().map(String::as_str));
        assert_eq!(third, ids[128..]);
        let wrap = select_import_batch(&ids, third.last().map(String::as_str));
        assert_eq!(wrap, first);
        let panel =
            std::env::temp_dir().join(format!("bomb-code-import-progress-{}", Uuid::new_v4()));
        std::fs::create_dir(&panel).unwrap();
        assert_eq!(import_cursor(&panel).unwrap(), None);
        save_import_cursor(&panel, &first[63]).unwrap();
        assert_eq!(import_cursor(&panel).unwrap(), Some(first[63].clone()));
        save_import_cursor(&panel, &second[63]).unwrap();
        assert_eq!(import_cursor(&panel).unwrap(), Some(second[63].clone()));
        std::fs::remove_dir_all(panel).unwrap();
    }
}

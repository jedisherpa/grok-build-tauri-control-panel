//! Source-identity and role-structured CDISS observations. No dispatch API exists.
//! Allocated mass describes retained proposals; it is never calibrated confidence.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

mod word_shapes;
pub use word_shapes::word_shapes;
pub mod retrieval;

pub const ALGORITHM: &str = "bomb-code/cdiss-source-structure/v1";
const SCHEMA: &str = "bomb-code/cdiss-state/v1";
const INPUT_LIMIT: usize = 32 * 1024 * 1024;
const GEOMETRY_VERSION: &str = "semantic-e8/geometry-standard-lex/v1";
const PLANE_SHA: &str = "fd8ac2058aee11bfb68ed69dec7aa424db56db465e8e61724cfdbda89e58472a";

#[derive(Debug, thiserror::Error)]
pub enum CdissError {
    #[error("CDISS unavailable: {0}")]
    Invalid(String),
    #[error("CDISS serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}
type Result<T> = std::result::Result<T, CdissError>;
fn invalid<T>(why: impl Into<String>) -> Result<T> {
    Err(CdissError::Invalid(why.into()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub retention: f64,
    pub max_features: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            retention: 0.5,
            max_features: 8192,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if !self.retention.is_finite()
            || !(0.0..=1.0).contains(&self.retention)
            || !(16..=8192).contains(&self.max_features)
        {
            return invalid("invalid finite retention or feature budget");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Basis {
    pub thread_id: Option<String>,
    pub language: String,
    pub context_hash: String,
    pub manifest_sha256: String,
    pub source_graph_hash: String,
    pub model_snapshot_hash: String,
    pub model_id: String,
    pub geometry_version: String,
    pub reference_digest: String,
    pub provider: String,
    pub provider_model: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Feature {
    pub key: String,
    pub mass: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AtomObservation {
    pub atom_id: String,
    pub surface: String,
    pub span: [usize; 2],
    pub selected_identities: Vec<String>,
    pub all_candidate_sense_ids: Vec<String>,
    pub context_centers: Vec<Value>,
    pub alternative_count: usize,
    pub geometrically_unmapped: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadingObservation {
    pub reading_id: String,
    pub frame: Value,
    pub atoms: Vec<AtomObservation>,
    /// Exact native activation objects, including source IDs, fine position and residual.
    pub geometry: Vec<Value>,
    pub uncertainty: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Observation {
    pub reading_count: usize,
    pub atom_count: usize,
    pub event_count: usize,
    pub alternative_count: usize,
    pub mapped_mass: f64,
    pub unmapped_mass: f64,
    pub allocation: String,
    pub source_vector: Vec<Feature>,
    pub structure_vector: Vec<Feature>,
    /// Fingerprint of occurrence multiplicities and alternative partitions, independent of local IDs.
    pub partition_hash: String,
    pub readings: Vec<ReadingObservation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Distances {
    pub total_variation: f64,
    pub jensen_shannon_distance: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Continuity {
    pub status: String,
    pub reasons: Vec<String>,
    pub source_distance: Option<Distances>,
    pub structure_distance: Option<Distances>,
    pub partition_changed: Option<bool>,
    pub source_mixture: Vec<Feature>,
    pub structure_mixture: Vec<Feature>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct State {
    pub schema: String,
    pub algorithm_version: String,
    pub config_digest: String,
    pub basis: Basis,
    pub request_id: String,
    pub input_hash: String,
    pub evidence_hash: String,
    pub observation: Observation,
    pub continuity: Continuity,
    pub previous_state_hash: Option<String>,
    pub state_hash: String,
}

/// Deterministically observe a successful, native read-only Joe result.
/// Previous state is used only for an explicitly requested comparison; never model input.
pub fn analyze_joe_result(
    result: &Value,
    previous: Option<&State>,
    config: &Config,
) -> Result<State> {
    config.validate()?;
    let encoded = serde_json::to_vec(result)?;
    if encoded.len() > INPUT_LIMIT {
        return invalid(
            "native result exceeds 32 MiB; split passage without discarding alternatives",
        );
    }
    if result["schema"] != "bomb-code/joe-result/v1" {
        return invalid("unsupported native Joe result schema");
    }
    for key in ["toolsDispatched", "approvalsGranted", "memoryCommitted"] {
        if result["authority"][key] != false {
            return invalid("missing read-only native receipt");
        }
    }
    let binding = &result["interpretation"]["binding"];
    if binding["schema"] != "semantic-e8/structured-interpretation/v2"
        || binding["status"] != "grounded-model-proposal"
        || binding["interpretation_authority"] != "model-proposed"
    {
        return invalid("successful source-bound proposal required");
    }
    for key in [
        "dispatch_authority_granted",
        "dictionary_alignment_changed",
        "coordinates_changed",
    ] {
        if binding[key] != false {
            return invalid("native binding boundary receipt missing");
        }
    }
    let request = &binding["request"];
    let sentence = occurrence_text(result, "sentence")?;
    let language = text(result, "language", 32)?;
    if request["sentence"] != sentence || request["language"] != language {
        return invalid("native request does not match exact input");
    }
    if !request["context"].is_object() {
        return invalid("explicit native context required");
    }
    let shared = &request["shared_reference"];
    if shared["geometry"]["version"] != GEOMETRY_VERSION
        || shared["geometry"]["plane_source_sha256"] != PLANE_SHA
    {
        return invalid("unsupported pinned E8 convention");
    }
    if shared["sense_snap"]["schema"] != "semantic-e8/sensesnap-usage/v1" {
        return invalid("SenseSnap implementation reference missing");
    }
    let implementation = shared["sense_snap"]["implementation_hashes"]
        .as_object()
        .ok_or_else(|| CdissError::Invalid("SenseSnap code hashes missing".into()))?;
    if implementation.is_empty() || implementation.len() > 16 {
        return invalid("invalid SenseSnap implementation pins");
    }
    for value in implementation.values() {
        require_hash(value)?;
    }
    let scale = shared["lattice_scale"]
        .as_f64()
        .ok_or_else(|| CdissError::Invalid("lattice scale missing".into()))?;
    if !scale.is_finite() || scale <= 0.0 || scale > 1e6 {
        return invalid("invalid explicit lattice scale");
    }
    let graph_hash = require_hash(&shared["source_graph_hash"])?;
    let model_hash = require_hash(&shared["model_snapshot_hash"])?;
    let manifest = require_hash(&result["reference"]["manifestSha256"])?;
    if result["referenceRequested"]["manifestSha256"] != manifest {
        return invalid("reference manifest does not match requested pin");
    }
    if binding["source_packet"]["shared_reference"] != *shared
        || binding["receipt"]["source_graph_hash"] != graph_hash
        || binding["receipt"]["model_id"] != shared["model_id"]
        || binding["receipt"]["shared_reference_id"] != shared["reference_id"]
    {
        return invalid("native source/reference mismatch");
    }
    let thread_id = match &result["threadId"] {
        Value::Null => None,
        Value::String(s) if !s.is_empty() && s.len() <= 200 => Some(s.clone()),
        _ => return invalid("invalid thread scope"),
    };
    let basis = Basis {
        thread_id,
        language: language.to_owned(),
        context_hash: digest(&request["context"])?,
        manifest_sha256: manifest.to_owned(),
        source_graph_hash: graph_hash.to_owned(),
        model_snapshot_hash: model_hash.to_owned(),
        model_id: text(shared, "model_id", 200)?.to_owned(),
        geometry_version: GEOMETRY_VERSION.into(),
        reference_digest: digest(shared)?,
        provider: text(result, "provider", 100)?.to_owned(),
        provider_model: text(result, "model", 200)?.to_owned(),
    };
    let observation = observe(binding, sentence, scale, config.max_features)?;
    let config_digest = digest(&json!({"algorithm":ALGORITHM,"config":config}))?;
    let mut reasons = Vec::new();
    let compatible = if let Some(prior) = previous {
        if prior.schema != SCHEMA
            || prior.algorithm_version != ALGORITHM
            || state_digest(prior)? != prior.state_hash
            || !valid_vector(&prior.continuity.source_mixture, config.max_features)
            || !valid_vector(&prior.continuity.structure_mixture, config.max_features)
            || !valid_vector(&prior.observation.source_vector, config.max_features)
            || !valid_vector(&prior.observation.structure_vector, config.max_features)
        {
            return invalid("previous CDISS state failed integrity or numeric validation");
        }
        if prior.basis != basis {
            reasons.push("thread, context, language, provider or pinned reference changed".into());
        }
        if basis.thread_id.is_none() {
            reasons.push("unscoped reviews cannot carry continuity".into());
        }
        if prior.config_digest != config_digest {
            reasons.push("reducer configuration changed".into());
        }
        reasons.is_empty()
    } else {
        false
    };
    let (source_mixture, structure_mixture, source_distance, structure_distance, partition_changed) =
        if compatible {
            let prior = previous.expect("compatible requires previous state");
            (
                mix(
                    &prior.continuity.source_mixture,
                    &observation.source_vector,
                    config.retention,
                    config.max_features,
                )?,
                mix(
                    &prior.continuity.structure_mixture,
                    &observation.structure_vector,
                    config.retention,
                    config.max_features,
                )?,
                Some(distance(
                    &prior.observation.source_vector,
                    &observation.source_vector,
                )?),
                Some(distance(
                    &prior.observation.structure_vector,
                    &observation.structure_vector,
                )?),
                Some(prior.observation.partition_hash != observation.partition_hash),
            )
        } else {
            (
                observation.source_vector.clone(),
                observation.structure_vector.clone(),
                None,
                None,
                None,
            )
        };
    let mut state = State {
        schema: SCHEMA.into(),
        algorithm_version: ALGORITHM.into(),
        config_digest,
        basis,
        request_id: text(result, "requestId", 200)?.into(),
        input_hash: digest(&json!({"sentence":sentence,"language":language}))?,
        evidence_hash: hex::encode(Sha256::digest(encoded)),
        observation,
        continuity: Continuity {
            status: if compatible {
                "compared"
            } else if previous.is_some() {
                "reset-incompatible"
            } else {
                "fresh"
            }
            .into(),
            reasons,
            source_distance,
            structure_distance,
            partition_changed,
            source_mixture,
            structure_mixture,
        },
        previous_state_hash: previous.map(|p| p.state_hash.clone()),
        state_hash: String::new(),
    };
    state.state_hash = state_digest(&state)?;
    Ok(state)
}

fn observe(binding: &Value, sentence: &str, scale: f64, limit: usize) -> Result<Observation> {
    let native_readings = array(binding, "readings", 64)?;
    if native_readings.is_empty() {
        return invalid("no retained readings");
    }
    let packet_atoms = array(&binding["source_packet"], "atoms", 64)?;
    let reference_points = array(&binding["source_packet"], "reference_points", 4096)?;
    let mut inventory = BTreeMap::new();
    for atom in packet_atoms {
        let id = text(&atom["atom"], "id", 100)?;
        if inventory.insert(id, atom).is_some() {
            return invalid("duplicate inventory atom");
        }
    }
    let mut source = BTreeMap::new();
    let mut structure = BTreeMap::new();
    let mut readings = Vec::new();
    let mut partitions = Vec::new();
    let mut reading_ids = BTreeSet::new();
    let mut atom_count = 0;
    let mut event_count = 0;
    let mut alternative_count = 0;
    let mut mapped = 0.0;
    for reading in native_readings {
        let rid = text(reading, "id", 100)?;
        if !reading_ids.insert(rid) {
            return invalid("duplicate reading ID");
        }
        if reading["frame"]["id"] != rid {
            return invalid("reading frame identity mismatch");
        }
        let atoms = array(&reading["bound_usage"], "atoms", 64)?;
        if atoms.len() != packet_atoms.len() || atoms.is_empty() {
            return invalid("retained atoms differ from source inventory");
        }
        let native_geometry = array(reading, "e8_activations", 4096)?;
        let mut atom_observations = Vec::new();
        let mut identities = BTreeMap::new();
        for atom in atoms {
            let native_atom = &atom["atom"];
            let aid = text(native_atom, "id", 100)?;
            let surface = occurrence_text(native_atom, "surface")?;
            let span = span(&native_atom["span"], sentence, surface)?;
            let packet = inventory
                .get(aid)
                .ok_or_else(|| CdissError::Invalid("atom absent from supplied inventory".into()))?;
            if packet["atom"]["surface"] != surface || packet["atom"]["span"] != native_atom["span"]
            {
                return invalid("atom/source occurrence mismatch");
            }
            let candidates = array(packet, "candidates", 512)?;
            let selected = array(atom, "selected_source_bindings", 512)?;
            let mut keys = BTreeSet::new();
            let mut fitted = BTreeSet::new();
            let mut selected_senses = BTreeSet::new();
            for candidate in selected {
                if !candidates.contains(candidate) {
                    return invalid("selected source binding was not in exact atom inventory");
                }
                let sid = text(&candidate["sense"], "id", 200)?;
                if !selected_senses.insert(sid) {
                    return invalid("duplicate selected source sense");
                }
                let cids = string_array(candidate, "concept_ids", 512)?;
                if cids.is_empty() {
                    keys.insert(format!("source:{sid}:unmapped-concept"));
                }
                for cid in cids {
                    let key = format!("source:{sid}:{cid}");
                    keys.insert(key.clone());
                    for activation in native_geometry.iter().filter(|a| {
                        a["atom_id"] == aid && a["sense_id"] == sid && a["concept_id"] == cid
                    }) {
                        if !activation["placement"].is_null() {
                            validate_geometry(activation, scale)?;
                            let point = reference_points
                                .iter()
                                .find(|p| p["concept_id"] == cid)
                                .ok_or_else(|| {
                                    CdissError::Invalid(
                                        "selected point absent from native source reference".into(),
                                    )
                                })?;
                            for field in ["position8", "root_id", "residual8"] {
                                if point[field] != activation["placement"][field] {
                                    return invalid(
                                        "activation does not match native reference point",
                                    );
                                }
                            }
                            fitted.insert(key.clone());
                        }
                    }
                }
            }
            let centers = array(&atom["sense_snap"], "centers", 512)?;
            let mut contextual = Vec::new();
            for center in centers.iter().filter(|c| c["origin"] == "context-pin") {
                if center["source_mapping_asserted"] != false {
                    return invalid("context pin asserted dictionary authority");
                }
                let mid = text(center, "meeting_id", 200)?;
                let key = format!("context:{mid}");
                keys.insert(key.clone());
                if !center["display"].is_null() {
                    validate_geometry(&center["display"], scale)?;
                    fitted.insert(key);
                }
                contextual.push(center.clone());
            }
            if keys.is_empty() {
                keys.insert(format!("unmapped:{surface}"));
            }
            let keys: Vec<_> = keys.into_iter().collect();
            let mass = 1.0 / keys.len() as f64;
            for key in &keys {
                *source.entry(key.clone()).or_insert(0.0) += mass;
                if fitted.contains(key) {
                    mapped += mass;
                }
            }
            atom_count += 1;
            alternative_count += keys.len();
            let mut all_candidate_sense_ids = Vec::new();
            for c in candidates {
                all_candidate_sense_ids.push(text(&c["sense"], "id", 200)?.into());
            }
            all_candidate_sense_ids.sort();
            all_candidate_sense_ids.dedup();
            if identities.insert(aid.to_owned(), keys.clone()).is_some() {
                return invalid("duplicate retained atom ID");
            }
            atom_observations.push(AtomObservation {
                atom_id: aid.into(),
                surface: surface.into(),
                span,
                selected_identities: keys.clone(),
                all_candidate_sense_ids,
                context_centers: contextual,
                alternative_count: keys.len(),
                geometrically_unmapped: fitted.is_empty(),
            });
        }
        // Ensure every activation refers to a selected source identity, including unavailable geometry.
        for activation in native_geometry {
            let aid = text(activation, "atom_id", 100)?;
            let sid = text(activation, "sense_id", 200)?;
            let cid = text(activation, "concept_id", 200)?;
            if !identities
                .get(aid)
                .is_some_and(|v| v.contains(&format!("source:{sid}:{cid}")))
            {
                return invalid("orphan geometric activation");
            }
        }
        let frame = &reading["frame"];
        let events = array(frame, "events", 64)?;
        let mut event_keys = BTreeMap::new();
        let mut reading_structure = Vec::new();
        let speech = text(frame, "speech_act", 32)?;
        add(&mut structure, format!("speech-act:{speech}"))?;
        reading_structure.push(format!("speech-act:{speech}"));
        for event in events {
            let eid = text(event, "id", 100)?;
            let predicate = identities
                .get(text(event, "predicate", 100)?)
                .ok_or_else(|| CdissError::Invalid("event predicate absent".into()))?;
            let mut roles = Vec::new();
            for role in array(event, "roles", 64)? {
                let values = identities
                    .get(text(role, "atom_id", 100)?)
                    .ok_or_else(|| CdissError::Invalid("role atom absent".into()))?;
                roles.push(json!({"role":text(role,"role",32)?,"identities":values}));
            }
            roles.sort_by_key(|v| v.to_string());
            let mut cues = Vec::new();
            for cue in array(event, "cue_spans", 64)? {
                let surface = occurrence_text(cue, "surface")?;
                span(&cue["span"], sentence, surface)?;
                cues.push(json!({"kind":text(cue,"kind",32)?,"surface":surface}));
            }
            cues.sort_by_key(|v| v.to_string());
            let key = format!(
                "event:{}",
                json!({"predicate":predicate,"roles":roles,"polarity":text(event,"polarity",32)?,"modality":text(event,"modality",32)?,"cues":cues})
            );
            if event_keys.insert(eid.to_owned(), key.clone()).is_some() {
                return invalid("duplicate event identity");
            }
            add(&mut structure, key.clone())?;
            reading_structure.push(key);
            event_count += 1;
        }
        for link in array(frame, "event_links", 64)? {
            let left = event_keys
                .get(text(link, "source", 100)?)
                .ok_or_else(|| CdissError::Invalid("event link source absent".into()))?;
            let right = event_keys
                .get(text(link, "target", 100)?)
                .ok_or_else(|| CdissError::Invalid("event link target absent".into()))?;
            let key = format!(
                "link:{}",
                json!({"source":left,"target":right,"type":text(link,"type",32)?})
            );
            add(&mut structure, key.clone())?;
            reading_structure.push(key);
        }
        for reference in array(frame, "references", 64)? {
            let left = identities
                .get(text(reference, "source", 100)?)
                .ok_or_else(|| CdissError::Invalid("reference source absent".into()))?;
            let right = identities
                .get(text(reference, "target", 100)?)
                .ok_or_else(|| CdissError::Invalid("reference target absent".into()))?;
            let key = format!("reference:{}", json!({"source":left,"target":right}));
            add(&mut structure, key.clone())?;
            reading_structure.push(key);
        }
        let mut reading_atoms: Vec<_> = identities.into_values().collect();
        reading_atoms.sort();
        reading_structure.sort();
        partitions.push(json!({"atoms":reading_atoms,"structure":reading_structure}));
        let uncertainty = string_array(reading, "selection_uncertainty", 64)?;
        readings.push(ReadingObservation {
            reading_id: rid.into(),
            frame: frame.clone(),
            atoms: atom_observations,
            geometry: native_geometry.clone(),
            uncertainty,
        });
    }
    partitions.sort_by_key(|v| v.to_string());
    let mapped_mass = mapped / atom_count as f64;
    Ok(Observation { reading_count:readings.len(), atom_count,event_count,alternative_count,mapped_mass,unmapped_mass:1.0-mapped_mass,
        allocation:"one unit per retained atom occurrence; equal alternative allocation; not probability, confidence, authority or completion".into(),
        source_vector:normalize(source,limit)?,structure_vector:normalize(structure,limit)?,partition_hash:digest(&json!(partitions))?,readings })
}
fn add(map: &mut BTreeMap<String, f64>, key: String) -> Result<()> {
    if key.len() > 65536
        || (!map.contains_key(&key)
            && (map.len() >= 8192
                || map.keys().map(String::len).sum::<usize>() + key.len() > 4 * 1024 * 1024))
    {
        return invalid(
            "structural feature byte budget exceeded; split passage without dropping alternatives",
        );
    }
    *map.entry(key).or_insert(0.0) += 1.0;
    Ok(())
}
fn text<'a>(value: &'a Value, key: &str, max: usize) -> Result<&'a str> {
    match value[key].as_str() {
        Some(s) if !s.is_empty() && s.len() <= max => Ok(s),
        _ => invalid(format!("missing or unbounded text: {key}")),
    }
}
fn occurrence_text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    // Match the native host's Unicode scalar limit, not a smaller UTF-8 byte limit.
    let s = text(value, key, 48_000)?;
    if s.chars().count() > 12_000 {
        return invalid(format!(
            "occurrence text exceeds Unicode character limit: {key}"
        ));
    }
    Ok(s)
}
fn array<'a>(value: &'a Value, key: &str, max: usize) -> Result<&'a Vec<Value>> {
    match value[key].as_array() {
        Some(a) if a.len() <= max => Ok(a),
        _ => invalid(format!("missing or unbounded list: {key}")),
    }
}
fn string_array(value: &Value, key: &str, max: usize) -> Result<Vec<String>> {
    array(value, key, max)?
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| s.len() <= 4000)
                .map(str::to_owned)
                .ok_or_else(|| CdissError::Invalid("invalid string list".into()))
        })
        .collect()
}
fn require_hash(value: &Value) -> Result<&str> {
    match value.as_str() {
        Some(s)
            if s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) =>
        {
            Ok(s)
        }
        _ => invalid("missing canonical SHA256 pin"),
    }
}
fn span(value: &Value, sentence: &str, surface: &str) -> Result<[usize; 2]> {
    let a = value
        .as_array()
        .filter(|a| a.len() == 2)
        .ok_or_else(|| CdissError::Invalid("invalid occurrence span".into()))?;
    let start = a[0]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| CdissError::Invalid("invalid span start".into()))?;
    let end = a[1]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| CdissError::Invalid("invalid span end".into()))?;
    let chars: Vec<_> = sentence.chars().collect();
    if start >= end || end > chars.len() || chars[start..end].iter().collect::<String>() != surface
    {
        return invalid("span does not match exact Unicode occurrence");
    }
    Ok([start, end])
}
fn vec8(value: &Value) -> Result<[f64; 8]> {
    let values = value
        .as_array()
        .filter(|v| v.len() == 8)
        .ok_or_else(|| CdissError::Invalid("eight finite coordinates required".into()))?;
    let mut out = [0.0; 8];
    for (i, v) in values.iter().enumerate() {
        out[i] = v
            .as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| CdissError::Invalid("nonfinite coordinate".into()))?;
    }
    Ok(out)
}
/// Stable norm preserves finite large and subnormal directions; unrepresentable radius rejects.
pub fn stable_norm(vector: &[f64; 8]) -> Result<f64> {
    if vector.iter().any(|x| !x.is_finite()) {
        return invalid("nonfinite norm input");
    }
    let scale = vector.iter().map(|x| x.abs()).fold(0.0, f64::max);
    if scale == 0.0 {
        return Ok(0.0);
    }
    let norm = scale
        * vector
            .iter()
            .map(|x| (x / scale).powi(2))
            .sum::<f64>()
            .sqrt();
    if !norm.is_finite() {
        return invalid("radius exceeds Float64; rescale explicitly");
    }
    Ok(norm)
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}
fn validate_geometry(activation: &Value, scale: f64) -> Result<()> {
    let p = &activation["placement"];
    let position = vec8(&p["position8"])?;
    let anchor = vec8(&p["scaled_anchor8"])?;
    let residual = vec8(&p["residual8"])?;
    let radius = p["radius"]
        .as_f64()
        .filter(|r| r.is_finite() && *r >= 0.0)
        .ok_or_else(|| CdissError::Invalid("invalid native radius".into()))?;
    if !close(stable_norm(&position)?, radius) {
        return invalid("native radius/direction mismatch");
    }
    for i in 0..8 {
        if !close(position[i], anchor[i] + residual[i]) {
            return invalid("native root residual reconstruction failed");
        }
    }
    let root_index = p["root_index"]
        .as_u64()
        .filter(|n| *n < 240)
        .ok_or_else(|| CdissError::Invalid("native root index missing".into()))?;
    if p["root_id"] != format!("e8-root:{}", root_index + 1) {
        return invalid("native root ID/index mismatch");
    }
    let roots = canonical_roots();
    let unit = roots[root_index as usize].map(|x| x as f64 / (2.0 * 2f64.sqrt()));
    for i in 0..8 {
        if !close(anchor[i], radius * unit[i]) {
            return invalid("root anchor does not match pinned lexicographic E8 convention");
        }
    }
    let address = &activation["lattice_address"];
    let lattice = vec8(&address["position"])?;
    let remainder = vec8(&address["residual"])?;
    if address["geometry_version"] != GEOMETRY_VERSION
        || address["fine_position_scale"].as_f64() != Some(scale)
    {
        return invalid("native lattice convention mismatch");
    }
    if address["semantic_equivalence_asserted"] != false {
        return invalid("lattice address asserted semantic equivalence");
    }
    let doubled = array(address, "doubled_standard", 8)?;
    if doubled.len() != 8 {
        return invalid("eight doubled lattice coordinates required");
    }
    let mut integers = [0i64; 8];
    for (i, v) in doubled.iter().enumerate() {
        integers[i] = v
            .as_i64()
            .ok_or_else(|| CdissError::Invalid("noninteger lattice coordinate".into()))?;
        if integers[i].unsigned_abs() > 2_000_000_000_000
            || !close(lattice[i], integers[i] as f64 / 2.0)
        {
            return invalid("lattice coordinate/address mismatch");
        }
    }
    let parity = integers[0].rem_euclid(2);
    if integers.iter().any(|x| x.rem_euclid(2) != parity)
        || integers.iter().sum::<i64>().rem_euclid(4) != 0
    {
        return invalid("address is outside the E8 lattice");
    }
    for i in 0..8 {
        if !close(position[i], (lattice[i] + remainder[i]) / scale) {
            return invalid("native lattice residual reconstruction failed");
        }
    }
    Ok(())
}
fn canonical_roots() -> Vec<[i8; 8]> {
    let mut roots = Vec::with_capacity(240);
    for i in 0..8 {
        for j in i + 1..8 {
            for left in [-2, 2] {
                for right in [-2, 2] {
                    let mut r = [0; 8];
                    r[i] = left;
                    r[j] = right;
                    roots.push(r);
                }
            }
        }
    }
    for bits in 0u32..256 {
        if bits.count_ones() % 2 == 0 {
            let mut r = [1; 8];
            for (i, v) in r.iter_mut().enumerate() {
                if bits & (1 << i) != 0 {
                    *v = -1;
                }
            }
            roots.push(r);
        }
    }
    roots.sort();
    roots
}
fn normalize(map: BTreeMap<String, f64>, limit: usize) -> Result<Vec<Feature>> {
    let map: BTreeMap<_, _> = map.into_iter().filter(|(_, mass)| *mass > 0.0).collect();
    if map.is_empty()
        || map.len() > limit
        || map.keys().map(String::len).sum::<usize>() > 4 * 1024 * 1024
    {
        return invalid("sparse feature budget exceeded or empty distribution; nothing truncated");
    }
    let sum: f64 = map.values().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return invalid("invalid feature mass");
    }
    Ok(map
        .into_iter()
        .filter(|(_, m)| *m > 0.0)
        .map(|(key, mass)| Feature {
            key,
            mass: mass / sum,
        })
        .collect())
}
fn valid_vector(v: &[Feature], limit: usize) -> bool {
    !v.is_empty()
        && v.len() <= limit
        && v.iter()
            .all(|f| f.mass.is_finite() && f.mass > 0.0 && f.key.len() <= 65536)
        && v.windows(2).all(|a| a[0].key < a[1].key)
        && ((v.iter().map(|f| f.mass).sum::<f64>()) - 1.0).abs() < 1e-10
}
fn mix(
    previous: &[Feature],
    current: &[Feature],
    retention: f64,
    limit: usize,
) -> Result<Vec<Feature>> {
    let mut out = BTreeMap::new();
    for f in previous {
        *out.entry(f.key.clone()).or_insert(0.0) += retention * f.mass;
    }
    for f in current {
        *out.entry(f.key.clone()).or_insert(0.0) += (1.0 - retention) * f.mass;
    }
    normalize(out, limit)
}
/// TV and square-root JS(base 2), on the union of explicit identities. No geometry cost.
pub fn distance(left: &[Feature], right: &[Feature]) -> Result<Distances> {
    if !valid_vector(left, 8192) || !valid_vector(right, 8192) {
        return invalid("distance requires normalized finite identity vectors");
    }
    let mut pairs: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for f in left {
        pairs.entry(&f.key).or_default().0 = f.mass;
    }
    for f in right {
        pairs.entry(&f.key).or_default().1 = f.mass;
    }
    let mut tv = 0.0;
    let mut js = 0.0;
    for (p, q) in pairs.values().copied() {
        tv += (p - q).abs();
        if p > 0.0 {
            js += p * (2.0 * (p / (p + q))).log2();
        }
        if q > 0.0 {
            js += q * (2.0 * (q / (p + q))).log2();
        }
    }
    Ok(Distances {
        total_variation: (tv / 2.0).clamp(0.0, 1.0),
        jensen_shannon_distance: (js / 2.0).max(0.0).sqrt().min(1.0),
    })
}
fn digest(value: &Value) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}
fn state_digest(state: &State) -> Result<String> {
    let mut value = serde_json::to_value(state)?;
    value
        .as_object_mut()
        .expect("serialized state is object")
        .remove("stateHash");
    digest(&value)
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    #[test]
    fn occurrence_limits_match_unicode_host_characters() {
        let input = json!({"sentence":"界".repeat(12_000)});
        let sentence = occurrence_text(&input, "sentence").unwrap();
        assert_eq!(
            span(&json!([0, 12_000]), sentence, sentence).unwrap(),
            [0, 12_000]
        );
        assert!(occurrence_text(&json!({"sentence":"界".repeat(12_001)}), "sentence").is_err());
    }
    #[test]
    fn subnormal_js_mass_cannot_turn_into_infinite_ratio() {
        let a = [
            Feature {
                key: "a".into(),
                mass: f64::from_bits(1),
            },
            Feature {
                key: "b".into(),
                mass: 1.0,
            },
        ];
        let b = [Feature {
            key: "b".into(),
            mass: 1.0,
        }];
        let d = distance(&a, &b).unwrap();
        assert!(d.jensen_shannon_distance.is_finite() && d.jensen_shannon_distance < 1e-150);
        let min = f64::from_bits(1);
        let left = [
            Feature {
                key: "a".into(),
                mass: 1.0,
            },
            Feature {
                key: "b".into(),
                mass: min,
            },
        ];
        let right = [
            Feature {
                key: "a".into(),
                mass: 1.0,
            },
            Feature {
                key: "c".into(),
                mass: min,
            },
        ];
        let both = distance(&left, &right).unwrap();
        assert_eq!(both.total_variation, min);
        assert_eq!(both.jensen_shannon_distance, min.sqrt());
    }
    #[test]
    fn retention_endpoints_do_not_charge_zero_mass_support_to_feature_budget() {
        let vector = |prefix: &str| {
            (0..16)
                .map(|n| Feature {
                    key: format!("{prefix}{n:02}"),
                    mass: 1.0 / 16.0,
                })
                .collect::<Vec<_>>()
        };
        let old = vector("a");
        let current = vector("b");
        let fresh = mix(&old, &current, 0.0, 16).unwrap();
        let retained = mix(&old, &current, 1.0, 16).unwrap();
        assert_eq!(distance(&fresh, &current).unwrap().total_variation, 0.0);
        assert_eq!(distance(&retained, &old).unwrap().total_variation, 0.0);
        assert!(mix(&old, &current, 0.5, 16).is_err());
    }
}

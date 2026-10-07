//! Pure complete-chain comparisons. Native consistency is not linguistic correctness.
use crate::{analyze_joe_result, distance, stable_norm, Config, Feature, State};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, String>;
const OUTPUT_BYTES: usize = 32 * 1024 * 1024;
fn encoded_len(value: &impl Serialize) -> Result<usize> {
    Ok(serde_json::to_vec(value).map_err(|e| e.to_string())?.len())
}
fn charge(total: &mut usize, value: &impl Serialize) -> Result<()> {
    charge_bytes(total, encoded_len(value)?)
}
fn charge_bytes(total: &mut usize, amount: usize) -> Result<()> {
    *total = total.checked_add(amount).ok_or("output byte overflow")?;
    if *total > OUTPUT_BYTES {
        return Err(
            "complete evidence output budget exceeded; split without dropping records".into(),
        );
    }
    Ok(())
}
fn max_bytes<'a>(values: impl Iterator<Item = &'a Value>) -> Result<usize> {
    values.map(encoded_len).try_fold(0, |a, b| Ok(a.max(b?)))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetrievalLimits {
    pub max_aggregate_bytes: usize,
    pub max_candidates: usize,
    pub max_reading_pairs: usize,
    pub max_record_pairs: usize,
    pub max_postings: usize,
}
impl Default for RetrievalLimits {
    fn default() -> Self {
        Self {
            max_aggregate_bytes: 32 * 1024 * 1024,
            max_candidates: 64,
            max_reading_pairs: 4096,
            max_record_pairs: 65536,
            max_postings: 65536,
        }
    }
}
impl RetrievalLimits {
    pub fn validate(&self) -> Result<()> {
        if !(1024..=32 * 1024 * 1024).contains(&self.max_aggregate_bytes)
            || !(1..=64).contains(&self.max_candidates)
            || !(1..=4096).contains(&self.max_reading_pairs)
            || !(1..=65536).contains(&self.max_record_pairs)
            || !(1..=65536).contains(&self.max_postings)
        {
            return Err("invalid retrieval budget".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
struct Atom {
    id: String,
    surface: String,
    span: Value,
    selected: Vec<Value>,
    alternatives: Vec<Value>,
    sense_snap: Value,
    sense_ids: BTreeSet<String>,
    asserted_concepts: BTreeSet<String>,
    candidate_concepts: BTreeSet<String>,
}
#[derive(Clone, Debug)]
struct Reading {
    id: String,
    atoms: Vec<Atom>,
    frame: Value,
    geometry: Vec<Value>,
    uncertainty: Value,
}
/// Private fields: import only original native packets through the existing validator.
/// Host verification of original source files/citations remains required.
#[derive(Clone, Debug)]
pub struct VerifiedRetrievalInput {
    state: State,
    native: Value,
    readings: Vec<Reading>,
    bytes: usize,
    audit_bytes: usize,
}
impl VerifiedRetrievalInput {
    pub fn from_native(native: &Value) -> Result<Self> {
        let state =
            analyze_joe_result(native, None, &Config::default()).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(native).map_err(|e| e.to_string())?.len();
        let binding = &native["interpretation"]["binding"];
        let inventory = list(&binding["source_packet"]["atoms"])?;
        let mut audit_bytes = 4096 + encoded_len(&state.basis)? + encoded_len(&native["sentence"])?;
        // Preflight repeated alternatives before cloning source inventories for
        // each retained reading; the profile never silently removes a branch.
        for raw in list(&binding["readings"])? {
            charge(&mut audit_bytes, &raw["frame"])?;
            charge(&mut audit_bytes, &raw["e8_activations"])?;
            for bound in list(&raw["bound_usage"]["atoms"])? {
                let supplied = inventory
                    .iter()
                    .find(|a| a["atom"]["id"] == bound["atom"]["id"])
                    .ok_or("source atom missing")?;
                charge(&mut audit_bytes, &bound["atom"])?;
                charge_bytes(
                    &mut audit_bytes,
                    3 * encoded_len(&bound["selected_source_bindings"])? + 1024,
                )?;
                charge(&mut audit_bytes, &supplied["candidates"])?;
                charge(&mut audit_bytes, &bound["sense_snap"])?;
            }
        }
        let mut readings = Vec::new();
        for raw in list(&binding["readings"])? {
            let mut atoms = Vec::new();
            for bound in list(&raw["bound_usage"]["atoms"])? {
                let id = string(&bound["atom"]["id"])?;
                let supplied = inventory
                    .iter()
                    .find(|a| a["atom"]["id"] == id)
                    .ok_or("retained occurrence has no source inventory")?;
                let selected = list(&bound["selected_source_bindings"])?.clone();
                let alternatives = list(&supplied["candidates"])?.clone();
                let mut sense_ids = BTreeSet::new();
                let mut asserted_concepts = BTreeSet::new();
                let mut candidate_concepts = BTreeSet::new();
                for candidate in &selected {
                    let sid = string(&candidate["sense"]["id"])?;
                    sense_ids.insert(sid.clone());
                    let cids = strings(&candidate["concept_ids"])?;
                    for alignment in list(&candidate["alignments"])? {
                        if alignment["sense_id"] != sid {
                            return Err("alignment sense/selected binding mismatch".into());
                        }
                        if let Some(cid) = alignment["concept_id"].as_str() {
                            if !cids.contains(cid) {
                                return Err("alignment concept absent from selected binding".into());
                            }
                            if alignment["kind"] == "equivalent" && alignment["asserted"] == true {
                                asserted_concepts.insert(cid.to_owned());
                            } else {
                                candidate_concepts.insert(cid.to_owned());
                            }
                        }
                    }
                }
                let sense_snap = bound["sense_snap"].clone();
                for center in list(&sense_snap["centers"])? {
                    match center["origin"].as_str() {
                        Some("dictionary-source") => {
                            let center_senses = strings(&center["source_sense_ids"])?;
                            if center["source_mapping_asserted"] != true
                                || center_senses.is_empty()
                                || !center_senses.is_subset(&sense_ids)
                            {
                                return Err("dictionary SenseSnap center not joined to selected source senses".into());
                            }
                            let cid = string(&center["semantic_concept_id"])?;
                            if center["meeting_id"] != cid {
                                return Err(
                                    "dictionary center meeting/concept identity mismatch".into()
                                );
                            }
                            let mut proof_ids = BTreeSet::new();
                            for candidate in &selected {
                                if center_senses.contains(&string(&candidate["sense"]["id"])?) {
                                    let before = proof_ids.len();
                                    for alignment in list(&candidate["alignments"])? {
                                        if alignment["concept_id"] == cid
                                            && alignment["asserted"] == true
                                            && (alignment["kind"] == "equivalent"
                                                || alignment["kind"] == "language_specific")
                                        {
                                            proof_ids.insert(string(&alignment["id"])?);
                                        }
                                    }
                                    if proof_ids.len() == before {
                                        return Err("listed dictionary center sense has no supported alignment to its concept".into());
                                    }
                                }
                            }
                            if proof_ids.is_empty()
                                || strings(&center["alignment_ids"])? != proof_ids
                            {
                                return Err("dictionary SenseSnap center has no exact supported source alignment proof".into());
                            }
                        }
                        Some("context-pin") if center["source_mapping_asserted"] == false => (),
                        _ => {
                            return Err("unsupported or authority-claiming SenseSnap center".into())
                        }
                    }
                    center_geometry_supported(native, center)?;
                }
                atoms.push(Atom {
                    id,
                    surface: string(&bound["atom"]["surface"])?,
                    span: bound["atom"]["span"].clone(),
                    selected,
                    alternatives,
                    sense_snap,
                    sense_ids,
                    asserted_concepts,
                    candidate_concepts,
                });
            }
            readings.push(Reading {
                id: string(&raw["id"])?,
                atoms,
                frame: raw["frame"].clone(),
                geometry: list(&raw["e8_activations"])?.clone(),
                uncertainty: raw["selection_uncertainty"].clone(),
            });
        }
        Ok(Self {
            state,
            native: native.clone(),
            readings,
            bytes,
            audit_bytes,
        })
    }
    pub fn native_packet(&self) -> &Value {
        &self.native
    }
    pub fn audit_size_upper_bound(&self) -> usize {
        self.audit_bytes
    }
    pub fn audit_profile(&self) -> Value {
        json!({"schema":"bomb-code/structured-retrieval-profile/v1", "requestId":self.state.request_id,
            "stateHash":self.state.state_hash,"evidenceHash":self.state.evidence_hash,"basis":self.state.basis,
            "sentence":self.native["sentence"],"allocation":self.state.observation.allocation,
            "readings":self.readings.iter().map(|r| json!({"readingId":r.id,"frame":r.frame,
                "uncertainty":r.uncertainty,"geometry":r.geometry,"occurrences":r.atoms.iter().map(|a| json!({
                    "atomId":a.id,"surface":a.surface,"span":a.span,"selectedSourceBindings":a.selected,
                    "allSourceAlternatives":a.alternatives,"assertedConceptIds":a.asserted_concepts,
                    "candidateConceptIds":a.candidate_concepts,"senseSnap":a.sense_snap})).collect::<Vec<_>>() })).collect::<Vec<_>>(),
            "qualification":"Consistent source-bound proposal; source files require host verification; interpretation correctness unvalidated"})
    }
}
fn list(value: &Value) -> Result<&Vec<Value>> {
    value
        .as_array()
        .ok_or_else(|| "required array missing".into())
}
fn string(value: &Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "required text missing".into())
}
fn strings(value: &Value) -> Result<BTreeSet<String>> {
    list(value)?.iter().map(string).collect()
}
fn overlap(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.intersection(right).cloned().collect()
}
fn distribution(keys: impl IntoIterator<Item = Vec<String>>) -> Vec<Feature> {
    let mut mass = BTreeMap::<String, f64>::new();
    for values in keys {
        if !values.is_empty() {
            let unit = 1.0 / values.len() as f64;
            for key in values {
                *mass.entry(key).or_default() += unit;
            }
        }
    }
    let total: f64 = mass.values().sum();
    mass.into_iter()
        .map(|(key, mass)| Feature {
            key,
            mass: mass / total,
        })
        .collect()
}
fn measured(left: &[Feature], right: &[Feature]) -> Result<Value> {
    if left.is_empty() || right.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::to_value(distance(left, right).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn compatible(query: &VerifiedRetrievalInput, candidate: &VerifiedRetrievalInput) -> Result<()> {
    let q = &query.state.basis;
    let c = &candidate.state.basis;
    if q.manifest_sha256 != c.manifest_sha256
        || q.source_graph_hash != c.source_graph_hash
        || q.model_snapshot_hash != c.model_snapshot_hash
        || q.model_id != c.model_id
        || q.geometry_version != c.geometry_version
        || q.reference_digest != c.reference_digest
    {
        return Err(
            "structured retrieval source/model/geometry/SenseSnap snapshot incompatible".into(),
        );
    }
    Ok(())
}
fn scopes(query: &VerifiedRetrievalInput, candidate: &VerifiedRetrievalInput) -> Value {
    let q = &query.state.basis;
    let c = &candidate.state.basis;
    json!({"threadEqual":q.thread_id==c.thread_id,"contextHashEqual":q.context_hash==c.context_hash,
        "languageEqual":q.language==c.language,"providerEqual":q.provider==c.provider&&q.provider_model==c.provider_model,
        "meaning":"Different contexts/languages/providers remain separate scopes; hash equality is identity, not meaning"})
}
fn semantic_identity(atom: &Atom) -> Value {
    let unresolved: Vec<_> = atom
        .selected
        .iter()
        .filter(|s| {
            !s["alignments"].as_array().is_some_and(|rows| {
                rows.iter().any(|a| {
                    a["kind"] == "equivalent"
                        && a["asserted"] == true
                        && a["concept_id"].is_string()
                })
            })
        })
        .map(|s| s["sense"]["id"].clone())
        .collect();
    json!({"assertedConceptIds":atom.asserted_concepts,
        "unresolvedSourceSenseIds":unresolved,
        "unmappedSurface":if atom.sense_ids.is_empty(){Some(&atom.surface)}else{None},
        "evidenceStatus":if atom.asserted_concepts.is_empty(){"source-equivalence-unavailable"}else{"asserted-source-concept"}})
}
fn fully_asserted(atom: &Atom) -> bool {
    !atom.selected.is_empty()
        && atom.selected.iter().all(|s| {
            s["alignments"].as_array().is_some_and(|rows| {
                rows.iter().any(|a| {
                    a["kind"] == "equivalent"
                        && a["asserted"] == true
                        && a["concept_id"].is_string()
                })
            })
        })
}
fn atom<'a>(reading: &'a Reading, id: &Value) -> Result<&'a Atom> {
    reading
        .atoms
        .iter()
        .find(|a| id.as_str() == Some(a.id.as_str()))
        .ok_or_else(|| "frame occurrence absent".into())
}
fn events(reading: &Reading) -> Result<Vec<Value>> {
    let mut output = Vec::new();
    for event in list(&reading.frame["events"])? {
        let predicate = atom(reading, &event["predicate"])?;
        let mut roles = Vec::new();
        let mut all_asserted = fully_asserted(predicate);
        for role in list(&event["roles"])? {
            let participant = atom(reading, &role["atom_id"])?;
            all_asserted &= fully_asserted(participant);
            roles.push(json!({"role":role["role"],"identity":semantic_identity(participant)}));
        }
        roles.sort_by_key(Value::to_string);
        let canonical = json!({"predicate":semantic_identity(predicate),"roles":roles,
            "polarity":event["polarity"],"modality":event["modality"]});
        output.push(
            json!({"eventId":event["id"],"canonical":canonical,"sourceEvent":event,
            "fullyAssertedParticipants":all_asserted,"predicateAtomId":predicate.id}),
        );
    }
    Ok(output)
}
fn graph(reading: &Reading, event_records: &[Value]) -> Result<Value> {
    let mut links = Vec::new();
    for link in list(&reading.frame["event_links"])? {
        let source = event_records
            .iter()
            .find(|e| e["eventId"] == link["source"])
            .ok_or("event link source missing")?;
        let target = event_records
            .iter()
            .find(|e| e["eventId"] == link["target"])
            .ok_or("event link target missing")?;
        links.push(
            json!({"source":source["canonical"],"target":target["canonical"],"type":link["type"]}),
        );
    }
    let mut references = Vec::new();
    for reference in list(&reading.frame["references"])? {
        references.push(
            json!({"source":semantic_identity(atom(reading,&reference["source"])?),
            "target":semantic_identity(atom(reading,&reference["target"])?)}),
        );
    }
    links.sort_by_key(Value::to_string);
    references.sort_by_key(Value::to_string);
    Ok(json!({"links":links,"references":references}))
}
fn context_keys(
    profile: &VerifiedRetrievalInput,
    reading: &Reading,
    origin: &str,
) -> Result<Vec<Vec<String>>> {
    let mut output = Vec::new();
    for atom in &reading.atoms {
        let mut values = Vec::new();
        for center in list(&atom.sense_snap["centers"])? {
            if center["origin"] == origin {
                if origin == "dictionary-source" {
                    values.push(center["semantic_concept_id"].to_string());
                } else if profile.state.basis.thread_id.is_some()
                    && center["pin_evidence"]
                        .as_array()
                        .is_some_and(|e| !e.is_empty())
                {
                    values.push(json!({"threadId":profile.state.basis.thread_id,"meetingId":center["meeting_id"],
                        "pinIds":center["pin_ids"],"evidence":center["pin_evidence"]}).to_string());
                }
            }
        }
        output.push(values);
    }
    Ok(output)
}
fn vector(value: &Value) -> Result<[f64; 8]> {
    let raw = list(value)?;
    if raw.len() != 8 {
        return Err("geometry dimension missing".into());
    }
    let mut result = [0.0; 8];
    for (i, v) in raw.iter().enumerate() {
        result[i] = v
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or("nonfinite geometry")?;
    }
    Ok(result)
}
fn norm_delta(q: &Value, c: &Value) -> Result<f64> {
    let q = vector(q)?;
    let c = vector(c)?;
    let mut delta = [0.0; 8];
    for i in 0..8 {
        delta[i] = q[i] - c[i];
    }
    stable_norm(&delta).map_err(|e| e.to_string())
}
fn center_count(reading: &Reading) -> Result<usize> {
    reading
        .atoms
        .iter()
        .try_fold(0usize, |n, a| Ok(n + list(&a.sense_snap["centers"])?.len()))
}
fn center_geometry_supported(native: &Value, center: &Value) -> Result<bool> {
    if !center["display"]["placement"].is_object() {
        return Ok(false);
    }
    let Some(cid) = center["semantic_concept_id"].as_str() else {
        return Ok(false);
    };
    let binding = &native["interpretation"]["binding"];
    let point = list(&binding["source_packet"]["reference_points"])?
        .iter()
        .find(|p| p["concept_id"] == cid);
    let supported = point.is_some_and(|p| {
        ["position8", "root_id", "residual8"]
            .iter()
            .all(|key| p[*key] == center["display"]["placement"][*key])
    });
    if !supported {
        if center["origin"] == "dictionary-source" {
            return Err("dictionary center geometry differs from native source reference".into());
        }
        return Ok(false);
    }
    let scale = binding["request"]["shared_reference"]["lattice_scale"]
        .as_f64()
        .ok_or("native lattice scale missing")?;
    crate::validate_geometry(&center["display"], scale).map_err(|e| e.to_string())?;
    Ok(true)
}
fn center_comparisons(
    query: &VerifiedRetrievalInput,
    q: &Reading,
    candidate: &VerifiedRetrievalInput,
    c: &Reading,
) -> Result<Vec<Value>> {
    let mut output = Vec::new();
    for qa in &q.atoms {
        for qc in list(&qa.sense_snap["centers"])? {
            for ca in &c.atoms {
                for cc in list(&ca.sense_snap["centers"])? {
                    let same_origin = qc["origin"] == cc["origin"];
                    let dictionary = same_origin && qc["origin"] == "dictionary-source";
                    let evidence = qc["pin_evidence"].as_array().is_some_and(|e| !e.is_empty())
                        && qc["pin_evidence"] == cc["pin_evidence"]
                        && qc["pin_ids"] == cc["pin_ids"];
                    let scoped = query.state.basis.thread_id.is_some()
                        && query.state.basis.thread_id == candidate.state.basis.thread_id;
                    let context =
                        same_origin && qc["origin"] == "context-pin" && evidence && scoped;
                    let exact = (dictionary || context) && qc["meeting_id"] == cc["meeting_id"];
                    let qp = &qc["display"]["placement"];
                    let cp = &cc["display"]["placement"];
                    let supported = center_geometry_supported(query.native_packet(), qc)?
                        && center_geometry_supported(candidate.native_packet(), cc)?;
                    let fine = if supported && qp.is_object() && cp.is_object() {
                        json!({
                            "finePositionL2":norm_delta(&qp["position8"],&cp["position8"])?,
                            "residualL2":norm_delta(&qp["residual8"],&cp["residual8"])?,
                            "sameRoot":qp["root_id"]==cp["root_id"],"semanticEquivalenceInferred":false
                        })
                    } else {
                        Value::Null
                    };
                    output.push(json!({"queryAtomId":qa.id,"candidateAtomId":ca.id,
                "queryCenter":qc,"candidateCenter":cc,"sameOrigin":same_origin,
                "scopeAndEvidenceCompatible":dictionary||context,"exactDeclaredCenterMatch":exact,
                        "fineGeometry":fine,"nativeGeometrySupported":supported,
                        "meaning":"Exact declared source/pin evidence and fine geometry are separate comparisons; unsupported geometry unavailable"}));
                }
            }
        }
    }
    Ok(output)
}
fn geometry(q: &Reading, c: &Reading) -> Result<Vec<Value>> {
    let mut output = Vec::new();
    for qa in &q.geometry {
        for ca in &c.geometry {
            let qp = &qa["placement"];
            let cp = &ca["placement"];
            let exact = qa["sense_id"] == ca["sense_id"] && qa["concept_id"] == ca["concept_id"];
            let concept = qa["concept_id"] == ca["concept_id"];
            let same_root = qp.is_object() && cp.is_object() && qp["root_id"] == cp["root_id"];
            if !exact && !concept && !same_root {
                continue;
            }
            let present = qp.is_object() && cp.is_object();
            let mut numeric = Value::Null;
            if present {
                let qv = vector(&qp["position8"])?;
                let cv = vector(&cp["position8"])?;
                let qn = stable_norm(&qv).map_err(|e| e.to_string())?;
                let cn = stable_norm(&cv).map_err(|e| e.to_string())?;
                let angle = if qn > 0.0 && cn > 0.0 {
                    Some(
                        qv.iter()
                            .zip(cv)
                            .map(|(a, b)| (a / qn) * (b / cn))
                            .sum::<f64>()
                            .clamp(-1.0, 1.0)
                            .acos(),
                    )
                } else {
                    None
                };
                numeric = json!({"finePositionL2":norm_delta(&qp["position8"],&cp["position8"])?,
                "residualL2":norm_delta(&qp["residual8"],&cp["residual8"])?,
                "directionAngleRadians":angle,"radiusDifference":(qn-cn).abs()});
            }
            output.push(json!({"queryAtomId":qa["atom_id"],"candidateAtomId":ca["atom_id"],
            "sameSourceIdentity":exact,"sameConceptId":concept,"sameRoot":same_root,"rootCollision":same_root&&!concept,
            "queryActivation":qa,"candidateActivation":ca,"distances":numeric,
            "status":if present{"native-fine-correspondence"}else{"geometry-unavailable"},"semanticEquivalenceInferred":false}));
        }
    }
    Ok(output)
}
fn count_keys(events: &[Value]) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::new();
    for e in events {
        *result.entry(e["canonical"].to_string()).or_default() += 1;
    }
    result
}
fn atom_collisions(reading: &Reading) -> bool {
    let mut seen = BTreeSet::new();
    reading
        .atoms
        .iter()
        .any(|a| !seen.insert(semantic_identity(a).to_string()))
}
fn compare_reading(
    query: &VerifiedRetrievalInput,
    q: &Reading,
    candidate: &VerifiedRetrievalInput,
    c: &Reading,
) -> Result<Value> {
    let source_q = distribution(
        q.atoms
            .iter()
            .map(|a| a.sense_ids.iter().cloned().collect()),
    );
    let source_c = distribution(
        c.atoms
            .iter()
            .map(|a| a.sense_ids.iter().cloned().collect()),
    );
    let concepts_q = distribution(
        q.atoms
            .iter()
            .map(|a| a.asserted_concepts.iter().cloned().collect()),
    );
    let concepts_c = distribution(
        c.atoms
            .iter()
            .map(|a| a.asserted_concepts.iter().cloned().collect()),
    );
    let qe = events(q)?;
    let ce = events(c)?;
    let mut occurrence_matches = Vec::new();
    for qa in &q.atoms {
        for ca in &c.atoms {
            let senses = overlap(&qa.sense_ids, &ca.sense_ids);
            let concepts = overlap(&qa.asserted_concepts, &ca.asserted_concepts);
            if !senses.is_empty() || !concepts.is_empty() {
                occurrence_matches.push(json!({"queryAtomId":qa.id,"candidateAtomId":ca.id,"querySpan":qa.span,"candidateSpan":ca.span,
                "sameSenseIds":senses,"assertedSharedConceptIds":concepts,"querySourceBindings":qa.selected,"candidateSourceBindings":ca.selected}));
            }
        }
    }
    let mut event_pairs = Vec::new();
    for qevent in &qe {
        for cevent in &ce {
            event_pairs.push(json!({"queryEventId":qevent["eventId"],"candidateEventId":cevent["eventId"],
            "predicateEqual":qevent["canonical"]["predicate"]==cevent["canonical"]["predicate"],
            "rolesEqual":qevent["canonical"]["roles"]==cevent["canonical"]["roles"],
            "polarityEqual":qevent["canonical"]["polarity"]==cevent["canonical"]["polarity"],
            "modalityEqual":qevent["canonical"]["modality"]==cevent["canonical"]["modality"],
            "contentEqual":qevent["canonical"]==cevent["canonical"],"query":qevent,"candidate":cevent}));
        }
    }
    let qgraph = graph(q, &qe)?;
    let cgraph = graph(c, &ce)?;
    let collisions =
        count_keys(&qe).values().any(|n| *n > 1) || count_keys(&ce).values().any(|n| *n > 1);
    let eq = distribution(qe.iter().map(|e| vec![e["canonical"].to_string()]));
    let ec = distribution(ce.iter().map(|e| vec![e["canonical"].to_string()]));
    let qpins = distribution(context_keys(query, q, "context-pin")?);
    let cpins = distribution(context_keys(candidate, c, "context-pin")?);
    let dq = distribution(context_keys(query, q, "dictionary-source")?);
    let dc = distribution(context_keys(candidate, c, "dictionary-source")?);
    Ok(json!({"queryReadingId":q.id,"candidateReadingId":c.id,
        "sourceSense":{"distance":measured(&source_q,&source_c)?,"queryDistribution":source_q,"candidateDistribution":source_c},
        "assertedConcept":{"distance":measured(&concepts_q,&concepts_c)?,"queryDistribution":concepts_q,"candidateDistribution":concepts_c},
        "senseSnap":{"dictionaryDistance":measured(&dq,&dc)?,
            "contextPinDistance":if query.state.basis.thread_id.is_some()&&query.state.basis.thread_id==candidate.state.basis.thread_id {
                measured(&qpins,&cpins)?}else{Value::Null},
            "centerComparisons":center_comparisons(query,q,candidate,c)?,
            "contextPolicy":"Exact declared pin provenance/evidence plus thread scope; whole context hash is not a meaning measure",
            "queryContextPinDistribution":qpins,"candidateContextPinDistribution":cpins},
        "eventLocal":{"conceptNormalizedDistance":measured(&eq,&ec)?,"eventPairs":event_pairs,
            "queryEvents":qe,"candidateEvents":ce,"queryCanonicalGraph":qgraph,"candidateCanonicalGraph":cgraph,
            "canonicalLinksAndReferencesEqual":qgraph==cgraph,"signatureCollisions":collisions,
            "atomIdentityCollisions":atom_collisions(q)||atom_collisions(c),"graphIsomorphismEstablished":false,
            "speechActEqual":q.frame["speech_act"]==c.frame["speech_act"],"queryOriginalFrame":q.frame,"candidateOriginalFrame":c.frame},
        "occurrenceMatches":occurrence_matches,"geometryCorrespondences":geometry(q,c)?,
        "multiplicity":{"queryOccurrences":q.atoms.len(),"candidateOccurrences":c.atoms.len(),
            "queryEvents":qe.len(),"candidateEvents":ce.len(),"eventSignatureCountsEqual":count_keys(&qe)==count_keys(&ce),
            "occurrenceCountEqual":q.atoms.len()==c.atoms.len(),"zeroDistanceEstablishesCompleteEquivalence":false},
        "coverage":{"querySourceOccurrences":q.atoms.iter().filter(|a|!a.sense_ids.is_empty()).count(),
            "candidateSourceOccurrences":c.atoms.iter().filter(|a|!a.sense_ids.is_empty()).count(),
            "queryAssertedConceptOccurrences":q.atoms.iter().filter(|a|!a.asserted_concepts.is_empty()).count(),
            "candidateAssertedConceptOccurrences":c.atoms.iter().filter(|a|!a.asserted_concepts.is_empty()).count()},
        "uncertainty":{"query":q.uncertainty,"candidate":c.uncertainty}}))
}
fn work_budget<'a>(
    query: &VerifiedRetrievalInput,
    candidates: impl Iterator<Item = &'a VerifiedRetrievalInput> + Clone,
    limits: &RetrievalLimits,
) -> Result<()> {
    limits.validate()?;
    if candidates.clone().count() > limits.max_candidates {
        return Err("candidate budget exceeded; split without dropping records".into());
    }
    let bytes = candidates
        .clone()
        .try_fold(query.bytes, |sum, c| sum.checked_add(c.bytes))
        .ok_or("aggregate byte overflow")?;
    if bytes > limits.max_aggregate_bytes {
        return Err("aggregate packet budget exceeded; split without dropping records".into());
    }
    let mut pairs = 0usize;
    let mut records = 0usize;
    let mut estimate = bytes;
    for c in candidates {
        compatible(query, c)?;
        pairs = pairs
            .checked_add(query.readings.len() * c.readings.len())
            .ok_or("reading pair overflow")?;
        for q in &query.readings {
            for r in &c.readings {
                records = records
                    .checked_add(
                        q.atoms.len() * r.atoms.len()
                            + q.geometry.len() * r.geometry.len()
                            + list(&q.frame["events"])?.len() * list(&r.frame["events"])?.len()
                            + center_count(q)? * center_count(r)?,
                    )
                    .ok_or("record pair overflow")?;
                let source_q = q
                    .atoms
                    .iter()
                    .map(|a| encoded_len(&a.selected))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .max()
                    .unwrap_or(0);
                let source_r = r
                    .atoms
                    .iter()
                    .map(|a| encoded_len(&a.selected))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .max()
                    .unwrap_or(0);
                let qevents = events(q)?;
                let revents = events(r)?;
                let qc = q
                    .atoms
                    .iter()
                    .flat_map(|a| a.sense_snap["centers"].as_array().into_iter().flatten());
                let rc = r
                    .atoms
                    .iter()
                    .flat_map(|a| a.sense_snap["centers"].as_array().into_iter().flatten());
                let terms = [
                    (q.atoms.len() * r.atoms.len(), source_q + source_r + 1024),
                    (
                        q.geometry.len() * r.geometry.len(),
                        max_bytes(q.geometry.iter())?
                            + max_bytes(r.geometry.iter())?
                            + source_q
                            + source_r
                            + 1024,
                    ),
                    (
                        qevents.len() * revents.len(),
                        max_bytes(qevents.iter())? + max_bytes(revents.iter())? + 1024,
                    ),
                    (
                        center_count(q)? * center_count(r)?,
                        max_bytes(qc)? + max_bytes(rc)? + 1024,
                    ),
                ];
                for (number, size) in terms {
                    estimate = estimate
                        .checked_add(
                            number
                                .checked_mul(size)
                                .ok_or("output preflight overflow")?,
                        )
                        .ok_or("output preflight overflow")?;
                    if estimate > OUTPUT_BYTES {
                        return Err("complete pair/proof output preflight exceeds 32 MiB; split without dropping alternatives".into());
                    }
                }
            }
        }
    }
    if pairs > limits.max_reading_pairs || records > limits.max_record_pairs {
        return Err(
            "complete reading/record pair budget exceeded; split without dropping alternatives"
                .into(),
        );
    }
    Ok(())
}
pub fn compare_structured(
    query: &VerifiedRetrievalInput,
    candidate: &VerifiedRetrievalInput,
    limits: &RetrievalLimits,
) -> Result<Value> {
    work_budget(query, std::iter::once(candidate), limits)?;
    compare_unchecked(query, candidate)
}
fn compare_unchecked(
    query: &VerifiedRetrievalInput,
    candidate: &VerifiedRetrievalInput,
) -> Result<Value> {
    let mut pairs = Vec::new();
    let mut output_bytes = 0usize;
    for q in &query.readings {
        for c in &candidate.readings {
            let pair = compare_reading(query, q, candidate, c)?;
            charge(&mut output_bytes, &pair)?;
            pairs.push(pair);
        }
    }
    let result = json!({"schema":"bomb-code/structured-comparison/v1","queryStateHash":query.state.state_hash,"candidateStateHash":candidate.state.state_hash,
        "compatibleSourceSnapshot":true,"scopes":scopes(query,candidate),
        "originalCdissSourceDistance":measured(&query.state.observation.source_vector,&candidate.state.observation.source_vector)?,
        "originalCdissStructureDistance":measured(&query.state.observation.structure_vector,&candidate.state.observation.structure_vector)?,
        "originalPartitionChanged":query.state.observation.partition_hash!=candidate.state.observation.partition_hash,
        "readingPairs":pairs,"overallRelevanceScore":Value::Null,
        "notice":"Separate evidence comparisons; allocated mass is not confidence, linguistic correctness or completion"});
    if encoded_len(&result)? > OUTPUT_BYTES {
        return Err(
            "complete comparison output exceeds 32 MiB; split without dropping evidence".into(),
        );
    }
    Ok(result)
}
/// Direct ephemeral concept postings, stable supplied order, no learned relevance rank.
pub fn retrieve_structured(
    query: &VerifiedRetrievalInput,
    candidates: &[(String, VerifiedRetrievalInput)],
    limits: &RetrievalLimits,
) -> Result<Value> {
    work_budget(query, candidates.iter().map(|(_, c)| c), limits)?;
    let mut ids = BTreeSet::new();
    for (id, _) in candidates {
        if id.is_empty() || id.len() > 200 || !ids.insert(id) {
            return Err("empty/oversized/duplicate candidate ID".into());
        }
    }
    let mut postings = BTreeMap::<String, Vec<Value>>::new();
    let mut count = 0usize;
    let mut posting_bytes = 0usize;
    let mut output_bytes = 16384usize;
    let mut matched_postings = 0usize;
    for (id, profile) in candidates {
        for reading in &profile.readings {
            for atom in &reading.atoms {
                for cid in &atom.asserted_concepts {
                    count += 1;
                    if count > limits.max_postings {
                        return Err("concept posting budget exceeded; nothing truncated".into());
                    }
                    charge_bytes(&mut posting_bytes, encoded_len(&atom.selected)? + 1024)?;
                    let proof: Vec<_> = atom
                        .selected
                        .iter()
                        .filter(|s| {
                            list(&s["alignments"]).is_ok_and(|a| {
                                a.iter().any(|r| {
                                    r["concept_id"] == *cid
                                        && r["kind"] == "equivalent"
                                        && r["asserted"] == true
                                })
                            })
                        })
                        .cloned()
                        .collect();
                    postings.entry(cid.clone()).or_default().push(json!({"candidateId":id,"readingId":reading.id,"atomId":atom.id,"span":atom.span,
                "sourceProof":proof,"candidateStateHash":profile.state.state_hash,"candidateEvidenceHash":profile.state.evidence_hash}));
                }
            }
        }
    }
    let mut matches = Vec::new();
    let mut root_postings = BTreeMap::<String, Vec<Value>>::new();
    for (id, profile) in candidates {
        for reading in &profile.readings {
            for activation in &reading.geometry {
                if let Some(root) = activation["placement"]["root_id"].as_str() {
                    count += 1;
                    if count > limits.max_postings {
                        return Err(
                            "combined concept/root posting budget exceeded; nothing truncated"
                                .into(),
                        );
                    }
                    let source_atom = atom(reading, &activation["atom_id"])?;
                    charge_bytes(
                        &mut posting_bytes,
                        encoded_len(&source_atom.selected)? + encoded_len(activation)? + 1024,
                    )?;
                    let proof: Vec<_> = source_atom
                        .selected
                        .iter()
                        .filter(|s| s["sense"]["id"] == activation["sense_id"])
                        .cloned()
                        .collect();
                    root_postings.entry(root.to_owned()).or_default().push(json!({
                "candidateId":id,"readingId":reading.id,"atomId":activation["atom_id"],"span":source_atom.span,
                "activation":activation,"sourceProof":proof,"candidateStateHash":profile.state.state_hash
            }));
                }
            }
        }
    }
    let mut root_matches = Vec::new();
    let mut root_hits = BTreeSet::new();
    for reading in &query.readings {
        for activation in &reading.geometry {
            if let Some(root) = activation["placement"]["root_id"].as_str() {
                if let Some(found) = root_postings.get(root) {
                    matched_postings = matched_postings
                        .checked_add(found.len())
                        .ok_or("posting fanout overflow")?;
                    if matched_postings > limits.max_postings {
                        return Err("matched concept/root posting fanout budget exceeded; nothing truncated".into());
                    }
                    charge_bytes(
                        &mut output_bytes,
                        encoded_len(found)? + encoded_len(activation)? + 2048,
                    )?;
                    for posting in found {
                        root_hits.insert(string(&posting["candidateId"])?);
                    }
                    root_matches.push(json!({"rootId":root,"queryReadingId":reading.id,
                    "queryActivation":activation,"postings":found,"semanticEquivalenceInferred":false}));
                }
            }
        }
    }
    let mut hits = BTreeSet::new();
    for reading in &query.readings {
        for atom in &reading.atoms {
            for cid in &atom.asserted_concepts {
                if let Some(found) = postings.get(cid) {
                    matched_postings = matched_postings
                        .checked_add(found.len())
                        .ok_or("posting fanout overflow")?;
                    if matched_postings > limits.max_postings {
                        return Err("matched concept/root posting fanout budget exceeded; nothing truncated".into());
                    }
                    charge_bytes(
                        &mut output_bytes,
                        encoded_len(found)? + encoded_len(&atom.selected)? + 2048,
                    )?;
                    for posting in found {
                        hits.insert(string(&posting["candidateId"])?);
                    }
                    matches.push(json!({"conceptId":cid,"queryReadingId":reading.id,"queryAtomId":atom.id,"querySpan":atom.span,
                "querySourceProof":atom.selected,"postings":found}));
                }
            }
        }
    }
    let mut comparisons = Vec::new();
    for (id, profile) in candidates {
        let comparison = compare_unchecked(query, profile)?;
        charge_bytes(&mut output_bytes, encoded_len(&comparison)? + 512)?;
        comparisons.push(
            json!({"candidateId":id,"conceptCandidate":hits.contains(id),
        "comparison":comparison}),
        );
    }
    let result = json!({"schema":"bomb-code/structured-retrieval/v1","queryStateHash":query.state.state_hash,
        "conceptPostings":matches,"matchedCandidateIds":hits,"candidateComparisons":comparisons,"postingCount":count,
        "rootBucketPostings":root_matches,"rootBucketCandidateIds":root_hits,
        "rootBucketMeaning":"Exact native coarse addresses only; distinct source/concept/fine coordinates retained; no equivalence or score",
        "candidateCount":candidates.len(),"queryReadingCount":query.readings.len(),"limits":limits,
        "candidateOrder":"supplied identity order; no relevance score",
        "authority":{"toolsDispatched":false,"approvalsGranted":false,"memoryCommitted":false},
        "qualification":"Complete retained proposals; no corpus annotation or inferred interpretation; root collisions are not concept equality"});
    if serde_json::to_vec(&result)
        .map_err(|e| e.to_string())?
        .len()
        > 32 * 1024 * 1024
    {
        return Err("complete output exceeds 32 MiB; split without discarding evidence".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile(name: &str) -> VerifiedRetrievalInput {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("joe-{name}.json"));
        VerifiedRetrievalInput::from_native(
            &serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn active_passive_preserves_content_but_not_original_frame() {
        let q = profile("bank");
        let c = profile("passive");
        let v = compare_structured(&q, &c, &RetrievalLimits::default()).unwrap();
        assert_eq!(
            v["readingPairs"][0]["eventLocal"]["conceptNormalizedDistance"]["totalVariation"],
            0.0
        );
        assert_ne!(q.native_packet()["sentence"], c.native_packet()["sentence"]);
        assert_eq!(
            v["readingPairs"][0]["eventLocal"]["queryOriginalFrame"],
            q.native_packet()["interpretation"]["binding"]["readings"][0]["frame"]
        );
        assert_eq!(
            v["readingPairs"][0]["multiplicity"]["zeroDistanceEstablishesCompleteEquivalence"],
            false
        );
    }
    #[test]
    fn negation_condition_remain_event_local() {
        let q = profile("bank");
        for name in ["negative", "conditional"] {
            let c = profile(name);
            let v = compare_structured(&q, &c, &RetrievalLimits::default()).unwrap();
            assert!(
                v["readingPairs"][0]["eventLocal"]["conceptNormalizedDistance"]["totalVariation"]
                    .as_f64()
                    .unwrap()
                    > 0.0
            );
            assert_eq!(
                v["readingPairs"][0]["sourceSense"]["distance"]["totalVariation"],
                0.0
            );
        }
    }
    #[test]
    fn direct_postings_and_budgets() {
        let q = profile("bank");
        let c = profile("passive");
        let v = retrieve_structured(
            &q,
            &[("passive".into(), c.clone())],
            &RetrievalLimits::default(),
        )
        .unwrap();
        assert_eq!(v["matchedCandidateIds"], json!(["passive"]));
        assert!(!v["conceptPostings"].as_array().unwrap().is_empty());
        let limits = RetrievalLimits {
            max_postings: 1,
            ..Default::default()
        };
        assert!(retrieve_structured(&q, &[("passive".into(), c)], &limits).is_err());
    }
    #[test]
    fn independent_context_pins_and_missing_evidence() {
        let q = profile("bank");
        let c = profile("context-pin");
        let v = compare_structured(&q, &c, &RetrievalLimits::default()).unwrap();
        assert!(v["readingPairs"][0]["senseSnap"]["contextPinDistance"].is_null());
        assert_eq!(
            v["readingPairs"][0]["assertedConcept"]["distance"]["totalVariation"],
            0.0
        );
        assert!(
            c.audit_profile()["readings"][0]["occurrences"][0]["senseSnap"]["centers"]
                .as_array()
                .unwrap()
                .len()
                > 1
        );
    }
    #[test]
    fn malformed_dictionary_center_and_changed_coordinates_rejected() {
        let q = profile("bank");
        let mut raw = q.native_packet().clone();
        raw["interpretation"]["binding"]["readings"][0]["bound_usage"]["atoms"][0]["sense_snap"]
            ["centers"][0]["source_sense_ids"] = json!(["sense:wrong"]);
        assert!(VerifiedRetrievalInput::from_native(&raw).is_err());
        let mut raw = q.native_packet().clone();
        raw["interpretation"]["binding"]["readings"][0]["e8_activations"][0]["placement"]
            ["position8"][0] = json!(0.5);
        assert!(VerifiedRetrievalInput::from_native(&raw).is_err());
    }
    #[test]
    fn mixed_unresolved_alternatives_survive_concept_identity() {
        let q = profile("bank");
        let mut a = q.readings[0].atoms[0].clone();
        a.selected
            .push(json!({"sense":{"id":"sense:unresolved"},"alignments":[]}));
        a.sense_ids.insert("sense:unresolved".into());
        assert_eq!(
            semantic_identity(&a)["unresolvedSourceSenseIds"],
            json!(["sense:unresolved"])
        );
        assert!(!fully_asserted(&a));
    }
    #[test]
    fn dictionary_meeting_and_display_tampering_rejected() {
        let q = profile("bank");
        for field in ["meeting", "position"] {
            let mut raw = q.native_packet().clone();
            let center = &mut raw["interpretation"]["binding"]["readings"][0]["bound_usage"]
                ["atoms"][0]["sense_snap"]["centers"][0];
            if field == "meeting" {
                center["meeting_id"] = json!("wrong");
            } else {
                center["display"]["placement"]["position8"][0] = json!(0.75);
            }
            assert!(VerifiedRetrievalInput::from_native(&raw).is_err());
        }
    }
    #[test]
    fn repeated_atom_identity_is_explicit_collision() {
        let q = profile("bank");
        let mut reading = q.readings[0].clone();
        assert!(!atom_collisions(&reading));
        reading.atoms.push(reading.atoms[0].clone());
        assert!(atom_collisions(&reading));
    }
    #[test]
    fn profile_audit_preflight_matches_real_output_upper_bound() {
        for name in ["bank", "passive", "context-pin"] {
            let p = profile(name);
            assert!(encoded_len(&p.audit_profile()).unwrap() <= p.audit_size_upper_bound());
        }
    }
    #[test]
    fn proof_amplification_rejected_before_pair_materialization() {
        let q = profile("bank");
        let mut c = q.clone();
        // Internal synthetic stress input exercises budgeting, not native authentication.
        c.readings[0].atoms[0].selected[0]["sense"]["definition"] =
            json!("x".repeat(17 * 1024 * 1024));
        assert!(compare_structured(&q, &c, &RetrievalLimits::default())
            .unwrap_err()
            .contains("preflight"));
    }
    #[test]
    fn repeated_query_posting_fanout_rejected_without_dropping_readings() {
        let candidate = profile("bank");
        let mut query = candidate.clone();
        let mut second = query.readings[0].clone();
        second.id = "second".into();
        query.readings.push(second);
        let limits = RetrievalLimits {
            max_postings: 4,
            ..Default::default()
        };
        assert!(
            retrieve_structured(&query, &[("bank".into(), candidate)], &limits)
                .unwrap_err()
                .contains("fanout")
        );
    }
    #[test]
    fn cross_thread_context_unavailable_while_source_comparison_remains() {
        let q = profile("context-pin");
        let mut raw = q.native_packet().clone();
        raw["threadId"] = json!("other-thread");
        let c = VerifiedRetrievalInput::from_native(&raw).unwrap();
        let v = compare_structured(&q, &c, &RetrievalLimits::default()).unwrap();
        assert_eq!(
            v["readingPairs"][0]["assertedConcept"]["distance"]["totalVariation"],
            0.0
        );
        assert!(v["readingPairs"][0]["senseSnap"]["contextPinDistance"].is_null());
        assert_eq!(v["scopes"]["threadEqual"], false);
    }
    #[test]
    fn native_source_decimal_preserves_exact_ieee_value_through_json() {
        // Real source coordinate exposed a one-ULP default serde_json parsing
        // error. Compare to Rust's correctly rounded standard decimal parser;
        // no tolerance may conceal a changed imported coordinate.
        for decimal in [
            "0.42064063095769044",
            "-0.42064063095769044",
            "0.04118380581665001",
            "0.5783254736198649",
            "0.7401354098744385",
            "5e-324",
        ] {
            let expected = decimal.parse::<f64>().unwrap().to_bits();
            let parsed: Value = serde_json::from_str(decimal).unwrap();
            assert_eq!(parsed.as_f64().unwrap().to_bits(), expected, "{decimal}");
            let encoded = serde_json::to_string(&parsed).unwrap();
            let reloaded: Value = serde_json::from_str(&encoded).unwrap();
            assert_eq!(reloaded.as_f64().unwrap().to_bits(), expected, "{decimal}");
        }
    }
}

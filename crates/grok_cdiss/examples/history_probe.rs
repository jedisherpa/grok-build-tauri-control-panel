//! Read private native bindings; emit only IDs, hashes and numeric summaries.
//! Authored lexical frames do not qualify language understanding or intent.
use grok_cdiss::{analyze_joe_result, Config, State};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("binding-results path required")?;
    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut prior: BTreeMap<String, State> = BTreeMap::new();
    let mut rows = Vec::new();
    for case in cases {
        if case["status"] != "bound" {
            rows.push(json!({"caseId":case["caseId"],"status":"native-binding-rejected","reason":case["reason"]}));
            continue;
        }
        let input: Value = serde_json::from_slice(&std::fs::read(
            case["path"].as_str().ok_or("path missing")?,
        )?)?;
        let group = case["threadId"]
            .as_str()
            .ok_or("thread scope missing")?
            .to_string();
        let previous = prior.get(&group);
        let start = Instant::now();
        match analyze_joe_result(&input, previous, &Config::default()) {
            Ok(state) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                let replay = analyze_joe_result(&input, previous, &Config::default())?;
                let native = &input["interpretation"]["binding"]["readings"][0];
                let geometry_equal = serde_json::to_value(&state.observation.readings[0].geometry)? == native["e8_activations"];
                let atoms = native["bound_usage"]["atoms"].as_array().ok_or("atoms missing")?;
                let candidate_retained = state.observation.readings[0].atoms.iter().zip(atoms).all(|(o,a)| {
                    let mut ids: Vec<_> = a["all_source_candidates"].as_array().unwrap().iter()
                        .map(|c|c["sense"]["id"].as_str().unwrap()).collect();
                    ids.sort_unstable(); ids.dedup();
                    ids == o.all_candidate_sense_ids.iter().map(String::as_str).collect::<Vec<_>>()
                }) && state.observation.readings[0].atoms.len() == atoms.len();
                let native_center_count: usize = atoms.iter().map(|a| a["sense_snap"]["centers"].as_array().unwrap().len()).sum();
                let native_context_pin_count: usize = atoms.iter().map(|a| a["sense_snap"]["centers"].as_array().unwrap().iter().filter(|c|c["origin"]=="context-pin").count()).sum();
                let centers_equal = state.observation.readings[0].atoms.iter().zip(atoms)
                    .all(|(o,a)| {
                        let expected: Vec<_> = a["sense_snap"]["centers"].as_array().unwrap().iter().filter(|c| c["origin"] == "context-pin").cloned().collect();
                        serde_json::to_value(&o.context_centers).unwrap() == serde_json::to_value(expected).unwrap()
                    });
                rows.push(json!({"caseId":case["caseId"],"source":case["source"],"status":"ready", "stateHash":state.state_hash,
                    "inputHash":state.input_hash,"evidenceHash":state.evidence_hash,"previousStateHash":state.previous_state_hash,
                    "replayIdentical":state.state_hash==replay.state_hash,"nativeGeometryExactlyRetained":geometry_equal,
                    "sourceCandidatesExactlyRetained":candidate_retained,"senseSnapContextPinsExactlyRetained":centers_equal,"nativeSenseSnapCenterCount":native_center_count,"nativeContextPinCount":native_context_pin_count,"contextPinCoverage":if native_context_pin_count==0 {"not-exercised"} else {"exercised"},
                    "readingCount":state.observation.reading_count,"atomCount":state.observation.atom_count,"eventCount":state.observation.event_count,
                    "alternativeCount":state.observation.alternative_count,"mappedMass":state.observation.mapped_mass,
                    "unmappedMass":state.observation.unmapped_mass,"sourceFeatureCount":state.observation.source_vector.len(),
                    "structureFeatureCount":state.observation.structure_vector.len(),"continuity":state.continuity.status,
                    "sourceDistance":state.continuity.source_distance,"structureDistance":state.continuity.structure_distance,
                    "partitionChanged":state.continuity.partition_changed,"elapsedMs":elapsed_ms}));
                prior.insert(group,state);
            }
            Err(error) => rows.push(json!({"caseId":case["caseId"],"source":case["source"],"status":"cdiss-rejected","reason":error.to_string()})),
        }
    }
    println!(
        "{}",
        serde_json::to_string(
            &json!({"schema":"bomb-code/history-cdiss-probe/v1", "verificationClass":"actual private passages, authored lexical-only frames; no linguistic accuracy measured", "cases":rows})
        )?
    );
    Ok(())
}

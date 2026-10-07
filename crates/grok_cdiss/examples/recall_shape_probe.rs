//! Controlled retrieval experiment on source-bound authored frames, not NLP accuracy.
use grok_cdiss::{analyze_joe_result, distance, word_shapes, Config, State};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn load(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("joe-{name}.json"));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn signature(shape: &Value) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for reading in shape["readings"].as_array().unwrap() {
        for occurrence in reading["occurrences"].as_array().unwrap() {
            for role in occurrence["roleBindings"].as_array().unwrap() {
                let identities: Vec<_> = occurrence["alternatives"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|a| a["selected"] == true)
                    .map(|a| a["senseId"].clone())
                    .collect();
                // IDs/spans differ between active and passive; compare role/sense/scope content.
                let identity = if identities.is_empty() {
                    json!({"unmapped":occurrence["surface"]})
                } else {
                    json!(identities)
                };
                keys.insert(
                    json!([identity, role["role"], role["polarity"], role["modality"]]).to_string(),
                );
            }
        }
    }
    keys
}
fn roots(state: &State) -> Vec<String> {
    let mut values: Vec<_> = state
        .observation
        .readings
        .iter()
        .flat_map(|r| &r.geometry)
        .filter_map(|a| a["placement"]["root_id"].as_str().map(str::to_owned))
        .collect();
    values.sort();
    values
}
fn audit_saved(folder: &std::path::Path) {
    let baseline_packet = load("bank");
    let baseline = analyze_joe_result(&baseline_packet, None, &Config::default()).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    assert!(files.len() <= 64, "receipt sample budget exceeded");
    let mut rows = Vec::new();
    for path in files {
        assert!(
            std::fs::metadata(&path).unwrap().len() <= 32 * 1024 * 1024,
            "receipt input budget exceeded"
        );
        let raw = std::fs::read(&path).unwrap();
        let before = hex::encode(Sha256::digest(&raw));
        let packet: Value = serde_json::from_slice(&raw).unwrap();
        let attempt = analyze_joe_result(&packet, None, &Config::default());
        let result = match attempt {
            Err(error) => json!({"status":"unavailable","reason":error.to_string()}),
            Ok(state) => {
                let pins_match = state.basis.manifest_sha256 == baseline.basis.manifest_sha256
                    && state.basis.source_graph_hash == baseline.basis.source_graph_hash
                    && state.basis.model_snapshot_hash == baseline.basis.model_snapshot_hash
                    && state.basis.model_id == baseline.basis.model_id;
                assert!(
                    pins_match,
                    "saved proposal dictionary/model differs from native fixture basis"
                );
                let shared =
                    &packet["interpretation"]["binding"]["source_packet"]["shared_reference"];
                // Python fixtures encode this numeric scale as 8.0; saved packets use 8.
                // Normalize this declared numeric field only, preserving every other pin.
                let mut actual_reference = shared.clone();
                let mut fixture_reference = baseline_packet["interpretation"]["binding"]
                    ["source_packet"]["shared_reference"]
                    .clone();
                actual_reference["lattice_scale"] =
                    json!(shared["lattice_scale"].as_f64().unwrap());
                fixture_reference["lattice_scale"] =
                    json!(fixture_reference["lattice_scale"].as_f64().unwrap());
                assert_eq!(actual_reference, fixture_reference);
                let code_pins = shared["sense_snap"]["implementation_hashes"]
                    .as_object()
                    .unwrap();
                for (relative, expected) in code_pins {
                    assert!(matches!(
                        relative.as_str(),
                        "semantic_e8/sense_snap.py"
                            | "sensesnap/src/sensesnap/place.py"
                            | "sensesnap/src/sensesnap/store.py"
                    ));
                    let current =
                        std::fs::read(folder.parent().unwrap().join("reference").join(relative))
                            .unwrap();
                    assert_eq!(
                        hex::encode(Sha256::digest(current)),
                        expected.as_str().unwrap()
                    );
                }
                let shapes = word_shapes(&packet).unwrap();
                let mut selected_count = 0;
                let mut role_count = 0;
                let mut geometry_count = 0;
                let shape_readings = shapes["readings"].as_array().unwrap();
                let native_readings = packet["interpretation"]["binding"]["readings"]
                    .as_array()
                    .unwrap();
                assert_eq!(shape_readings.len(), native_readings.len());
                for (reading, native) in shape_readings.iter().zip(native_readings) {
                    assert_eq!(reading["frame"], native["frame"]);
                    assert_eq!(
                        reading["occurrences"].as_array().unwrap().len(),
                        native["bound_usage"]["atoms"].as_array().unwrap().len()
                    );
                    let mut retained_activations = Vec::new();
                    for occurrence in reading["occurrences"].as_array().unwrap() {
                        role_count += occurrence["roleBindings"].as_array().unwrap().len();
                        for candidate in occurrence["alternatives"].as_array().unwrap() {
                            if candidate["selected"] == true {
                                selected_count += 1;
                            }
                            for activation in candidate["e8"]["activations"].as_array().unwrap() {
                                retained_activations
                                    .push(serde_json::to_string(activation).unwrap());
                                geometry_count += 1;
                            }
                        }
                    }
                    let mut original_activations: Vec<_> = native["e8_activations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|activation| serde_json::to_string(activation).unwrap())
                        .collect();
                    retained_activations.sort();
                    original_activations.sort();
                    assert_eq!(retained_activations, original_activations);
                }
                json!({"status":"ready","readings":shapes["readings"].as_array().unwrap().len(),"selectedSourceAlternatives":selected_count,"roleBindings":role_count,"retainedNativeActivations":geometry_count,"sourceAndModelPinsMatchNativeFixture":pins_match,"sharedReferenceWithNumericScaleAndCurrentSenseSnapCodeMatch":true,"framesAndActivationsRetainedExactly":true,"tokenCoverage":{"tokens":shapes["tokenCoverage"]["sentenceTokenCount"],"uncovered":shapes["tokenCoverage"]["uncoveredTokenCount"]}})
            }
        };
        let after = hex::encode(Sha256::digest(std::fs::read(&path).unwrap()));
        assert_eq!(before, after);
        rows.push(json!({"receiptFile":path.file_name().unwrap().to_string_lossy(),"sha256":before,"originalPreserved":true,"result":result}));
    }
    println!("{}",serde_json::to_string_pretty(&json!({"schema":"bomb-code/saved-shape-fidelity/v1","verificationClass":"saved model proposals, not independent linguistic labels; dictionary/model fixture basis crosscheck, original files read-only","receipts":rows,"providerCalls":0,"nativeWrites":0})).unwrap());
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--saved-receipts" {
        audit_saved(std::path::Path::new(&args[2]));
        return;
    }
    assert_eq!(
        args.len(),
        1,
        "expected no arguments or --saved-receipts DIRECTORY"
    );
    let mut packets = BTreeMap::new();
    for name in ["bank", "passive", "negative", "conditional"] {
        packets.insert(name.to_owned(), load(name));
    }
    let mut reversed = load("bank");
    let roles =
        &mut reversed["interpretation"]["binding"]["readings"][0]["frame"]["events"][0]["roles"];
    roles[0]["role"] = json!("theme");
    roles[1]["role"] = json!("agent");
    packets.insert("reversed-role".to_owned(), reversed);
    let states: BTreeMap<_, _> = packets
        .iter()
        .map(|(id, p)| {
            (
                id.clone(),
                analyze_joe_result(p, None, &Config::default()).unwrap(),
            )
        })
        .collect();
    let shapes: BTreeMap<_, _> = packets
        .iter()
        .map(|(id, p)| (id.clone(), word_shapes(p).unwrap()))
        .collect();
    let mut rows = Vec::new();
    for (query_id, query) in &states {
        let mut source_ties = Vec::new();
        let mut shape_ties = Vec::new();
        let mut root_ties = Vec::new();
        let mut comparisons = Vec::new();
        for (document_id, document) in &states {
            let source = distance(
                &query.observation.source_vector,
                &document.observation.source_vector,
            )
            .unwrap()
            .total_variation;
            let structure = distance(
                &query.observation.structure_vector,
                &document.observation.structure_vector,
            )
            .unwrap()
            .total_variation;
            let shape_equal = signature(&shapes[query_id]) == signature(&shapes[document_id]);
            if source == 0.0 {
                source_ties.push(document_id);
            }
            if shape_equal {
                shape_ties.push(document_id);
            }
            if roots(query) == roots(document) {
                root_ties.push(document_id);
            }
            assert_eq!(
                shape_equal,
                structure == 0.0,
                "shape role/scope signature must agree with CDISS authored-frame equality"
            );
            comparisons.push(json!({"document":document_id,"sourceTV":source,"structureTV":structure,"shapeRoleScopeEqual":shape_equal}));
        }
        let expected: BTreeSet<_> = if query_id == "bank" || query_id == "passive" {
            ["bank", "passive"].into_iter().map(str::to_owned).collect()
        } else {
            [query_id.clone()].into_iter().collect()
        };
        assert_eq!(
            shape_ties
                .iter()
                .map(|v| (*v).clone())
                .collect::<BTreeSet<_>>(),
            expected
        );
        rows.push(json!({"query":query_id,"sourceOnlyZeroDistanceCandidates":source_ties,"rootOnlyTies":root_ties,"shapeRoleScopeZeroDistanceCandidates":shape_ties,"expectedAuthoredFrameMatches":expected,"comparisons":comparisons}));
    }
    let collision: Value =
        serde_json::from_str(include_str!("../tests/fixtures/source-root-collision.json")).unwrap();
    let records = collision["records"].as_array().unwrap();
    let collision_status = json!({"sameRoot":records[0]["placement"]["root_id"]==records[1]["placement"]["root_id"],"differentSenseIds":records[0]["sense"]["id"]!=records[1]["sense"]["id"],"differentFineCoordinates":records[0]["placement"]["position8"]!=records[1]["placement"]["position8"]});
    assert_eq!(collision_status["sameRoot"], true);
    assert_eq!(collision_status["differentSenseIds"], true);
    assert_eq!(collision_status["differentFineCoordinates"], true);
    println!("{}",serde_json::to_string_pretty(&json!({"schema":"bomb-code/controlled-shape-retrieval/v1","verificationClass":"five source-bound authored frames; labels supplied, no automatic interpretation or human relevance claim","checksPassed":true,"queries":rows,"rootCollision":collision_status,"scope":"Structured sense-role-polarity-modality information distinguishes these supplied frames; E8 roots/source bags alone do not. No provider call, memory write or production ranking change."})).unwrap());
}

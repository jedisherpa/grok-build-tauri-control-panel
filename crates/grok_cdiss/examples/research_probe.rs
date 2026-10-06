//! Reproduce distances and CPU timings on actual frozen-source, authored grammar fixtures.
use grok_cdiss::{analyze_joe_result, distance, Config};
use serde_json::{json, Value};
use std::time::Instant;
fn main() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let load = |name: &str| {
        serde_json::from_slice::<Value>(
            &std::fs::read(folder.join(format!("joe-{name}.json"))).unwrap(),
        )
        .unwrap()
    };
    let bank = load("bank");
    let start = Instant::now();
    let baseline = analyze_joe_result(&bank, None, &Config::default()).unwrap();
    let first_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut rows = Vec::new();
    for name in ["bank", "passive", "negative", "conditional"] {
        let input = load(name);
        let start = Instant::now();
        let state = analyze_joe_result(&input, Some(&baseline), &Config::default()).unwrap();
        rows.push(json!({"case":name,"sourceDistance":distance(&baseline.observation.source_vector,&state.observation.source_vector).unwrap(),
            "structureDistance":distance(&baseline.observation.structure_vector,&state.observation.structure_vector).unwrap(),"partitionChanged":state.continuity.partition_changed,
            "nativePositionsUnchanged":baseline.observation.readings[0].geometry.iter().map(|a|&a["placement"]["position8"]).eq(state.observation.readings[0].geometry.iter().map(|a|&a["placement"]["position8"])),
            "elapsedMs":start.elapsed().as_secs_f64()*1000.0}));
    }
    println!("{}",serde_json::to_string_pretty(&json!({"scope":"authored grammar through unmodified pinned source binder; no live provider or human-intent accuracy claim", "firstAnalysisMs":first_ms,
        "baseline":{"readingCount":baseline.observation.reading_count,"atomCount":baseline.observation.atom_count,"mappedMass":baseline.observation.mapped_mass,"unmappedMass":baseline.observation.unmapped_mass,
            "modelId":baseline.basis.model_id,"sourceGraphHash":baseline.basis.source_graph_hash,"modelSnapshotHash":baseline.basis.model_snapshot_hash,"manifestSha256":baseline.basis.manifest_sha256,"stateHash":baseline.state_hash},"cases":rows})).unwrap());
}

use grok_cdiss::{analyze_joe_result, Config};
use serde_json::{json, Value};

fn native_fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/joe-context-pin.json")).unwrap()
}

#[test]
fn native_fitted_context_pin_and_unfitted_private_pin_keep_separate_origins() {
    let input = native_fixture();
    let unchanged = input.clone();
    let state = analyze_joe_result(&input, None, &Config::default()).unwrap();
    assert_eq!(input, unchanged);
    let bank = state.observation.readings[0]
        .atoms
        .iter()
        .find(|a| a.surface == "bank")
        .unwrap();
    assert_eq!(bank.alternative_count, 2);
    assert!(bank
        .selected_identities
        .iter()
        .any(|k| k == "context:pwn30:09213434-n"));
    assert!(bank
        .selected_identities
        .iter()
        .any(|k| k.starts_with("source:") && k.ends_with("pwn30:08420278-n")));
    let public = &bank.context_centers[0];
    assert_eq!(public["origin"], "context-pin");
    assert_eq!(public["source_mapping_asserted"], false);
    assert_eq!(public["semantic_concept_id"], "pwn30:09213434-n");
    assert!(public["source_sense_ids"].as_array().unwrap().is_empty());
    assert_eq!(public["display"]["placement"]["status"], "fitted");
    assert_eq!(
        public["display"]["placement"]["concept_id"],
        "pwn30:09213434-n"
    );
    assert_eq!(
        public["display"]["placement"]["position8"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert!(!bank.geometrically_unmapped);
    // Shoreline coordinates came from the native public-pin display path, rather
    // than an activation silently added to the financial dictionary selection.
    assert!(!state.observation.readings[0]
        .geometry
        .iter()
        .any(|a| a["concept_id"] == "pwn30:09213434-n"));
    let native_bank = input["interpretation"]["binding"]["readings"][0]["bound_usage"]["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["atom"]["surface"] == "bank")
        .unwrap();
    let native_public = native_bank["sense_snap"]["centers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["origin"] == "context-pin")
        .unwrap();
    assert_eq!(public, native_public);

    let loan = state.observation.readings[0]
        .atoms
        .iter()
        .find(|a| a.surface == "loan")
        .unwrap();
    assert_eq!(
        loan.selected_identities,
        ["context:meet:personal:financing-request"]
    );
    assert!(loan.geometrically_unmapped);
    let private = &loan.context_centers[0];
    assert_eq!(private["source_mapping_asserted"], false);
    assert!(private["semantic_concept_id"].is_null());
    assert!(private["display"].is_null());
    assert!((state.observation.unmapped_mass - 1.0 / 3.0).abs() < 1e-12);
    for flag in ["toolsDispatched", "approvalsGranted", "memoryCommitted"] {
        assert_eq!(input["authority"][flag], false);
    }
}

#[test]
fn invalid_public_pin_geometry_rejects_and_a_changed_pin_context_resets_continuity() {
    let mut input = native_fixture();
    let centers = input["interpretation"]["binding"]["readings"][0]["bound_usage"]["atoms"][0]
        ["sense_snap"]["centers"]
        .as_array_mut()
        .unwrap();
    let public = centers
        .iter_mut()
        .find(|c| c["origin"] == "context-pin")
        .unwrap();
    public["display"]["placement"]["position8"][0] = json!(99.0);
    assert!(analyze_joe_result(&input, None, &Config::default()).is_err());

    let original: Value = serde_json::from_str(include_str!("fixtures/joe-bank.json")).unwrap();
    let prior = analyze_joe_result(&original, None, &Config::default()).unwrap();
    let current = analyze_joe_result(&native_fixture(), Some(&prior), &Config::default()).unwrap();
    assert_eq!(current.continuity.status, "reset-incompatible");
    assert!(current.continuity.source_distance.is_none());
    assert_ne!(current.basis.context_hash, prior.basis.context_hash);
}

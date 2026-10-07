use grok_cdiss::word_shapes;
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    serde_json::from_slice(&std::fs::read(root.join(format!("joe-{name}.json"))).unwrap()).unwrap()
}
fn occurrence<'a>(output: &'a Value, id: &str) -> &'a Value {
    output["readings"][0]["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["atomId"] == id)
        .unwrap()
}

#[test]
fn every_candidate_and_source_field_survives_with_exact_native_geometry() {
    let input = fixture("bank");
    let before = input.clone();
    let output = word_shapes(&input).unwrap();
    assert_eq!(input, before);
    assert_eq!(output["schema"], "bomb-code/word-shapes/v1");
    let bank = occurrence(&output, "bank");
    assert_eq!(bank["tokenPosition"], 2);
    assert_eq!(bank["sentenceTokenCount"], 5);
    let candidates = input["interpretation"]["binding"]["source_packet"]["atoms"][0]["candidates"]
        .as_array()
        .unwrap();
    assert_eq!(
        bank["alternatives"].as_array().unwrap().len(),
        candidates.len()
    );
    for (shape, source) in bank["alternatives"]
        .as_array()
        .unwrap()
        .iter()
        .zip(candidates)
    {
        for (key, value) in source["sense"].as_object().unwrap() {
            assert_eq!(&shape["dictionary"][key], value);
        }
        assert_eq!(shape["crossLanguage"]["alignments"], source["alignments"]);
        assert_eq!(shape["conceptIds"], source["concept_ids"]);
    }
    let selected = bank["alternatives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["selected"] == true)
        .unwrap();
    assert_eq!(
        selected["e8"]["activations"][0],
        input["interpretation"]["binding"]["readings"][0]["e8_activations"][0]
    );
    assert_eq!(
        bank["senseSnap"],
        input["interpretation"]["binding"]["readings"][0]["bound_usage"]["atoms"][0]["sense_snap"]
    );
    assert_eq!(output["tokenCoverage"]["uncoveredTokenCount"], 2);
    assert_eq!(output["authority"]["toolsDispatched"], false);
}

#[test]
fn roles_rotate_only_declared_usage_arrows_and_scope_remains_explicit() {
    let input = fixture("bank");
    let active = word_shapes(&input).unwrap();
    let mut reverse = input.clone();
    let events = &mut reverse["interpretation"]["binding"]["readings"][0]["frame"]["events"];
    events[0]["roles"][0]["role"] = json!("theme");
    events[0]["roles"][1]["role"] = json!("agent");
    let reversed = word_shapes(&reverse).unwrap();
    assert_eq!(
        occurrence(&active, "bank")["usageOrientation"]["arrows"][0]["angleDegrees"],
        45
    );
    assert_eq!(
        occurrence(&reversed, "bank")["usageOrientation"]["arrows"][0]["angleDegrees"],
        135
    );
    assert_eq!(
        occurrence(&active, "bank")["alternatives"],
        occurrence(&reversed, "bank")["alternatives"]
    );
    assert_eq!(
        occurrence(&active, "bank")["usageOrientation"]["nativeGeometryRotated"],
        false
    );
    let negative = word_shapes(&fixture("negative")).unwrap();
    let conditional = word_shapes(&fixture("conditional")).unwrap();
    assert_eq!(
        occurrence(&negative, "bank")["roleBindings"][0]["polarity"],
        "negative"
    );
    assert_eq!(
        occurrence(&negative, "bank")["roleBindings"][0]["cueSpans"][0]["surface"],
        "not"
    );
    assert_eq!(
        occurrence(&conditional, "bank")["roleBindings"][0]["modality"],
        "conditional"
    );
    assert_eq!(
        occurrence(&conditional, "bank")["roleBindings"][0]["cueSpans"][0]["span"],
        json!([0, 2])
    );
    assert_eq!(
        occurrence(&active, "approve")["usageOrientation"]["arrows"][0]["role"],
        "predicate"
    );
}

#[test]
fn unknown_role_and_missing_source_data_have_no_fake_angles_or_markers() {
    let mut input = fixture("bank");
    input["interpretation"]["binding"]["readings"][0]["frame"]["events"][0]["roles"][0]["role"] =
        json!("beneficiary");
    let output = word_shapes(&input).unwrap();
    assert!(occurrence(&output, "bank")["usageOrientation"]["arrows"][0]["angleDegrees"].is_null());
    assert!(occurrence(&output, "loan")["alternatives"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["selected"] == false));
    assert_eq!(occurrence(&output, "loan")["surfaceUnmapped"], true);
    let source =
        &mut input["interpretation"]["binding"]["source_packet"]["atoms"][0]["candidates"][0];
    source["sense"]["definition"] = Value::Null;
    source["alignments"] = json!([]);
    let output = word_shapes(&input).unwrap();
    let alt = &occurrence(&output, "bank")["alternatives"][0];
    assert_eq!(alt["dictionary"]["definitionStatus"], "missing");
    assert_eq!(alt["crossLanguage"]["status"], "missing");
    assert_eq!(alt["e8"]["status"], "not-selected");
    let pinned = word_shapes(&fixture("context-pin")).unwrap();
    assert!(occurrence(&pinned, "loan")["senseSnap"]["centers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["origin"] == "context-pin"));
}

#[test]
fn unicode_scalar_spans_and_repeated_occurrences_keep_independent_positions() {
    let mut input = fixture("bank");
    let sentence = "é bank bank approved the loan.";
    input["sentence"] = json!(sentence);
    let binding = &mut input["interpretation"]["binding"];
    binding["request"]["sentence"] = json!(sentence);
    let spans = [[2, 6], [12, 20], [25, 29]];
    for (i, span) in spans.iter().enumerate() {
        binding["source_packet"]["atoms"][i]["atom"]["span"] = json!(span);
        binding["readings"][0]["bound_usage"]["atoms"][i]["atom"]["span"] = json!(span);
    }
    let mut packet = binding["source_packet"]["atoms"][0].clone();
    packet["atom"]["id"] = json!("bank2");
    packet["atom"]["span"] = json!([7, 11]);
    binding["source_packet"]["atoms"]
        .as_array_mut()
        .unwrap()
        .push(packet);
    let mut second = binding["readings"][0]["bound_usage"]["atoms"][0].clone();
    second["atom"]["id"] = json!("bank2");
    second["atom"]["span"] = json!([7, 11]);
    binding["readings"][0]["bound_usage"]["atoms"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let mut activation = binding["readings"][0]["e8_activations"][0].clone();
    activation["atom_id"] = json!("bank2");
    binding["readings"][0]["e8_activations"]
        .as_array_mut()
        .unwrap()
        .push(activation);
    let output = word_shapes(&input).unwrap();
    assert_eq!(occurrence(&output, "bank")["tokenPosition"], 2);
    assert_eq!(occurrence(&output, "bank2")["tokenPosition"], 3);
    assert_eq!(output["tokenCoverage"]["tokens"][0]["span"], json!([0, 1]));
    assert_eq!(
        output["tokenCoverage"]["tokens"][1]["readings"][0]["atomIds"],
        json!(["bank"])
    );
    assert_eq!(
        output["tokenCoverage"]["tokens"][2]["readings"][0]["atomIds"],
        json!(["bank2"])
    );
    // A partial lexical span remains an occurrence but does not falsely cover the whole token.
    input["sentence"] = json!("é banks bank approved the loan.");
    let b = &mut input["interpretation"]["binding"];
    b["request"]["sentence"] = json!("é banks bank approved the loan.");
    for a in b["source_packet"]["atoms"].as_array_mut().unwrap() {
        if a["atom"]["id"] != "bank" {
            for n in a["atom"]["span"].as_array_mut().unwrap() {
                *n = json!(n.as_u64().unwrap() + 1);
            }
        }
    }
    for a in b["readings"][0]["bound_usage"]["atoms"]
        .as_array_mut()
        .unwrap()
    {
        if a["atom"]["id"] != "bank" {
            for n in a["atom"]["span"].as_array_mut().unwrap() {
                *n = json!(n.as_u64().unwrap() + 1);
            }
        }
    }
    let output = word_shapes(&input).unwrap();
    assert_eq!(output["tokenCoverage"]["tokens"][1]["covered"], false);
}

#[test]
fn colliding_source_roots_keep_distinct_senses_and_fine_coordinates() {
    let collision: Value =
        serde_json::from_str(include_str!("fixtures/source-root-collision.json")).unwrap();
    let mut input = fixture("bank");
    let b = &mut input["interpretation"]["binding"];
    let candidates:Vec<_>=collision["records"].as_array().unwrap().iter().map(|r|json!({"sense":r["sense"],"concept_ids":[r["placement"]["concept_id"]],"alignments":[]})).collect();
    b["source_packet"]["atoms"][0]["candidates"] = json!(candidates);
    b["readings"][0]["bound_usage"]["atoms"][0]["selected_source_bindings"] = json!(candidates);
    let seed = b["readings"][0]["e8_activations"][0].clone();
    let mut activations: Vec<_> = b["readings"][0]["e8_activations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["atom_id"] != "bank")
        .cloned()
        .collect();
    for record in collision["records"].as_array().unwrap() {
        let mut a = seed.clone();
        a["sense_id"] = record["sense"]["id"].clone();
        a["concept_id"] = record["placement"]["concept_id"].clone();
        a["placement"] = record["placement"].clone();
        // Authored valid lattice address for these actual source points; no nearest-address claim.
        a["lattice_address"]["position"] = json!(vec![0.0; 8]);
        a["lattice_address"]["doubled_standard"] = json!(vec![0; 8]);
        a["lattice_address"]["residual"] = json!(record["placement"]["position8"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap() * 8.0)
            .collect::<Vec<_>>());
        b["source_packet"]["reference_points"]
            .as_array_mut()
            .unwrap()
            .push(record["placement"].clone());
        activations.push(a);
    }
    b["readings"][0]["e8_activations"] = json!(activations);
    let output = word_shapes(&input).unwrap();
    let alternatives = occurrence(&output, "bank")["alternatives"]
        .as_array()
        .unwrap();
    assert_ne!(alternatives[0]["senseId"], alternatives[1]["senseId"]);
    let first = &alternatives[0]["e8"]["activations"][0]["placement"];
    let second = &alternatives[1]["e8"]["activations"][0]["placement"];
    assert_eq!(first["root_id"], second["root_id"]);
    assert_ne!(first["position8"], second["position8"]);
}

#[test]
fn malformed_unavailable_authority_and_output_overflow_fail_closed() {
    assert!(word_shapes(&json!({})).is_err());
    let mut duplicated = fixture("bank");
    let reading = &mut duplicated["interpretation"]["binding"]["readings"][0];
    reading["frame"]["events"][0]["roles"] = json!([{ "role":"agent", "atom_id":"bank" }]);
    reading["bound_usage"]["atoms"][2] = reading["bound_usage"]["atoms"][0].clone();
    assert!(word_shapes(&duplicated)
        .unwrap_err()
        .contains("duplicate retained atom ID"));
    let mut input = fixture("bank");
    input["interpretation"]["binding"]["status"] = json!("unavailable");
    assert!(word_shapes(&input).is_err());
    let mut input = fixture("bank");
    input["authority"]["memoryCommitted"] = json!(true);
    assert!(word_shapes(&input).is_err());
    let mut input = fixture("bank");
    input["interpretation"]["binding"]["source_packet"]["atoms"][0]["candidates"][0]
        ["alignments"] = json!({});
    assert!(word_shapes(&input).is_err());
    let mut input = fixture("bank");
    input["interpretation"]["binding"]["source_packet"]["atoms"][0]["candidates"][0]["sense"]
        ["original_gloss"] = json!("x".repeat(9 * 1024 * 1024));
    assert!(word_shapes(&input).unwrap_err().contains("8 MiB"));
}

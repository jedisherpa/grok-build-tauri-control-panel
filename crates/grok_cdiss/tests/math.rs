use grok_cdiss::{analyze_joe_result, distance, stable_norm, Config, Feature, State};
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    serde_json::from_slice(&std::fs::read(root.join(format!("joe-{name}.json"))).unwrap()).unwrap()
}
fn analyze(value: &Value) -> State {
    analyze_joe_result(value, None, &Config::default()).unwrap()
}

#[test]
fn actual_source_geometry_is_retained_byte_for_byte_and_unknowns_remain() {
    let input = fixture("bank");
    let before = input.clone();
    let state = analyze(&input);
    assert_eq!(input, before);
    assert_eq!(
        state.observation.readings[0].geometry,
        *input["interpretation"]["binding"]["readings"][0]["e8_activations"]
            .as_array()
            .unwrap()
    );
    assert!(state.observation.unmapped_mass > 0.0);
    assert!(state.observation.readings[0]
        .atoms
        .iter()
        .any(|a| a.surface == "loan" && a.selected_identities == ["unmapped:loan"]));
    assert!(state
        .observation
        .source_vector
        .iter()
        .any(|f| f.key.starts_with("source:sense:")));
    let saved: State = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(saved.state_hash, state.state_hash);
    assert_eq!(
        analyze_joe_result(&input, Some(&saved), &Config::default())
            .unwrap()
            .continuity
            .status,
        "compared"
    );
}

#[test]
fn active_passive_and_arbitrary_local_id_renaming_preserve_declared_structure() {
    let active = analyze(&fixture("bank"));
    let passive = analyze(&fixture("passive"));
    assert_eq!(
        distance(
            &active.observation.source_vector,
            &passive.observation.source_vector
        )
        .unwrap()
        .total_variation,
        0.0
    );
    assert_eq!(
        distance(
            &active.observation.structure_vector,
            &passive.observation.structure_vector
        )
        .unwrap()
        .total_variation,
        0.0
    );
    assert_eq!(
        active.observation.partition_hash,
        passive.observation.partition_hash
    );
    let mut renamed = fixture("bank");
    fn rename(value: &mut Value) {
        match value {
            Value::String(s) => {
                if ["bank", "approve", "loan", "r1", "e1"].contains(&s.as_str()) {
                    *s = format!("local_{s}");
                }
            }
            Value::Array(a) => a.iter_mut().for_each(rename),
            Value::Object(m) => m.values_mut().for_each(rename),
            _ => {}
        }
    }
    // Rename identities only; preserve surface strings and native candidate lemma metadata.
    let binding = &mut renamed["interpretation"]["binding"];
    for r in binding["readings"].as_array_mut().unwrap() {
        r["id"] = json!("local_r1");
        rename(&mut r["frame"]);
        for a in r["bound_usage"]["atoms"].as_array_mut().unwrap() {
            a["atom"]["id"] = json!(format!("local_{}", a["atom"]["id"].as_str().unwrap()));
        }
        for a in r["e8_activations"].as_array_mut().unwrap() {
            a["atom_id"] = json!(format!("local_{}", a["atom_id"].as_str().unwrap()));
        }
    }
    for a in binding["source_packet"]["atoms"].as_array_mut().unwrap() {
        a["atom"]["id"] = json!(format!("local_{}", a["atom"]["id"].as_str().unwrap()));
    }
    let renamed = analyze(&renamed);
    assert_eq!(
        active.observation.partition_hash,
        renamed.observation.partition_hash
    );
}

#[test]
fn role_reversal_negation_and_condition_are_visible_while_mean_and_root_controls_erase_them() {
    let bank = fixture("bank");
    let active = analyze(&bank);
    let mut reverse = bank.clone();
    reverse["interpretation"]["binding"]["readings"][0]["frame"]["events"][0]["roles"][0]["role"] =
        json!("theme");
    reverse["interpretation"]["binding"]["readings"][0]["frame"]["events"][0]["roles"][1]["role"] =
        json!("agent");
    for v in [reverse, fixture("negative"), fixture("conditional")] {
        let other = analyze(&v);
        assert_eq!(
            distance(
                &active.observation.source_vector,
                &other.observation.source_vector
            )
            .unwrap()
            .total_variation,
            0.0
        );
        assert!(
            distance(
                &active.observation.structure_vector,
                &other.observation.structure_vector
            )
            .unwrap()
            .total_variation
                > 0.0
        );
        assert_eq!(native_mean(&bank), native_mean(&v));
        assert_eq!(root_only(&bank), root_only(&v));
        assert_eq!(hashed_bag(&active), hashed_bag(&other));
    }
}
fn native_mean(v: &Value) -> [f64; 8] {
    let mut out = [0.0; 8];
    let a = v["interpretation"]["binding"]["readings"][0]["e8_activations"]
        .as_array()
        .unwrap();
    for row in a {
        for (i, x) in row["placement"]["position8"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            out[i] += x.as_f64().unwrap() / a.len() as f64;
        }
    }
    out
}
fn root_only(v: &Value) -> Vec<String> {
    let mut out: Vec<_> = v["interpretation"]["binding"]["readings"][0]["e8_activations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["placement"]["root_id"].as_str().unwrap().to_owned())
        .collect();
    out.sort();
    out
}
fn hashed_bag(s: &State) -> [f64; 8] {
    let mut out = [0.0; 8];
    for f in &s.observation.source_vector {
        let index = f
            .key
            .bytes()
            .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize))
            % 8;
        out[index] += f.mass;
    }
    out
}

#[test]
fn occurrences_and_alternative_partitions_survive_unit_normalization() {
    let input = fixture("bank");
    let state = analyze(&input);
    let mut duplicated = input.clone();
    let mut second = duplicated["interpretation"]["binding"]["readings"][0].clone();
    second["id"] = json!("r2");
    second["frame"]["id"] = json!("r2");
    duplicated["interpretation"]["binding"]["readings"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let repeated = analyze(&duplicated);
    assert_eq!(
        distance(
            &state.observation.source_vector,
            &repeated.observation.source_vector
        )
        .unwrap()
        .total_variation,
        0.0
    );
    assert_ne!(
        state.observation.partition_hash,
        repeated.observation.partition_hash
    );
    assert_eq!(
        repeated.observation.atom_count,
        2 * state.observation.atom_count
    );
    assert_eq!(repeated.observation.reading_count, 2);
}

#[test]
fn continuity_requires_matching_scope_and_validated_content_not_an_old_claimed_hash() {
    let input = fixture("bank");
    let first = analyze(&input);
    let second =
        analyze_joe_result(&fixture("negative"), Some(&first), &Config::default()).unwrap();
    assert_eq!(second.continuity.status, "compared");
    assert!(second.continuity.partition_changed.unwrap());
    assert!(
        second
            .continuity
            .structure_distance
            .unwrap()
            .total_variation
            > 0.0
    );
    for field in ["threadId", "model", "provider", "language"] {
        let mut changed = input.clone();
        changed[field] = json!("changed");
        if field == "language" {
            changed["interpretation"]["binding"]["request"]["language"] = json!("changed");
        }
        let state = analyze_joe_result(&changed, Some(&first), &Config::default()).unwrap();
        assert_eq!(state.continuity.status, "reset-incompatible");
    }
    let mut corrupt = first.clone();
    corrupt.continuity.source_mixture[0].mass = 0.9;
    assert!(analyze_joe_result(&input, Some(&corrupt), &Config::default()).is_err());
    let config = Config {
        retention: 0.2,
        ..Config::default()
    };
    let state = analyze_joe_result(&input, Some(&first), &config).unwrap();
    assert_eq!(state.continuity.status, "reset-incompatible");
}

#[test]
fn finite_large_and_subnormal_vectors_keep_direction_or_reject_explicitly() {
    assert!((stable_norm(&[1e300; 8]).unwrap() / 1e300 - (8f64).sqrt()).abs() < 1e-14);
    assert!(stable_norm(&[1e-300; 8]).unwrap() > 0.0);
    assert!(stable_norm(&[f64::MAX; 8]).is_err());
    assert!(stable_norm(&[f64::NAN; 8]).is_err());
    let mut c = Config {
        retention: f64::NAN,
        ..Config::default()
    };
    assert!(c.validate().is_err());
    c.retention = -0.1;
    assert!(c.validate().is_err());
    c.retention = 0.5;
    c.max_features = 0;
    assert!(c.validate().is_err());
}

#[test]
fn exact_js_conventions_zero_mass_and_symmetric_identity_support() {
    let a = vec![Feature {
        key: "a".into(),
        mass: 1.0,
    }];
    let b = vec![Feature {
        key: "b".into(),
        mass: 1.0,
    }];
    assert_eq!(distance(&a, &a).unwrap().jensen_shannon_distance, 0.0);
    assert_eq!(distance(&a, &b).unwrap().jensen_shannon_distance, 1.0);
    assert_eq!(distance(&a, &b).unwrap().total_variation, 1.0);
    let c = vec![
        Feature {
            key: "a".into(),
            mass: 0.5,
        },
        Feature {
            key: "b".into(),
            mass: 0.5,
        },
    ];
    assert!((distance(&a, &c).unwrap().jensen_shannon_distance - 0.5579230452841438).abs() < 1e-14);
    assert_eq!(
        distance(&a, &c).unwrap().total_variation,
        distance(&c, &a).unwrap().total_variation
    );
}

#[test]
fn actual_distinct_source_concepts_can_share_a_root_without_collapsing_identity() {
    let p: Value =
        serde_json::from_str(include_str!("fixtures/source-root-collision.json")).unwrap();
    let a = &p["records"][0];
    let b = &p["records"][1];
    assert_eq!(a["placement"]["root_id"], b["placement"]["root_id"]);
    assert_ne!(a["placement"]["concept_id"], b["placement"]["concept_id"]);
    assert_ne!(a["placement"]["position8"], b["placement"]["position8"]);
    let source = |v: &Value| {
        vec![Feature {
            key: format!(
                "source:{}:{}",
                v["sense"]["id"].as_str().unwrap(),
                v["placement"]["concept_id"].as_str().unwrap()
            ),
            mass: 1.0,
        }]
    };
    assert_eq!(
        distance(&source(a), &source(b)).unwrap().total_variation,
        1.0
    );
    let root = |v: &Value| {
        vec![Feature {
            key: v["placement"]["root_id"].as_str().unwrap().into(),
            mass: 1.0,
        }]
    };
    assert_eq!(distance(&root(a), &root(b)).unwrap().total_variation, 0.0);
}

#[test]
fn mutation_wrong_coordinate_convention_or_material_receipt_fails_closed() {
    let input = fixture("bank");
    for mutate in [0, 1, 2, 3] {
        let mut changed = input.clone();
        match mutate {
            0 => changed["authority"]["approvalsGranted"] = json!(true),
            1 => {
                changed["interpretation"]["binding"]["readings"][0]["e8_activations"][0]
                    ["placement"]["position8"][0] = json!(99.0)
            }
            2 => {
                changed["interpretation"]["binding"]["request"]["shared_reference"]["geometry"]
                    ["version"] = json!("other/v1")
            }
            _ => {
                changed["interpretation"]["binding"]["readings"][0]["bound_usage"]["atoms"][0]
                    ["atom"]["span"] = json!([0, 2])
            }
        }
        assert!(analyze_joe_result(&changed, None, &Config::default()).is_err());
    }
}

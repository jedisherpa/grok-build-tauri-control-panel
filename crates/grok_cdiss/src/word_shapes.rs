//! Traceable display chains derived from a validated native proposal.
//! Layout and grammar arrows are declared display encodings, not E8 operators.
use crate::{analyze_joe_result, Config};
use serde_json::{json, Value};

const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const ALTERNATIVE_LIMIT: usize = 8192;

/// Preserve source alternatives and native geometry while exposing occurrence use.
/// No source mapping, placement, interpreter authority or semantic inference is added.
pub fn word_shapes(result: &Value) -> Result<Value, String> {
    let validated =
        analyze_joe_result(result, None, &Config::default()).map_err(|e| e.to_string())?;
    let sentence = result["sentence"].as_str().ok_or("sentence missing")?;
    let binding = &result["interpretation"]["binding"];
    let inventory = binding["source_packet"]["atoms"]
        .as_array()
        .ok_or("inventory missing")?;
    let readings = binding["readings"].as_array().ok_or("readings missing")?;
    let alternative_count = inventory.iter().try_fold(0usize, |sum, atom| {
        sum.checked_add(
            atom["candidates"]
                .as_array()
                .ok_or("candidates missing")?
                .len(),
        )
        .ok_or("alternative count overflow")
    })?;
    if alternative_count.saturating_mul(readings.len()) > ALTERNATIVE_LIMIT {
        return Err(
            "word shape alternative budget exceeded; split passage without dropping alternatives"
                .into(),
        );
    }
    let tokens = lexical_tokens(sentence);
    let mut coverage = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let mut by_reading = Vec::new();
        for reading in readings {
            let atoms = reading["bound_usage"]["atoms"]
                .as_array()
                .ok_or("atoms missing")?;
            let atom_ids: Vec<_> = atoms
                .iter()
                .filter(|atom| contains_span(&atom["atom"]["span"], token.start, token.end))
                .map(|atom| atom["atom"]["id"].clone())
                .collect();
            by_reading.push(json!({"readingId":reading["id"], "covered":!atom_ids.is_empty(),"atomIds":atom_ids}));
        }
        coverage.push(
            json!({"surface":token.surface,"span":[token.start,token.end],"position":index+1,
            "covered":by_reading.iter().all(|r|r["covered"]==true),"readings":by_reading}),
        );
    }
    let mut out_readings = Vec::new();
    for reading in readings {
        let mut occurrences = Vec::new();
        for atom in reading["bound_usage"]["atoms"]
            .as_array()
            .ok_or("atoms missing")?
        {
            let aid = &atom["atom"]["id"];
            let source = inventory
                .iter()
                .find(|p| p["atom"]["id"] == *aid)
                .ok_or("source atom missing")?;
            let selected = atom["selected_source_bindings"]
                .as_array()
                .ok_or("selected bindings missing")?;
            let mut alternatives = Vec::new();
            for candidate in source["candidates"]
                .as_array()
                .ok_or("candidates missing")?
            {
                let sid = &candidate["sense"]["id"];
                let is_selected = selected.contains(candidate);
                let activations: Vec<_> = reading["e8_activations"]
                    .as_array()
                    .ok_or("activations missing")?
                    .iter()
                    .filter(|a| a["atom_id"] == *aid && a["sense_id"] == *sid)
                    .cloned()
                    .collect();
                let mut dictionary = candidate["sense"].clone();
                let definition_present = dictionary["definition"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty());
                dictionary["definitionStatus"] = json!(if definition_present {
                    "source-definition"
                } else {
                    "missing"
                });
                let alignments = candidate["alignments"]
                    .as_array()
                    .ok_or("source alignments missing")?;
                if alignments.len() > 512 {
                    return Err("source alignment budget exceeded".into());
                }
                alternatives.push(json!({"senseId":sid,"selected":is_selected,"conceptIds":candidate["concept_ids"],
                    "dictionary":dictionary,"bindingKind":candidate["binding_kind"],"sourceForm":candidate["source_form"],"posComparison":candidate["pos_comparison"],
                    "crossLanguage":{"status":if alignments.is_empty(){"missing"}else{"source-alignment"},
                        "alignments":alignments,"counterpartsStatus":"not-in-native-packet",
                        "meaning":"attributed source concept alignment; bilingual equivalence is not newly inferred"},
                    "e8":{"status":if !is_selected{"not-selected"}else if activations.iter().any(|a|a["placement"].is_object()){"native-placement"}else{"missing"},
                        "activations":activations,"coordinatesChanged":false}}));
            }
            let atom_span = &atom["atom"]["span"];
            let start = atom_span[0].as_u64().ok_or("span start missing")? as usize;
            let end = atom_span[1].as_u64().ok_or("span end missing")? as usize;
            let positions: Vec<_> = tokens
                .iter()
                .enumerate()
                .filter(|(_, t)| t.start < end && start < t.end)
                .map(|(i, _)| i + 1)
                .collect();
            let roles = role_bindings(&reading["frame"], aid)?;
            occurrences.push(json!({"atomId":aid,"surface":atom["atom"]["surface"],"span":atom_span,
                "tokenPosition":positions.first(),"tokenPositions":positions,"sentenceTokenCount":tokens.len(),
                "roleBindings":roles,"alternatives":alternatives,"senseSnap":atom["sense_snap"],
                "usageOrientation":{"schema":"bomb-code/grammar-display-orientation/v1",
                    "status":if roles.is_empty(){"unassigned"}else{"declared-role-encoding"},
                    "basis":"fixed 2D display angles by proposed role; positive degrees clockwise from right; no E8 rotation",
                    "arrows":roles,"eventLinks":reading["frame"]["event_links"],"references":reading["frame"]["references"],
                    "semanticInferenceValidated":false,"nativeGeometryRotated":false},
                "selectionReason":atom["selection_reason"],"surfaceUnmapped":atom["surface_unmapped"]}));
        }
        out_readings.push(
            json!({"readingId":reading["id"],"occurrences":occurrences,"frame":reading["frame"],
            "uncertainty":reading["selection_uncertainty"]}),
        );
    }
    let covered = coverage.iter().filter(|t| t["covered"] == true).count();
    let output = json!({"schema":"bomb-code/word-shapes/v1","status":"ready","sentence":sentence,"language":result["language"],
        "basis":{"source":validated.basis,"evidenceHash":validated.evidence_hash,
            "stages":["dictionary","crossLanguage","senseSnap","sentenceUse"],
            "layout":"four display markers joined by directed provenance edges; marker positions are not a shared semantic coordinate basis",
            "geometry":"unchanged native activation vectors/root/residual; root collisions retain sense and concept identity",
            "orientationRule":{"predicate":0,"agent":45,"theme":135,"patient":135,"recipient":90,"experiencer":225,"instrument":270,"location":315,"other":null}},
        "tokenCoverage":{"tokenizer":"Unicode alphanumeric runs with combining marks and internal apostrophes; heuristic, not linguistic segmentation",
            "spanUnit":"Unicode scalar values; half-open","meaning":"covered means full token span retained in atom inventory; source selection and E8 fitting are separate statuses","sentenceTokenCount":tokens.len(),"coveredTokenCount":covered,
            "uncoveredTokenCount":tokens.len()-covered,"punctuationExcluded":true,"tokens":coverage},
        "readings":out_readings,"authority":{"toolsDispatched":false,"approvalsGranted":false,"memoryCommitted":false}});
    if serde_json::to_vec(&output)
        .map_err(|e| e.to_string())?
        .len()
        > OUTPUT_LIMIT
    {
        return Err(
            "word shape output exceeds 8 MiB; split passage without dropping alternatives".into(),
        );
    }
    Ok(output)
}

fn contains_span(span: &Value, start: usize, end: usize) -> bool {
    span[0].as_u64().is_some_and(|s| s <= start as u64)
        && span[1].as_u64().is_some_and(|e| e >= end as u64)
}

fn role_bindings(frame: &Value, aid: &Value) -> Result<Vec<Value>, String> {
    let mut output = Vec::new();
    for event in frame["events"].as_array().ok_or("events missing")? {
        let mut roles: Vec<&str> = event["roles"]
            .as_array()
            .ok_or("roles missing")?
            .iter()
            .filter(|r| r["atom_id"] == *aid)
            .filter_map(|r| r["role"].as_str())
            .collect();
        if event["predicate"] == *aid {
            roles.insert(0, "predicate");
        }
        for role in roles {
            output.push(json!({"eventId":event["id"],"role":role,"angleDegrees":role_angle(role),
                "polarity":event["polarity"],"modality":event["modality"],"cueSpans":event["cue_spans"],
                "authority":"proposed grammatical role, not dictionary fact"}));
        }
    }
    Ok(output)
}

fn role_angle(role: &str) -> Option<u16> {
    match role {
        "predicate" => Some(0),
        "agent" => Some(45),
        "theme" | "patient" => Some(135),
        "recipient" => Some(90),
        "experiencer" => Some(225),
        "instrument" => Some(270),
        "location" => Some(315),
        _ => None,
    }
}

struct Token {
    surface: String,
    start: usize,
    end: usize,
}
fn combining(c: char) -> bool {
    matches!(c as u32,0x0300..=0x036f|0x1ab0..=0x1aff|0x1dc0..=0x1dff|0x20d0..=0x20ff|0xfe20..=0xfe2f)
}
fn lexical_tokens(sentence: &str) -> Vec<Token> {
    let chars: Vec<char> = sentence.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < chars.len()
            && (chars[i].is_alphanumeric()
                || combining(chars[i])
                || (matches!(chars[i], '\'' | '’')
                    && i + 1 < chars.len()
                    && chars[i + 1].is_alphanumeric()))
        {
            i += 1;
        }
        tokens.push(Token {
            surface: chars[start..i].iter().collect(),
            start,
            end: i,
        });
    }
    tokens
}

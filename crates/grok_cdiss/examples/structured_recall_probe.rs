//! Bounded stdin/stdout native probe; no provider, filesystem or memory APIs.
use grok_cdiss::retrieval::{retrieve_structured, RetrievalLimits, VerifiedRetrievalInput};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Write};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    id: String,
    packet: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    query: Value,
    candidates: Vec<Candidate>,
    #[serde(default)]
    limits: RetrievalLimits,
}
fn run() -> Result<Value, String> {
    let maximum = 32 * 1024 * 1024;
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > maximum {
        return Err(
            "aggregate probe input exceeds 32 MiB; split without dropping alternatives".into(),
        );
    }
    let input: Input =
        serde_json::from_slice(&bytes).map_err(|_| "invalid structured probe JSON".to_string())?;
    input.limits.validate()?;
    if input.candidates.len() > input.limits.max_candidates {
        return Err("candidate budget exceeded".into());
    }
    let query = VerifiedRetrievalInput::from_native(&input.query)?;
    let candidates = input
        .candidates
        .iter()
        .map(|c| {
            Ok((
                c.id.clone(),
                VerifiedRetrievalInput::from_native(&c.packet)?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let retrieval = retrieve_structured(&query, &candidates, &input.limits)?;
    let retrieval_bytes = serde_json::to_vec(&retrieval)
        .map_err(|e| e.to_string())?
        .len();
    let profile_bytes = candidates
        .iter()
        .try_fold(query.audit_size_upper_bound(), |sum, (_, p)| {
            sum.checked_add(p.audit_size_upper_bound())
        })
        .ok_or("probe audit byte overflow")?;
    if profile_bytes
        .checked_add(retrieval_bytes + 16384)
        .ok_or("probe output byte overflow")?
        > maximum
    {
        return Err("complete probe audit/profile envelope exceeds 32 MiB; split without discarding source alternatives".into());
    }
    Ok(
        json!({"schema":"bomb-code/structured-retrieval-probe/v1","status":"ready",
        "queryProfile":query.audit_profile(),
        "candidateProfiles":candidates.iter().map(|(id,p)|json!({"candidateId":id,"profile":p.audit_profile()})).collect::<Vec<_>>(),
        "retrieval":retrieval,"authority":{"toolsDispatched":false,"approvalsGranted":false,"memoryCommitted":false}}),
    )
}
fn main() {
    let result = run().and_then(|value| {
        let encoded = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        if encoded.len() > 32 * 1024 * 1024 {
            return Err(
                "complete probe output exceeds 32 MiB; split without dropping evidence".into(),
            );
        }
        Ok(encoded)
    });
    match result {
        Ok(encoded) => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(&encoded).unwrap();
            stdout.write_all(b"\n").unwrap();
        }
        Err(error) => {
            println!(
                "{}",
                json!({"schema":"bomb-code/structured-retrieval-probe/v1","status":"unavailable","reason":error,
            "authority":{"toolsDispatched":false,"approvalsGranted":false,"memoryCommitted":false}})
            );
            std::process::exit(1);
        }
    }
}

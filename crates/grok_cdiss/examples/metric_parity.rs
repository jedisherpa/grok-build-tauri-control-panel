//! Deterministic numeric cases for independent SciPy parity verification.
use grok_cdiss::{distance, Feature};
use serde_json::json;
fn main() {
    let mut seed = 20261006u64;
    let mut vector = || {
        let raw: Vec<_> = (0..16)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let n = (seed >> 32) % 1001;
                if n.is_multiple_of(4) {
                    0.0
                } else {
                    n as f64
                }
            })
            .collect();
        let total: f64 = raw.iter().sum();
        raw.into_iter()
            .enumerate()
            .filter(|(_, v)| *v > 0.0)
            .map(|(n, v)| Feature {
                key: format!("k{n:02}"),
                mass: v / total,
            })
            .collect::<Vec<_>>()
    };
    let rows: Vec<_> = (0..128)
        .map(|_| {
            let p = vector();
            let q = vector();
            let metrics = distance(&p, &q).unwrap();
            json!({"p":p,"q":q,"metrics":metrics})
        })
        .collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}

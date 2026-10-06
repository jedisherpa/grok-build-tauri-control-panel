//! Read probability pairs from stdin and emit public CDISS distances for external parity checks.
use grok_cdiss::{distance, Feature};
use serde_json::Value;
use std::io::{self, Read};
fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let pairs: Value = serde_json::from_str(&input).unwrap();
    let out: Vec<_> = pairs
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            let vector = |v: &Value| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .filter_map(|(i, mass)| {
                        let mass = mass.as_f64().unwrap();
                        (mass > 0.0).then(|| Feature {
                            key: format!("identity:{i:04}"),
                            mass,
                        })
                    })
                    .collect::<Vec<_>>()
            };
            distance(&vector(&pair[0]), &vector(&pair[1])).unwrap()
        })
        .collect();
    println!("{}", serde_json::to_string(&out).unwrap());
}

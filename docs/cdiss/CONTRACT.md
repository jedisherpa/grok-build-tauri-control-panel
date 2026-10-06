# CDISS source and structure contract

The Bomb Code adapter tracks proposed readings in a product representation:

1. A finite sparse source-identity measure, whose keys retain **sense ID and concept ID**. Context pins have a separate namespace; missing source mappings remain explicit. Each atom occurrence supplies one descriptive unit, split equally across its retained alternatives. This allocation is not calibrated probability, confidence, approval, progress or human intent.
2. A finite sparse structure measure of declared events, roles, polarity, modality, scoped cue contents, references and event links. Local model IDs and character positions do not define cross-passage identity. Canonical event keys keep role membership and source alternatives together. Active/passive declarations with the same roles compare equally; role reversal, negation and conditions remain distinct.
3. Raw role-labelled atomic E8 measures: the native frames refer to atom IDs, and native activation objects retain every fine coordinate, root, radius, residual and full lattice address. No vector average replaces a reading. SenseSnap contextual centers remain separately typed. Equal coarse roots do not merge source records.

The observation also retains occurrence counts, alternative partitions, reading partitions, exact Unicode spans, native frames, all candidate source IDs and uncertainty. Its partition digest detects repeated occurrences and additional alternatives even when a normalized histogram would be unchanged. The full source inventory remains in the native Joe receipt; the adapter never modifies it.

For compatible, explicitly selected previous reviews, the deterministic state update is

`h_new = retention * h_previous + (1 - retention) * observation_current`.

The default retention is 0.5. This is a declared bookkeeping weight; it has no learned calibration. TV is `0.5 * sum(abs(p-q))`. JS distance is `sqrt(0.5*KL_2(p||m) + 0.5*KL_2(q||m))`, where `m=(p+q)/2`; zero terms contribute zero. Both distances use the union of exact declared keys. Distances compare raw successive observations; mixtures are presented separately. There is no weighted gate, completion scalar, emotion field, persona prior, learned embedding, random projection or byte-hash semantic fallback.

Compatibility requires the same nonempty thread, language, declared context digest, provider/model provenance, manifest, graph, fitted-model snapshot, full shared-reference digest (including SenseSnap implementation hashes), numeric geometry convention, algorithm and validated configuration. Changes reset comparison with an explicit reason. Requests without a thread cannot carry continuity. Exact input hashes, source evidence hashes, configuration digests, prior-state digests and recomputed state digests support replay and invalidation. Digests identify content; they are not authentication or human approval.

The boundary rejects failed or unavailable interpretations, foreign schemas, missing read-only authority receipts, unsupported E8 conventions, mismatched source pins, missing inventory atoms, out-of-inventory selections, malformed Unicode spans, orphan geometry, invalid or overflowing radii and inconsistent root/lattice reconstruction. Float64 norms scale by the maximum absolute component before squaring. The actual E8 root set is generated in the pinned lexicographic doubled-coordinate convention; lattice doubled coordinates must have common parity and sum divisible by four.

Budgets reject whole inputs rather than truncate alternatives: 32 MiB native result, 64 readings, 64 atoms per reading, 512 candidates or centers per atom, 64 events/roles/links/references per reading, 4096 activations per reading, 8192 sparse keys per channel, 64 KiB per structural key and 4 MiB total key bytes per channel. Only one prior state digest is retained, so state documents do not accumulate recursive history. The host is responsible for trusted native receipt retrieval and limiting concurrent requests; user-supplied JSON with an internally consistent digest is not an authenticated native receipt.

Geometry is inspected rather than trained here. No moments or transport ground cost are introduced: unrelated fitted training components can have uncalibrated relative orientation, and a centroid discards occurrence structure, unknowns and competing readings. Optimal transport is deferred until a qualified ground cost and benchmark show benefit beyond exact source identity and structure matching. A future entity/coreference layer would need explicitly supported referent identities; dictionary concept identity alone cannot establish that two occurrences name the same real entity.

# Research and verification

Primary sources:

- [SciPy's JS distance documentation](https://docs.scipy.org/doc/scipy/reference/generated/scipy.spatial.distance.jensenshannon.html) specifies square-root divergence, zero-support behavior and base-dependent normalization. Our implementation explicitly uses base 2 and requires prevalidated normalized vectors.
- [Singh et al., Context Mover's Distance, AISTATS 2020](https://proceedings.mlr.press/v108/singh20a.html) motivates retaining distributions over contexts rather than only point embeddings. Its reported gains belong to its own tasks and qualified transport model; this adapter does not inherit those accuracy claims.
- [AIM's E8 root-system description](https://aimath.org/E8/mcmullen.html) describes 240 roots in eight dimensions. An arbitrary eight-vector is not a source-calibrated semantic point.
- [Representation biases for compositional relations](https://aclanthology.org/2023.eacl-main.268/) and [negation/implicature limitations](https://aclanthology.org/2024.acl-long.33/) motivate testing role, negation and modality preservation explicitly; they do not validate our authored readings.

`generate_fixtures.py` runs the unchanged native binder against the pinned actual graph/model. Five full fixtures are included, labelled authored grammar and source choices, with zero provider calls. The loan noun is deliberately unavailable in this scoped source inventory; the adapter retains it as unmapped instead of substituting an available loan verb. A separate collision fixture contains two actual distinct source concepts, both labelled `able`, sharing E8 root 195 but retaining different fine coordinates and source sense IDs.

`cargo test -p grok_cdiss` checks actual native coordinate retention, active/passive and ID-renaming invariance, role reversal, negation, condition, root collisions, occurrence counts, incompatible-scope reset, current-state integrity, invalid configurations, stable large/subnormal norms, JS convention and material-receipt rejection. Unstructured coordinate means, root-only indexes and a hash of the unordered source bag erase role/negation/condition changes in these fixtures. This demonstrates information retention on authored boundary cases; it does not measure live language accuracy, psychometric intent or agreement.

`cargo run -p grok_cdiss --release --example research_probe` emits measured CPU timings and fixture distances. `fixtures/MANIFEST.json` pins the included evidence files. The native source package and existing E8/SenseSnap code remain unchanged.

The fifth fixture verifies native fitted public and unfitted private SenseSnap context pins. Their coordinates and origin remain distinct from source sense selections, and changed declared context resets comparison. Independent SciPy base-2 JS and NumPy TV checks cover 35 standard cases and 128 additional deterministic sparse cases; the largest absolute discrepancy is 2.23e-16. Unicode host limits, retention endpoints, and subnormal aggregate numeric behavior have dedicated regressions.

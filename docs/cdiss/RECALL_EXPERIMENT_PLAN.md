# Local word-shape recall experiment

Paul requested tests of four proposed benefits: intended-sense recall, differently worded and cross-language references, sentence relationships, and explanations of matches. This experiment compares fixed local retrieval methods against the existing Bomb Code hybrid baseline. It does not change the installed ranking, native app, archived conversations or memory index. Private targets, passages, rankings and source receipts remain outside Git.

## What constitutes evidence

Three results are reported separately:

1. **Mechanical fidelity:** the implementation retrieves the declared candidates, preserves source identity and scope, computes the frozen scores correctly, and explains its actual scoring contributions. This is testable without relevance judgments.
2. **Known-target retrieval:** a fixed agent-authored query retrieves its independently selected historical source message or excerpt. This is exploratory retrieval evidence on these cases. The target is one known positive, not an exhaustive relevance judgment; an unlabelled result is not necessarily irrelevant.
3. **Human utility:** Paul judges whether a match answers the intended question and whether its explanation is helpful. This remains unmeasured until reviewed. Agent labels, dictionary identities and geometric proximity cannot supply human intention or approval.

The complete set and all failures must be reported. Native boundary controls and multilingual dictionary lookups do not count as historical retrieval successes.

## Freeze and separation

Before ranking, freeze a versioned method configuration, its canonical SHA-256, the implementation hash, source snapshot identities and query/target manifest hash. The target-authoring agent may read original historical messages but must not read retrieval outputs. The method implementer must not inspect target queries before the configuration is frozen. The reviewer must not change expansion limits, role rules or weights after seeing results. Corrections to a genuine implementation bug require a new configuration/run identity, preserved failed run and disclosed reason; any tuned version needs new held-out cases.

Use 24–36 retrieval cases distributed across the first three families and multiple sources/domains where eligible stored history permits. The fourth family audits explanations for every query and method rather than supplying a separate target family. Query wording, target message/chunk identity, target excerpt/hash, family and eligibility reason are fixed before ranking. Avoid direct copied-query fragments when testing paraphrase, and record lexical overlap to make the difficulty visible. A query authored after reading its target is an artificial recall task even when independently authored; do not call it natural user-query accuracy.

## Baseline and ablations

The actual `scripts/memory_recall.py` baseline is SQLite FTS5 `unicode61` BM25 over OR-joined query terms, lexical top 100, and local Nomic cosine top 100 among model-matched vectors with positive similarity. Vector scanning has a fixed 50,000-candidate cap and deterministic chunk-ID order. Fusion sums `1/(60 + rank)` for each available channel and resolves score ties by chunk ID. The existing topic filter is an AND of literal case-folded substrings; it is not domain classification. Keep the corpus, filters, candidate budgets, query model digest/prefix/normalization and stable tie rules identical across paired methods. Query embeddings use only the existing loopback service; no new interpretation-provider calls or cloud fallback are needed.

Report lexical-only, vector-only and exact hybrid results. Candidate generation and reranking are different interventions: show whether an added method finds a target absent from the baseline candidate union or merely improves its order.

Additional methods are frozen, query-only dictionary expansion; independently declared lexical sense/context scoring; transparent relationship/scope heuristics; and a separate source-geometry ablation. The geometry ablation compares means of dictionary-derived source vectors: it deliberately discards occurrence structure and serves as a weak control, not a proposed new shape encoding. Never select an expansion or a source sense because the target has that identity. Ambiguous dictionary alternatives stay alternatives. Equivalence asserted by the frozen source is distinguished from candidate alignments or a shared concept label. No E8-root hit is automatically a synonym. Missing definitions, missing placements, missing vectors and unavailable frame evidence remain separate coverage statuses.

## Metrics and exploratory decisions

At chunk level report Hit@5, Hit@10, target reciprocal rank truncated at 10, and full candidate-union coverage. At original-message level collapse chunks to their first rank and report the same metrics, since 1,000-character chunks from one message must not inflate distinct relevant-message counts. When multiple target spans are accepted, freeze that list before evaluation. Report paired gain/loss/tie counts, per-family denominators and individual failures. Do not present precision, nDCG or false-positive rates without complete corresponding judgments.

The exploratory retrieval gate is an absolute mean reciprocal-rank-at-10 gain of at least 0.05 against exact hybrid in that family, with no decrease in its Hit@10. Also disclose harmed cases. A method below this gate is mixed or unsupported on this sample, even when a memorable example improves. A small convenience sample and repeated use of the same targets cannot establish population accuracy or statistical confirmation. Geometry is judged by its incremental result beyond the otherwise identical source-identity/structure method, not by its association with successful passages.

Record cold setup time separately from per-query elapsed time, paired query runtime, p50/p95 and peak working memory when available. Warm-cache reused embeddings must be identified. Report index chunk/message counts, exclusions, matching-model vector coverage, semantic scan limitation and target eligibility. Excluded or unavailable targets are accounted for outside the denominator; a target present in the declared corpus but missed by a method remains a miss.

## Four families

| Hypothesis | Local experiment | Required distinction |
| --- | --- | --- |
| Intended sense | Domain-qualified or ambiguous keyword questions paired with an independently selected historical target; compare literal hybrid against fixed dictionary/context expansion. | Improvement must be retrieval rank, not merely discovery of a dictionary entry. Dictionary candidate scoring is not validated word-sense disambiguation. |
| Different wording/language | Historical paraphrase targets with measured lexical overlap, plus source-backed multilingual equivalence controls where actual corpus-language coverage is insufficient. | Report historical paraphrase retrieval separately from multilingual mapping mechanics; candidate alignments do not qualify translation equivalence. |
| Relationships and scope | Historical questions involving who did what, positive/negative statements, conditions or corrections; compare frozen surface heuristics. Separately run native CDISS bank/negative/conditional/passive/role-reversal controls. | Surface negation cues cannot be called scope parsing or validated event extraction. Authored frames qualify information retention only. |
| Explanation | Emit exact query-term/concept/relationship/geometry contributions for each ranked hit, cited source spans and explicit unavailable features; verify recomputation and source hashes. | A faithful explanation of a heuristic is not necessarily a correct interpretation. Human comprehension/helpfulness remains pending. |

## Native CDISS and saved Joe controls

The current contract retains separate sparse source identity and canonical structure measures. The native authored fixtures include active bank approval, passive equivalent, negative approval, conditional approval, and an independent SenseSnap context-pin example. Assert active/passive structure equality; role reversal, negation and condition distinction; selected-source IDs and original E8 activations remain exact; context pins stay separately typed. Reconstruct the word-shape attachment using the actual native validator and preserve role polarity, modality, cue spans and orientation declarations. Role arrows are display encodings and do not rotate the native E8 coordinates.

The controlled retrieval probe compares a deduplicated selected-sense/role/polarity/modality signature with actual CDISS structure equality only in five authored single-event controls. Its signature omits general event membership, multiplicity, links and cue contents; successful equality agreement here must not be extended to arbitrary multi-event passages.

At plan time the private receipt inventory contains seven Joe receipts: five have bound interpretations with fourteen retained readings total, and two have no successful binding. Successful readings contain five to eighteen atoms and one to three events. Some have no fitted E8 activations. Their original receipts predate stored word-shape attachments. Local replay can test exact retained source/frame evidence and reproducibility without a provider call; these saved proposals are not independent gold labels. Verify current/saved manifest, graph/model and SenseSnap implementation pins before replay, and verify the original receipt and memory evidence bindings unchanged afterward. Failed receipts stay failed rather than being silently reinterpreted.

The previous imported-history mechanical benchmark used exact-surface dictionary inventories with empty frames. Its zero structure distances do not qualify role, negation, modality or completion accuracy. A new heuristic frame is a separate experimental feature with an explicit basis, not a retroactive validation of that benchmark.

## Preservation and reporting

Open original archives, index and receipts read-only. Write only private experiment artifacts in a directory with mode 0700, files mode 0600. Pin original file hashes and SQLite-consistent identities before/after; record model basis and frozen reference pins. Experiment ranking cannot dispatch tools, grant permissions, commit memories or send archived content to a provider. Explanation citations must identify the original message, precise Unicode scalar span and hashes, and stale source validation must fail closed.

The final report gives all four outcomes with their evidence type, baseline/ablation tables, harmed cases, coverage gaps and remaining human review. It may recommend a candidate feature after local evidence, but should not activate a new installed ranking solely from these exploratory tests.

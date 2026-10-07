# Four recall theories: local experiment, 6 October 2026

The native representation preserves useful authored distinctions, and match receipts are mechanically faithful. The fixed dictionary/context and sentence-cue proxies did **not** improve recall on this small imported-history benchmark. No production ranking change is justified by this run.

Research branch: `codex/word-shape-recall-experiment`, isolated from installed0.1.12 source `d451e1d6a796ce16304e2ce2f63f8d351b0ba9cd`. All scripts and the native example are experimental tooling; the installed application and its search implementation were not changed.

## Four outcomes

| Theory | Test | Result | What this establishes |
| --- | --- | --- | --- |
| Sense/domain improves keyword recall | Eight independently authored archive queries; frozen definition/context proxy | Target top10:7/8 for both methods; MRR@10:0.7125 for both | No improvement from this proxy; wrong-sense reduction unmeasured |
| Other wording/cross-language links improve recall | Eight archive paraphrase queries; six distinct-concept bilingual controls | Archive top10:2/8→1/8; MRR@10:0.0875→0.015625. Bilingual targets:0/6 literal→4/6 expanded | Source-link expansion mechanics work on controlled linked forms; actual archive paraphrase ranking worsened |
| Sentence roles/scope improve matching | Eight archive queries plus five authored native frames | Archive top10:8/8 for both; MRR@10:0.9375 for both. Native frames retain active/passive equivalence and separate negation, condition and reversed roles | Native information retention works on supplied frames; the restricted sentence heuristic showed no retrieval gain |
| Explain why each memory matched | Top10 receipts for24 queries×6methods; independent saved arithmetic/citation audit | 1,440 receipts verified,339 distinct excerpts; no arithmetic/citation mismatch | Source and score traceability works; human explanation usefulness still requires review |

## All24 archive cases

| Method | Target top5 | Target top10 | Chunk MRR@10 | Message MRR@10 |
| --- | ---: | ---: | ---: | ---: |
| Existing hybrid search | 17/24 | 17/24 | 0.579167 | 0.581250 |
| Definition/context proxy | 15/24 | 17/24 | 0.541071 | 0.544147 |
| Source-link expansion | 15/24 | 16/24 | 0.557292 | 0.557292 |
| Sentence-cue proxy | 14/24 | 14/24 | 0.541667 | 0.545833 |
| Mean E8 position proxy | 17/24 | 17/24 | 0.551389 | 0.553472 |
| All proxies combined | 14/24 | 17/24 | 0.531415 | 0.532407 |

MRR@10 is mean reciprocal rank of the selected known positive, with zero beyond rank10. Each case has one target; additional relevant passages were not exhaustively labelled. Message metrics collapse multiple chunks of the same source/thread/message before ranking. The sentence proxy recovers15/24 target messages in top10, versus14/24 exact chunks; the existing baseline remains17/24. These are target-recovery metrics, not precision or semantic truth.

Across all24 cases, MRR@10 gains/losses/ties versus baseline: definition0/3/21; expansion2/2/20; sentence0/3/21; geometry0/2/22; combined2/5/17. No family passes the preregistered exploratory gate of≥0.05 absolute MRR@10 gain without lower top10 recovery. This small set cannot support population-level or multilingual archive claims.

### Individual-channel supplement

After the primary results, a separate read-only audit reused the same24 frozen queries and native search with only its public result ceiling extended20→200, retaining the full lexical100/vector100 candidate union. No features, weights, labels or frozen primary scripts changed. All24 cases completed with zero errors; original source/reference/model pins remained unchanged. Hybrid top20/top100 IDs match the frozen run, and normalized scores agree within declared numeric tolerance (absolute1e-12, relative1e-9). Independent saved-channel arithmetic also passes.

| Existing channel | Chunk top10 | Chunk MRR@10 | Message top10 | Message MRR@10 | Known-target candidate coverage |
| --- | ---: | ---: | ---: | ---: | ---: |
| Lexical only | 16/24 | 0.543056 | 16/24 | 0.543056 | 16/24 |
| Vector only | 15/24 | 0.454861 | 16/24 | 0.459028 | 19/24 |
| Hybrid100 | 17/24 | 0.579167 | 17/24 | 0.581250 | 20/24 |

Full union200 retains20/24 targets, the same coverage as hybrid100. It is a candidate ceiling ordered by the existing hybrid RRF, not a new semantic method. Four selected targets are absent from both native candidate channels; additional reranking cannot recover those targets. This supplementary baseline comparison closes the original plan's individual-channel reporting requirement.

## Native and bilingual controls

Five single-event source-bound authored packets pass native CDISS/word-shape comparison. Active and passive packets match in selected-sense/role/polarity/modality content; negative, conditional and reversed-role packets separate. All five tie under root-only and source-bag matching. This comparison signature excludes event membership, cues and links, so the result is confined to these supplied single-event frames. An actual root-collision fixture retains distinct source senses and fine coordinates under the same root.

Five successful saved Joe proposals replay into14 readings,62 role bindings and31 native E8 activations. Equal reading/occurrence counts, exact complete frames and full activation multiset equality pass. Current SenseSnap code hashes and shared dictionary/model fixture pins match; numeric lattice scale8 versus8.0 is normalized only for comparison. All seven original files have identical before/after hashes. Two older failed proposals remain unavailable and are not counted as successes. Saved proposals are not independent linguistic gold.

The initial bilingual control recovered6/6 linked forms from **one concept**, and is retained separately. The supplemental control froze six distinct source concepts before evaluating the unchanged interpreter and FTS5 query logic. Expansion recovered4/6 targets across Japanese2, Indonesian2, Malay1 and Mandarin1; literal matching recovered0/6. Both methods retrieved0/6 supplied unrelated controls. All origins were selected; two targets fell outside the fixed12-term expansion budget. The lexicographic expansion budget can exclude later-sorted Japanese/Mandarin forms. These tiny documents exercise source-asserted links, not independently verified translations or actual multilingual archive relevance. Neither control opens the archive/index or uses embeddings/provider calls. Do not combine their denominators.

## Reproducibility and qualification

- Dataset SHA256: `de6827cfa6d01a690762841bcf23f6a4b07f5783cadacbb8137af2f3213c5520`.
- Frozen method/config SHA256: `0bd3f5e47fdb5d1760946fccc4a3807cd5499502596d810ced121b5f942aee7c`.
- Frozen engine SHA256: `6bda6fd8ff6d7744a3358e98c96438a4d4cabf43c0b3007a23ca5e13038a5b1e`.
- Baseline search-function digest: `97634da127e8c1047cd004b4f820aecb9729c324083398d7b2d5dec0b5341a59`.
- Diverse bilingual fixtures SHA256: `a8969afd2a6ac99dd752c5797f705f1b52cb93b5ffb11d8dda74ee8990abac73`.

All24 cases completed with zero exclusions. Cases were authored independently before ranking output was seen; queries are agent-authored known-positive targets awaiting human relevance review. Six cases come from each imported provider, covering21 distinct threads. The ranking function receives query text only. Dataset targets are joined against original source/span/hash metadata before evaluation.

The exact installed search function was reused with only the public result ceiling extended20→100; original top20 parity passed on every query. All29,755 vectors matched the pinned local Nomic model; no pending vectors, scan cap or fallback. Baseline target candidate recall@100 is20/24, which limits reranking-only methods. The scope is keyword plus positive cosine RRF candidate retrieval and fixed bounded dictionary/cue additions, not archive-wide interpreted word shapes. Lexical/vector-only results are supplied by the separately pinned post-primary channel audit above.

Before/after archive, index and reference pins match. Local embedding model identity remains `0a109f422b47e3a30ba2b10eca18548e944e8a23073ee3f3e947efcf3c45e59f`. The native pinned dictionary is reloaded to verify source bytes both before and after the run. No remote provider call, source/index write or installed ranking change occurs. Queries, historical excerpts, case IDs, full rankings and receipts remain in the private experiment output outside Git.

Query coverage:35 selected senses,39 unresolved encoded forms;20 queries have source expansion and20 have geometry. Definition availability and tied sense selection constrain the proxy. Mean fine E8 coordinates collapse occurrence/structure information and are a deliberately weak ablation, not a test of the complete word-shape representation. Wrong-sense negatives are unlabelled: a zero `wrongSenseAt5` in raw aggregates must not be read as measured accuracy; the qualified independent audit records it as unknown.

Median combined workload9.65s; nearest-rank p9510.05s; sum231.93s. This includes all methods, citation checks and a second baseline parity scan using cached local embeddings. It excludes dictionary startup and is not individual-method latency.

## Validation and next experiment

Workspace check and strict all-target Clippy pass. Native CDISS suite20 tests, frozen engine19 tests, case-boundary4 tests and independent citation-audit3 tests pass. Architecture and performance/simplicity reviews end with zero unresolved Critical/High/actionable Medium findings. The initial saved-reference comparison failed because8 differed from8.0; its failure log is preserved and the qualified numeric-only comparison passes. The prior subset-only activation check and permissive citation check were strengthened and re-reviewed.

Retain existing hybrid search. Next, assemble a small independently labelled set with complete source-bound interpreted frames, explicit domains, hard negation/role negatives and language diversity. Evaluate candidate generation separately from reranking, and compare complete occurrence/role/scope representations with geometry as a measured feature. Explain contributions locally while preserving unknowns. This experiment does not promote any proxy into production.

# Imported history benchmark

This benchmark uses actual imported main-thread user messages as private test material. It evaluates the installed CDISS algorithm's handling of exact lexical source identities, all available native dictionary alternatives, explicit unmapped forms, native dictionary-source SenseSnap centers in the retained binding and unchanged E8 activation objects. It makes no provider calls, issues no tool dispatch or approval, and writes no native history, memory or settings.

The selected corpus has equal allocations across ChatGPT, Claude Code, Codex and Grok where eligible messages exist. Thread groups and messages are ordered by SHA-256, then selected cases are ordered chronologically within groups. Subagent threads, source-truncated messages, credential-shaped text, instruction/environment wrappers, current CDISS benchmark prompts, connectivity fixtures and code/tool blocks are excluded. Each case records original thread/source/message IDs, original message hash, exact zero-based half-open Python Unicode character offsets, original and excerpt lengths, input hash and selection reason. Passages are capped at 500 characters and 48 Unicode words by an explicit leading excerpt; longer original remainders remain excluded and recorded rather than silently overwritten. The private corpus is never added to Git.

The native reference is warm-loaded once. Its actual unchanged binder gets a transparent authored outline: every Unicode word is an exact-span atom, part of speech is unknown, no lemma is guessed, all exact-surface dictionary senses are retained, and events/roles/references are deliberately empty. These are mechanical inventories, **not linguistic gold, model readings or human-approved interpretations**. Negation, modality, coreference, intent, contextual selection and completion quality therefore remain unmeasured. Existing authored grammar controls in the CDISS suite separately check representation retention for roles/negation/conditions; they must not be confused with language accuracy on the imported history.

Bindings rejected by native candidate or size budgets are preserved as negative results; alternatives are never pruned to force acceptance. The Rust `history_probe` example calls `analyze_joe_result` with the same default configuration as Bomb Code, compares within selected original thread groups, reruns each case for stable state hashes, and checks exact equality of native geometry, retained candidate IDs and typed context-pin center objects. Lexical cases have no imported pin overlay, so zero-pin equality is explicitly marked not exercised rather than qualified as context behavior. Its numeric summary exposes unmapped mass, ambiguity, sparse feature counts and continuity distances. Changes in lexical distributions are descriptive, not evidence of semantic disagreement or progress.

Run from the checkout, substituting a new output path outside Git:

```sh
python3 docs/cdiss/history_benchmark.py --self-test
cargo build -p grok_cdiss --release --example history_probe
/Users/paulcooper/Documents/Codex/2026-10-06/tak/work/sensesnap-reference/python-env/bin/python3 docs/cdiss/history_benchmark.py \
  --database /Users/paulcooper/.grok/control-panel/history/library.sqlite \
  --reference /Users/paulcooper/Documents/Codex/2026-10-06/tak/work/sensesnap-reference/current \
  --output /private/tmp/bomb-code-private-history-benchmark \
  --probe target/release/examples/history_probe
```

Use `--select-only` to audit corpus selection without source binding. The database is opened using SQLite `mode=ro&immutable=1` plus `query_only=ON`; it must be a stable imported library with no writer during the benchmark, with absent or empty WAL and journal files both before and after. The manifest must match installed Joe's expected frozen pin before loading binder code. Every source-package manifest member is hash-verified before and after native binding. A quick integrity check and whole-file SHA-256, size and nanosecond mtime are recorded before and after. Any database or pinned reference change fails the run. The harness refuses output inside its Git checkout and uses directory mode 0700 and file mode 0600. Receipts contain private corpus metadata and must remain local. Share aggregate measurements without passages or thread titles.

The sample cannot establish library-wide coverage, quality across languages, human intention, semantic correctness, E8 distance usefulness, or true completion. Its useful failures include missing exact-form grounding, unresolved competing senses, absent role frames and whole-case budget rejection. A successful geometry receipt confirms retained native coordinates; it does not certify a geometric interpretation of meaning.

ChatGPT imports can contain conversation branches without message-parent metadata. Selected same-thread sequence order is not proof of a single conversation path. Within-thread distance is a scope-compatible mechanical comparison, without assuming adjacent messages reflect one developing decision.

Exact source identities, retained ambiguity and explicit unknowns can support auditable lexical candidate retrieval. Matching context groups and pinned models support compatible comparisons. They do not yet establish relevant-memory retrieval rankings, domain selection, helpful clarifying questions, task dependencies, question resolution or actual progress. Those require separate domain/context representations and independent human review of outcomes; E8 proximity and low distribution distance alone cannot supply that authority.

A literal word inventory includes function words and technical surface forms, and performs no lemmatization or event grouping. Its unmapped mass and receipt-size failures diagnose this declared mechanical strategy against the scoped dictionary/model; they do not directly measure the live Joe model's framing or contextual selection quality. Dictionary candidate coverage and fitted geometric coverage are separate measurements.

## Measured initial history run — 2026-10-06

The final safety-checked run selected 48 actual passages from 28 main threads, with 12 passages per source. Twenty-eight cases bound successfully and yielded CDISS states; 20 remained whole-case native failures (19 exceeded the 180,000-character provider source view, one exceeded the 4 MiB-character full source receipt). No alternatives were discarded or replaced to obtain acceptance.

All 28 accepted cases replayed to identical state hashes and retained exact source candidate IDs and native E8 activation objects. They included 20 fresh states and eight within-thread comparisons. The accepted lexical inventory contained 772 word atoms: 650 had no exact-form dictionary candidates, 60 had multiple candidates, and there were 291 native candidate/source-center/activation objects. Some activation placements were unavailable; 291 is not a count of fitted geometric points. Descriptive fitted geometric mass ranged from 0 to 0.3125. This exposes limited grounding for this declared literal no-lemma strategy against the current scoped reference; it does not measure live Joe linguistic performance.

All event frames were empty, and all eight structure distances were therefore zero. Those zeros are a deliberately demonstrated limitation: they provide no evidence about semantic stability, negation, roles or progress. The imported lexical cases supplied zero context pins, so pin equality was marked `not-exercised`. Existing authored context-pin fixtures remain separate evidence.

All 340 frozen manifest members and the four native implementation pins passed checks; the history database's SHA-256, size and nanosecond mtime were identical before/after, and WAL/journal files were absent at both boundaries. Private output directories used mode 0700 and all artifact files used mode 0600. Three boundary self-tests, the example build/check and strict Clippy passed. Observed CDISS calculation time was median 5.59 ms, maximum 11.67 ms; the run was not an isolated performance qualification.

Private corpus, native bindings, per-case summaries and integrity receipts remain outside Git. The earlier intermediate runs are preserved separately and do not replace this final receipt. These measurements support source-aware retrieval experiments with explicit unknowns and bounded excerpts; they do not yet qualify domain clarification, relevant-memory ranking, LLM question quality, human intent or task completion.

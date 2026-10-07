# Independent recall evaluation cases

The experiment uses 24 private, independently authored known-positive cases from
the already imported archive. There are eight cases each for sense/domain
selection, differently worded reference retrieval, and sentence structure. Each
class has two cases from each of ChatGPT, Claude Code, Codex and Grok. The selected
messages cover 21 distinct main threads. Explanation faithfulness can be audited
on every case; whether an explanation helps the user remains a human judgment.

`scripts/recall_experiment_cases.py` selects candidates and freezes labels without
importing the experiment engine or invoking any retrieval function. Candidate
order is fixed by the `recall-preregister-v1` seed and SHA-256 ordering of source,
thread and message identities. Within a class/source stratum, it selects one
message per thread; target chunks do not repeat across classes. Labels were
written before the author saw any retrieval output. The retrieval configuration
is frozen separately before it receives these labels.

The script applies the existing `history_benchmark.py` exclusion policy to the
full original message. It additionally excludes ambient browser and attachment
wrappers, structured question-reply wrappers, missing-attachment boilerplate,
continuity-token tests, and pasted shell failure logs. Only untruncated user
messages from main threads are eligible. Eligible messages must fit into an
initial recall chunk, be 80–900 characters long, and contain 12–145 Unicode word
tokens. Sense candidates require an ambiguous or domain-specific word from a
fixed list; structure candidates require an explicit scope cue, excluding
"as if" similes. These are eligibility heuristics, not linguistic judgments.

An independent agent authored natural-language queries and source-based
rationales after inspecting only the selected messages. Reference cases omit
several source anchor words and use paraphrases. Sense cases include project
domains and proper names as well as ordinary ambiguous words. Structure queries
express the source's actual conditional, negated, or role-dependent request;
presence of negation does not imply contradiction. No negative passage was
invented or assigned solely because it differed from the selected target.

Every target is joined back to its source/thread/message identity in the native
archive. Original message SHA-256, exact Unicode character span, excerpt bytes,
and excerpt SHA-256 must agree with the derived recall index. Both SQLite files
are opened with `mode=ro&immutable=1` and `query_only=ON`; nonempty WAL or journal
sidecars abort preparation. File hashes before and after preparation must match.
The helper makes no network requests, embeds no text, refreshes no caches and
changes neither database.

Private selection, labels, excerpts, case IDs and judgments are saved outside
Git in a directory with mode `0700`, with files created exclusively as `0600`.
The retrieval engine must receive only the natural-language query as its query
input; target IDs, rationales and selected excerpts are evaluation metadata.
Case source identifiers are not query expansions or ranking features.

Run selection in a new private file, author the separate JSON label map keyed by
case ID with `query` and `rationale`, then freeze into another new file:

```sh
python3 -B scripts/recall_experiment_cases.py select --panel PANEL --output PRIVATE_CANDIDATES
python3 -B scripts/recall_experiment_cases.py freeze --panel PANEL --labels PRIVATE_LABELS --output PRIVATE_CASES
python3 -B scripts/recall_experiment_cases.py --self-test
```

The exact authority label is **agent-authored relevance judgments; not human
gold**. These labels are known positives, not exhaustive judgments of every
related archive chunk. A retrieved related passage outside the target set can
be useful while receiving no target hit. This small, English-query archive test
does not establish multilingual performance, unseen-domain accuracy, or a
causal benefit from E8 root geometry. A failure on a case is retained; cases are
not changed after seeing retrieval results.

## Separate bilingual expansion mechanics

`scripts/recall_experiment_bilingual.py` supplies a bounded dictionary control for
the cross-language part of the reference hypothesis. It opens the pinned native
dictionary, selects source-asserted equivalent English/target-language pairs by
sorted concept and sense IDs, and writes the complete fixture selection before
calling the frozen experiment Interpreter. English origins must have a defined,
unique encoded form. Target-language quotas are Japanese two, Indonesian two,
Malay one and Mandarin one. Candidate links do not qualify as equivalents.

Each pair has two tiny documents in an in-memory FTS5 database: the exact target
lemma, and a target-language control with disjoint source-asserted concept and
tokens. The same existing lexical function searches them using the original
English query and the frozen twelve-term reference expansion. Controls are
mechanically unrelated by these constraints; this is not independently labeled
semantic irrelevance. No archive/index files, embeddings or network are used.

The initial six-form control selected six target forms from one concept and one
English query. It is retained as a separate receipt: literal search found zero
targets, expansion found six, and neither found controls. After identifying the
limited concept coverage, a supplemental fixture rule required six distinct
concepts, frozen before its own rankings. This rule was adopted to improve
coverage, after the first result was available; it was not a preregistered part
of the initial run. The engine, weights and twelve-term budget were unchanged.

The supplemental control found four of six targets with expansion and zero with
literal search. Neither method found controls. Both Japanese targets remained
source-asserted references but fell outside the twelve-term expansion budget;
these misses were retained. The two runs have different coverage and must not
be pooled as twelve independent translation examples.

These results establish source-link and FTS expansion mechanics on selected
lexical forms. They do not establish independent translation correctness,
multilingual archive relevance, domain-general efficacy or meaning accuracy.
Compound lemmas are searched as FTS tokens, not as independently validated
morphological units. Reference and frozen configuration hashes must remain
unchanged before and after each run.

```sh
python3 -B scripts/recall_experiment_bilingual.py --output NEW_PRIVATE_DIRECTORY
python3 -B scripts/recall_experiment_bilingual.py --self-test
```

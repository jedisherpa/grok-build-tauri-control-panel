# Independent full-chain computation controls

The complete computational representation is tested using fresh authored
outlines and source choices, processed by the original pinned native
`prepare_request` → `ground_outline` → `bind_interpretation` functions. The case
lane does not invoke retrieval or inspect implementation scores. It writes all
blueprints and expected relations before binding, and freezes the final packet
manifest before prototype output.

`scripts/full_chain_recall_cases.py` creates 21 authored controls. Twenty have
accepted original native Joe packets. One Japanese alias is explicitly
unavailable at the existing native full-source receipt budget. The manifest
declares 12 relations, of which 11 have both profiles available. Case selection
was independent of retrieval scores; these are representation contracts, not
independent human relevance or linguistic gold.

## What the controls require

- **Active/passive invariance:** parent, child and approval source senses stay
  fixed while wording and occurrence spans change. Complete event content
  should agree; lexical and exact occurrence metadata can differ.
- **Scope and role distinction:** negative approval, conditional approval and
  role reversal retain the relevant source records while event scope or
  predicate-linked participants differ.
- **Multiple-event attachment:** “The parent approves the child, and the child
  rejects the artifact” and “The child approves the artifact, and the parent
  rejects the child” have equal word bags, selected-source concept bags and
  flattened selected sense/role bags. Their predicate-local attachments differ.
  A bag representation cannot satisfy this control.
- **Occurrence and references:** both child occurrences have independent IDs
  and exact spans. Authored reference presence versus absence is preserved.
  A passive paraphrase retains the corresponding two-event relationships.
- **Event links:** before and after links connect the same event content with
  different declared temporal relationships. Surface cues remain separate.
- **Contextual identity:** equal dictionary records and event content coexist
  with different authored personal child meeting IDs. Those contextual pins
  add neither dictionary equivalence nor invented geometry. They must not be
  discarded merely because the source concept is unchanged.
- **Root collision:** two original `able` sense records share `e8-root:195`
  while their source senses, definitions and concepts differ. Root identity
  cannot replace those records.
- **Cross-language identity:** English `entirely` and Japanese `すっぱり` have
  source-asserted equivalent concept links despite different source sense IDs.
  This alias fell outside the earlier twelve-term expansion. Concept postings
  should expose the direct source link without an alphabetical alias budget.
- **Competing readings and missingness:** a two-reading approval case retains
  both source choices, and an unknown word retains its occurrence without
  inventing a definition, source concept, frame event or fitted geometry.

The participants and predicates were verified in the frozen inventory.
`parent` uses its father-or-mother sense, `child` its human-offspring sense,
`approve` the judgment/commendation sense, and `reject` refusal to accept or
acknowledge. The third participant is the uniquely encoded `artifact` form.
Every additional lexical token is retained even when the authored selection
has no source sense. Missing selections do not remove occurrences.

## Retained native coverage limits

Initial parent/help/child controls exceeded the original 180,000-character
provider-view budget while retaining all source alternatives. A longer
conditional control exceeded the four-MiB full-source receipt budget. Those
failed attempts remain preserved. The final controls use source-attested
approval and a shorter explicit condition, without changing the binder,
budgets, dictionary or model, or dropping candidates.

The second cross-language target, Japanese `動く` corresponding to English
`be active`, still exceeds the original four-MiB full-source receipt budget as
a single lexical atom. Both asserted endpoint links were verified before
binding. The target is recorded as unavailable; no partial packet was invented
and no alternatives were deleted. Its declared comparison remains unmeasured.
This is an annotation/packet coverage gap, not a prototype retrieval miss.

The case generator corrected an initially invalid personal meeting namespace
to the native required `meet:personal:` form. That was an authoring correction,
not evidence that the native source mapping changed. It can reuse already
accepted packets from a preserved partial generation only after exact checks
of source pins, sentence, language, original outline, caller context and every
selected source ID. Their original bytes and hashes remain preserved.

## Integrity and use

The frozen manifest includes source byte hashes for the package manifest,
graph, model, three SenseSnap implementations, and native interpretation,
inspection, usage and context code. It records the source inventory counts:
34,801 senses, 2,205 concepts and 2,007 fitted placements. All checked files are
hashed again after generation. Each packet has an exact hash and byte count.
Files are created exclusively as `0600` in `0700` private directories outside
Git. The exact generator source used for the frozen run is preserved beside
its manifest. Neither provider calls nor archive/index access occurs.

The native probe accepts packets by bounded stdin. A private wrapper can load
the manifest's `file` members and send one query plus a small candidate batch;
the largest authored packet is approximately 3.44 MB, so pairwise batches fit
the probe's 32-MiB input budget. The manifest's `expectations` are evaluation
metadata, never extra query/ranking features.

```sh
python3 -B scripts/full_chain_recall_cases.py --output NEW_PRIVATE_DIRECTORY --alias-fixtures FROZEN_BILINGUAL_FIXTURES
python3 -B scripts/full_chain_recall_cases.py --self-test
```

The imported recall index contains 29,755 chunks and no structured meaning
annotations. Seven saved Joe receipts contain five successful proposals with
14 readings and two unavailable attempts. Only the approved cloud receipt has
saved selected-memory evidence joined to its request context. None of those
original receipts stores a word-shape attachment; replay derives the complete
representation from its unchanged binding. These sources support local
proposal retention and computation tests. They cannot establish retrieval
improvement over unannotated archive memories.

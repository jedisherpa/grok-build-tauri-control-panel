# Meaning memory integration — Bomb Code 0.1.13

## User flow

Open Memory. **Find source meanings** looks up the encoded dictionary and lets
you select source-asserted equivalent concepts. **Find encoded source forms in
memory** searches every encoded alias within explicit budgets and the current
source/scope filters. These are cited lexical candidates with usages UNSELECTED.
Existing keyword/vector recall remains available.

**Prepare this passage in Joe** copies one exact cited excerpt into Joe and
displays its context. Preparation stays local. Inspect the text and use the
existing manual **Analyze passage** action when you want the configured provider
to propose readings. A successful native analysis adds an immutable private
reference to its original receipt. An exact single cited excerpt produces a
passage profile; a question with recalled context remains a question profile.
No provider or embedding call is made by source lookup, profile import/status,
comparison or draft preparation.

**Import saved Joe proposals** brings prior successful receipts into the catalog
without changing their original bytes. Batches are limited to 64, with a durable
private UUID cursor so failed early receipts cannot starve later entries. The
catalog holds at most 1024 references and verifies at most 20 per requested page.
It shows stored subject counts separately from this page's current verification.

Choose one saved query and up to 3 candidate profiles, then **Compare selected
profiles**. The UI retains all reading pairs and separately shows source senses,
asserted concepts, dictionary SenseSnap centers, scoped context pins, predicate
and attached roles/polarity/modality, event links/references, multiplicity,
collisions and fine native E8 positions. Inspect original frames and source
provenance. Missing evidence stays explicit. There is no combined relevance or
confidence score, and no automatic meaning selection.

Difference questions use unambiguous predicate/event correspondences and carry
receipt/reading/event references. **Add question to unsent message** revalidates
current comparison inputs and respects the existing Joe composer guard. Edits,
thread switches, source/index changes and changed profile choices invalidate old
results. The user reviews and sends the unsent message through the existing flow.

## Code and storage

- `src-tauri/src/meaning_memory.rs`: bounded ID-only command, private catalog,
  original receipt reconstruction, current source joins and comparison/draft
  validation.
- `src-tauri/src/meaning_candidates.rs`: bounded embedded trusted Python worker
  sharing the existing recall operation lock and native citation validator.
- `scripts/source_concept_candidates.py`: all-encoded source alias lookup,
  read-only archive and scoped-note matching with explicit whole-result limits.
- `frontend/meaning-memory.js` and `.css`: existing Memory screen controls and
  separate comparison explanations; surgical Memory/Joe hooks reuse citation
  preparation, word-shape inspection and unsent composer behavior.
- `scripts/word_dictionary.py`: an explicit asserted-equivalent concept field
  beside the complete source alignment records.

The catalog stores small 0600 profile references in the existing 0700
`wizard-joe/meaning-profiles` directory. Complete original interpretation receipts
remain in `wizard-joe/receipts`. Original SHA-256 bytes, frozen model/source pins
and all 15 exact citation fields are rechecked. A generation refresh can preserve
a profile only when the underlying evidence identity and content are identical.
Changed, missing or oversized evidence is withheld as a whole result.

No second execution plane, tool authority, background provider indexing, raw
corpus duplication or fabricated completion percentage was added.

## Evidence and practical limits

Architecture and performance/simplicity reviews completed with zero unresolved
Critical, High or actionable Medium findings. Meaning-memory native regressions
include exact passage classification, immutable private refs, source tampering,
all-field citation changes, generation refresh, ambiguous events, draft limits,
compact-card budget parity and 130-record import progress.

Final validation: workspace check, strict all-target Clippy and 208 Rust tests
pass. The ordinary suite marks six environment fixtures ignored; four new
worker/host/private fixtures were explicitly executed and passed, while two
existing development-server fixtures remain ignored. All 132 frontend tests,
18 Python source candidate tests, 26 dictionary tests and 31 recall tests pass.

The installed Mac app imports five authentic saved proposals, verifies all five
and preserves two earlier unavailable receipts. The saved approval comparison
returns all six reading pairs, displays independent senses/concepts/context/event
and geometry signals, and prepares a traceable polarity question. Native
fingerprint validation precedes its insertion into the unsent composer; the
verification draft was cleared without sending. Source alias search returns
seven cited candidates across 29,755 indexed excerpts. One exact candidate is
prepared in Joe with visible citation/context and manual Analyze disclosure.
No Analyze or Send action was invoked during this release test.

All 13 protected history/index/configuration/original receipt/reference files
retain their prior hashes. Five new private profile references and one bounded
import-progress file are the intended new derived storage. The verified 0.1.13
bundle replaces the idle installed app; the old 0.1.12 bundle is preserved for
rollback. Code signing is local ad-hoc, with strict signature verification; this
is a local build, not a notarized distributable release.

The earlier 24-query exploratory proxy experiment did not establish an improved
ranker; existing hybrid ranking stays unchanged. The new complete-profile path
retains distinctions demonstrated by authored controls and authentic saved
readings. Archive-wide interpreted coverage, independent relevance accuracy and
human question usefulness remain unmeasured. Profile accumulation now makes
those evaluations possible as users analyze and review more passages.

For a useful evaluation, retain query, compared profile IDs, current source pins
and the user's independent judgment of relevant/wrong-sense/missing results.
Compare keyword/vector candidates with the separate source-concept channel,
then measure interpreted coverage and judged usefulness by domain. Increased
profile counts alone do not establish increased accuracy.

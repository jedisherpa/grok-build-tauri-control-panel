# Word shapes and inverse dictionary traceability

Release 0.1.12 implements Paul's dictionary → cross-language → SenseSnap → sentence-use chain in the existing Joe reader, plus a local browser for the complete encoded dictionary. The source E8 map remains fixed. A glyph is an inspectable representation of sourced identities and proposed usage, not a newly validated semantic operator.

## Source and occurrence contract

`grok_cdiss::word_shapes` accepts an existing Joe result only after the same native source/structure validation used by CDISS. It retains every candidate sense, selection flags, exact dictionary records and alignment IDs, unchanged selected native activations, SenseSnap centers, proposed sentence frames and scope cues. Repeated occurrences retain distinct IDs and half-open Unicode scalar spans. A word may branch into competing senses. Root collisions retain separate sense/concept IDs.

Coverage includes Unicode alphanumeric tokens with combining marks and internal apostrophes. It is a declared heuristic rather than a linguistic tokenizer; scripts without explicit word boundaries need language-aware segmentation before coverage can become a linguistic claim. Full-span inventory coverage, dictionary selection and fitted geometry are separate statuses. Tokens outside the interpreter inventory remain visible as uncovered.

Four display markers form the provenance path. Their spacing is declared layout, not a shared calibrated semantic distance. The optional eight-axis profile displays the original vector with a declared atan scale. The original eight coordinates, root address and residual remain inspectable. Fixed role arrows encode proposed predicate/agent/theme/patient/recipient/experiencer/instrument/location roles, and scope retains polarity, modality and cues. The display does not rotate the source E8 address, infer meaning from camera movement or grant authority.

Dictionary-source SenseSnap centers join only the selected source senses they explicitly name. Independent context pins remain occurrence-level proposals. Unselected candidates cannot inherit a different sense's meeting.

## Complete encoded inventory

The local `word_shape_dictionary` command reads a fixed app-private reference through a bounded Python subprocess. It verifies the pinned package manifest, original graph/model bytes, logical graph/model identity and three SenseSnap implementation hashes. Client payloads cannot choose paths or a provider. No network request, dictionary write or model generation occurs.

The frozen inventory has 34,801 senses across English, Japanese, Indonesian, Malay and Mandarin, 2,205 concepts, 2,007 fitted concept positions and 198 unplaced concepts. There are 16,435 sense-local definitions and 18,366 gaps. Shared concept glosses are separately labelled with their language; they are not substituted as native definitions. Membership and counterpart records retain source attribution, candidates and unresolved statuses.

Search accepts literal words/definitions, exact sense IDs, concept IDs and native E8 root IDs. Root lookup follows exact root → fitted concept → original source sense links and retains candidate-versus-asserted status. It never substitutes nearby roots or infers synonymy. Empty queries enumerate the inventory. Result pages and cross-language counterpart previews report exact totals and continuation offsets; each individual sense remains queryable by ID.

Every response is bounded to 1 MiB, each page to 20 senses, and the worker to 180 seconds with a single-operation mutex and child termination on timeout. The current snapshot loads in roughly five seconds; its transient parsing peak measured about 726 MiB. Source/model verification precedes use. Core shapes are bounded to 8 MiB; complete private receipts to 16 MiB. Larger shape attachments become visibly unavailable rather than discarding original interpretation evidence. The viewer renders lazily in pages and retains explicit display limits.

## Saved readings and native use

Joe's local authored comparison displays shapes without a provider call. New manual analyses save the shape attachment with the original result. A saved-review UUID can be replayed only in its original Bomb Code thread. Replay verifies the current manifest, logical graph, canonical model, model ID and SenseSnap code pins against the saved result. Selected recalled evidence must match both the saved memory context and the bound interpreter context and is revalidated before, after and before frontend display. Input/thread/context changes discard late responses. Replay and browsing never send a prompt, draft a coding message, commit memory or grant permission.

To try: open Joe, expand **Word shapes · inspect the encoded dictionary**, and search `bank`, a native word such as `月` in Japanese, an exact concept ID, or `e8-root:120`. For sentence shapes, choose **Try a local source-backed comparison** or inspect a saved Joe receipt. Expand the reading, occurrence and selected sense to see the glyph and source details. Existing Analyze retains its explicit provider disclosure.

## Validation and limits

Tests cover exact source/vector retention, role reversal, negation, conditional scope, repeated and Unicode occurrences, partial-span coverage, contextual pins, root collisions, missing definitions/placements, actual multilingual census and inverse root set equality. Boundary regressions cover stale replies, source/model/code changes, source substitutions, oversize receipts, malicious text, malformed vectors and independent SenseSnap centers. Independent architecture and performance/simplicity reviews precede the local installation.

The representation is implemented. Predictive interpretation from orientation remains unvalidated and is not enabled. Missing native dictionary evidence is a source-data gap; no fabricated definition, equivalence or E8 position fills it. The full native dictionary is the encoded snapshot, not every possible word in those languages. User intent and completion remain separate from geometric addresses and display orientations.

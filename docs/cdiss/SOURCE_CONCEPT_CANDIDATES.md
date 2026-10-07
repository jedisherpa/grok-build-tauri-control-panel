# Source-concept bridge for unannotated memory

`scripts/source_concept_candidates.py` supplies a source-based candidate channel alongside the native interpreted-profile prototype. It bridges the existing archive without assigning senses, annotating frames, invoking a provider, computing embeddings or adding another store.

One to eight caller-selected encoded concept IDs enumerate **every** dictionary source lemma having an asserted `equivalent` alignment to a queried concept. Candidate alignments are excluded from query support. This lookup has no twelve-alias cap. Complete alias and exact-form alternative inventories are bounded; oversized inventories fail explicitly rather than pruning forms or alternatives. The catalog retains every normalized form and all exact-form source sense IDs, including senses associated with different concepts.

Matching retains occurrence multiplicity and zero-based half-open Unicode scalar spans. It declares NFC/base-combining-cluster normalization, casefolding, underscore-to-space conversion and whitespace collapse. Span mappings retain the original surface even when normalization changes its length; half of a casefold expansion cannot match. Latin source forms require Unicode word boundaries. Forms containing Japanese/CJK characters use literal subsequences, explicitly without a linguistic segmentation claim. There is no lemmatization, guessed translation or definition-token expansion.

Every occurrence and source alternative has `usageSelection: UNSELECTED`. An asserted dictionary relationship describes its source record; it does not select the sense of an archived occurrence. Returned hits retain all exact-form sense alternatives, original definitions and languages, source metadata, full alignment records, exact chunk metadata and message-relative spans. SenseSnap and sentence-use evidence are explicitly unavailable for these unannotated candidates. The adapter neither invents those records nor treats an alias match as a verified interpreted profile.

The query-time scan orders candidates by the count of distinct queried source concepts supported, then stable chunk ID. More aliases or repeated occurrences cannot add ranking credit. Total candidates and matching occurrences are counted over the complete scan; a bounded return page reports how many candidates remain. All alternatives and occurrences of each returned hit remain present. This is source-concept support order, not a relevance probability or a replacement for existing hybrid search.

The CLI reads only the fixed installed reference, imported archive and derived recall index. `Dictionary.load()` verifies graph/model/manifest and SenseSnap implementation pins. SQLite scans use read-only immutable connections, with nonempty WAL/journal refusal; original file hashes, sizes and mtimes are checked before and after. Existing generation, source freshness, note snapshot and embedding-basis compatibility are checked. Stored model digest and dimension are recorded from index metadata, without contacting Ollama or using vectors. Active model identity is intentionally not queried by this source-only channel. Each returned candidate is rematerialized through the unchanged memory evidence validator before use.

Bounds are eight concepts, 8,192 normalized aliases, 512 characters per alias, 1 MiB total alias characters, 512 exact-form alternatives, 512 matching occurrences per chunk, the existing 1,000-scalar chunk limit, 250,000 scanned chunks, twenty returned candidates, 8 MiB complete output and a hard 180-second CLI deadline. Existing recall limits also apply: 1 GiB derived SQLite file/page allocation and 384 MiB cumulative serialized chunk text. Budget errors withhold a whole result and never imply absence of candidates. Outputs must be new directories outside Git and native stores, with directory mode 0700 and file mode 0600. Source files, annotations, index, memories and permissions remain unchanged.

Example, with a new private output directory:

```sh
python3 -B scripts/source_concept_candidates.py \
  --concept pwn30:08420278-n --limit 5 \
  --output /private/tmp/bomb-code-source-concept-control
python3 -B -m unittest discover -s scripts -p test_source_concept_candidates.py -v
```

The actual installed financial-bank concept control enumerated ten asserted source forms and scanned all 29,755 existing chunks in 12.59 seconds. Seven lexical candidate chunks were found; five were returned with full original citation validation. Original archive/index and reference checks passed, with zero provider calls, embedding calls or native writes. These seven are unselected candidates, not independently judged relevant financial-bank memories. Private originals and receipts remain outside Git.

Focused native dictionary controls use the six distinct-concept bilingual pairs frozen in the prior experiment. Direct concept enumeration retrieves their source forms including the two Japanese forms previously excluded by the twelve-term expansion budget. These tiny-record controls qualify graph traversal and span/provenance retention, not independent translation accuracy or multilingual archive recall. Core tests cover ambiguity, candidate-versus-asserted relationships, source records, repeated/Unicode occurrences, normalization, boundaries, concept-support ordering, explicit limits and private output.

Future integration should attach this candidate channel to the existing recall generation and source citations. A separate verified interpreted-profile posting records selected senses and complete event attachments; comparisons and clarification questions requiring sentence roles or contextual centers consume that channel. Alias occurrences can propose which passages to inspect or interpret, without being promoted into selected-profile evidence.

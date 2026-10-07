#!/usr/bin/env python3
"""Source-concept candidate bridge over unannotated local memory.

Only the CLI loads the fixed installed reference/index. Pure matching functions
accept records for focused controls. A lexical occurrence never selects a sense.
"""
import argparse
import collections
import contextlib
import hashlib
import json
import os
from pathlib import Path
import signal
import sqlite3
import time
import unicodedata

import memory_recall as recall
import word_dictionary

SCHEMA = 'bomb-code/source-concept-candidates/v1'
PANEL = Path('/Users/paulcooper/.grok/control-panel')
MAX_CONCEPTS = 8
MAX_ALIASES = 8192
MAX_ALIAS_CHARS = 512
MAX_ALIAS_TOTAL_CHARS = 1024 * 1024
MAX_CHUNKS = 250000
MAX_OCCURRENCES_PER_CHUNK = 512
MAX_ALTERNATIVES_PER_FORM = 512
MAX_OUTPUT_BYTES = 8 * 1024 * 1024
MAX_RETURNED = 20
MAX_SECONDS = 180


class CandidateError(ValueError):
    pass


def sha_file(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def word_character(char):
    return char.isalnum() or char == '_' or unicodedata.category(char).startswith('M')


def cjk(char):
    return any(low <= ord(char) <= high for low, high in
               ((0x3040, 0x30ff), (0x3400, 0x9fff), (0x20000, 0x3134f)))


def normalized_spans(text):
    """Normalize complete base/combining clusters, retaining scalar source spans."""
    result, spans = [], []
    index = 0
    while index < len(text):
        start = index
        index += 1
        while index < len(text) and unicodedata.category(text[index]).startswith('M'):
            index += 1
        cluster = unicodedata.normalize('NFC', text[start:index]).replace('_', ' ').casefold()
        for char in cluster:
            if char.isspace():
                if result and result[-1] == ' ':
                    spans[-1] = (spans[-1][0], index)
                    continue
                char = ' '
            result.append(char)
            spans.append((start, index))
    return ''.join(result), spans


class SourceAliases:
    def __init__(self, dictionary, concept_ids):
        if (not isinstance(concept_ids, list) or not 1 <= len(concept_ids) <= MAX_CONCEPTS
                or any(not isinstance(cid, str) or cid not in dictionary.concepts for cid in concept_ids)
                or len(set(concept_ids)) != len(concept_ids)):
            raise CandidateError('Provide one to eight distinct encoded source concept IDs')
        self.dictionary = dictionary
        self.concept_ids = sorted(concept_ids)
        query_ids = set(concept_ids)
        self.forms = {}
        for cid in self.concept_ids:
            for sid in sorted(dictionary.by_concept[cid]):
                sense = dictionary.senses[sid]
                asserted = [a for a in dictionary.alignments[sid]
                            if a.get('concept_id') == cid and a.get('kind') == 'equivalent'
                            and a.get('asserted') is True]
                if not asserted:
                    continue
                form = word_dictionary.normalize(sense['lemma'])
                if not form or len(form) > MAX_ALIAS_CHARS:
                    raise CandidateError('A source alias exceeds the complete alias budget')
                self.forms.setdefault(form, set()).add(cid)
        if (len(self.forms) > MAX_ALIASES
                or sum(len(form) for form in self.forms) > MAX_ALIAS_TOTAL_CHARS):
            raise CandidateError('Complete source alias budget exceeded; no aliases were dropped')
        self.alternatives = collections.defaultdict(list)
        for sid, sense in sorted(dictionary.senses.items()):
            form = word_dictionary.normalize(sense['lemma'])
            if form in self.forms:
                self.alternatives[form].append(sid)
        for form, ids in self.alternatives.items():
            if len(ids) > MAX_ALTERNATIVES_PER_FORM:
                raise CandidateError('Complete exact-form alternative budget exceeded')
        self.catalog = []
        self.trie = {}
        for form, ids in sorted(self.forms.items()):
            self.catalog.append({'normalizedForm': form, 'assertedQueryConceptIds': sorted(ids),
                                 'senseIds': self.alternatives[form], 'usageSelection': 'UNSELECTED'})
            node = self.trie
            for char in form:
                node = node.setdefault(char, {})
            node[None] = form
        self.catalog_sha256 = word_dictionary.canonical_hash(self.catalog)
        self.query_ids = query_ids

    def occurrences(self, text):
        if not isinstance(text, str) or len(text) > recall.MAX_CHARS:
            raise CandidateError('Recall chunk text exceeds the existing 1000-scalar budget')
        normalized, spans = normalized_spans(text)
        occurrences = []
        for start in range(len(normalized)):
            # A casefold expansion cannot be matched halfway through a source cluster.
            if start and spans[start] == spans[start - 1]:
                continue
            node = self.trie
            end = start
            while end < len(normalized) and normalized[end] in node:
                node = node[normalized[end]]
                end += 1
                form = node.get(None)
                if form is None or (end < len(spans) and spans[end] == spans[end - 1]):
                    continue
                has_cjk = any(cjk(char) for char in form)
                if not has_cjk and ((start and word_character(normalized[start - 1]) and word_character(form[0]))
                                    or (end < len(normalized) and word_character(normalized[end]) and word_character(form[-1]))):
                    continue
                left, right = spans[start][0], spans[end - 1][1]
                occurrences.append({'span': [left, right], 'surface': text[left:right],
                                    'normalizedForm': form, 'assertedQueryConceptIds': sorted(self.forms[form]),
                                    'alternativeSenseIds': self.alternatives[form],
                                    'usageSelection': 'UNSELECTED',
                                    'matchBasis': 'NFC/casefold/underscore/whitespace source-form match; '
                                        + ('CJK literal subsequence, not segmentation' if has_cjk else 'Unicode word boundaries')})
                if len(occurrences) > MAX_OCCURRENCES_PER_CHUNK:
                    raise CandidateError('Complete occurrence budget exceeded; no hits were dropped')
        return occurrences

    def source_records(self, forms):
        records = {}
        for form in sorted(forms):
            for sid in self.alternatives[form]:
                sense = self.dictionary.senses[sid]
                alignments = self.dictionary.alignments[sid]
                query_links = [a for a in alignments if a.get('concept_id') in self.query_ids
                               and a.get('kind') == 'equivalent' and a.get('asserted') is True]
                records[sid] = {'sense': sense, 'source': self.dictionary.source(sense.get('source_id')),
                                'alignments': alignments, 'assertedQueryAlignments': query_links,
                                'usageSelection': 'UNSELECTED',
                                'definitionStatus': 'source-definition' if sense.get('definition') else 'missing'}
        return records


def match_records(aliases, records, limit=10):
    if type(limit) is not int or not 1 <= limit <= MAX_RETURNED:
        raise CandidateError('Candidate limit must be one to twenty')
    best, scanned, candidates, occurrence_count = [], 0, 0, 0
    for record in records:
        scanned += 1
        if scanned > MAX_CHUNKS:
            raise CandidateError('Complete scan budget exceeded; no partial scan is returned')
        occurrences = aliases.occurrences(record['text'])
        if not occurrences:
            continue
        candidates += 1
        occurrence_count += len(occurrences)
        support = sorted({cid for occurrence in occurrences for cid in occurrence['assertedQueryConceptIds']})
        candidate = {'chunk': record, 'occurrences': occurrences, 'distinctConceptSupport': support,
                     'usageSelection': 'UNSELECTED', 'sentenceUse': 'unavailable-uninterpreted',
                     'senseSnap': 'unavailable-uninterpreted'}
        best.append(candidate)
        best.sort(key=lambda item: (-len(item['distinctConceptSupport']), item['chunk']['chunkId']))
        del best[limit:]
    return {'scannedChunks': scanned, 'candidateCount': candidates, 'matchedOccurrenceCount': occurrence_count,
            'returnedCount': len(best), 'notReturnedCount': candidates-len(best), 'completeScan': True,
            'selectionBasis': 'distinct queried source concepts descending; stable chunkId tie; '
                              'no boost for alias or occurrence count', 'hits': best}


class StoredModel:
    """Pinned stored model metadata only; no active-model or embedding request."""
    def __init__(self, digest):
        self.digest = digest
    def identity(self):
        return self.digest
    def embed(self, _texts):
        raise CandidateError('This source-only adapter never embeds text')


def immutable(path):
    connection = sqlite3.connect(path.resolve().as_uri()+'?mode=ro&immutable=1', uri=True)
    connection.row_factory = sqlite3.Row
    connection.execute('PRAGMA query_only=ON')
    return connection


def source_snapshot():
    snapshot = {}
    for path in (PANEL/'history/library.sqlite', PANEL/'memory-recall/recall.sqlite'):
        if path.is_symlink() or not path.is_file():
            raise CandidateError('Fixed source path unavailable or redirected')
        if path.name == 'recall.sqlite' and path.stat().st_size > recall.MAX_DATABASE_BYTES:
            raise CandidateError('Existing recall database byte budget exceeded; result withheld')
        for suffix in ('-wal', '-journal'):
            sidecar = Path(str(path)+suffix)
            if sidecar.exists() and sidecar.stat().st_size:
                raise CandidateError('Stable source required; nonempty WAL/journal present')
        snapshot[str(path)] = {'sha256': sha_file(path), 'bytes': path.stat().st_size,
                               'mtimeNs': path.stat().st_mtime_ns}
    return snapshot


def reference_snapshot(reference):
    expected = {word_dictionary.MANIFEST_PATH: reference['manifestSha256'],
                reference['graphPath']: reference['graphFileSha256'],
                reference['modelPath']: reference['modelFileSha256'],
                **reference['senseSnapImplementationHashes']}
    actual = {}
    for path, digest in expected.items():
        _, actual[path] = word_dictionary.read_member(word_dictionary.REFERENCE_ROOT, path,
            word_dictionary.MAX_GRAPH, expected_hash=digest)
    return actual


def index_records(db):
    """Enforce existing recall page/text budgets before returning a complete scan."""
    recall.check_database_budget(db)
    serialized_bytes = 0
    for row in db.execute('SELECT data FROM chunks ORDER BY chunk_id'):
        if not isinstance(row[0], str):
            raise CandidateError('Recall metadata record is not serialized text')
        size = len(row[0].encode('utf-8'))
        if size > 64 * 1024:
            raise CandidateError('Recall metadata record exceeds bounded input')
        serialized_bytes += size
        if serialized_bytes > recall.MAX_INDEX_BYTES:
            raise CandidateError('Existing serialized recall text budget exceeded; no partial scan is returned')
        yield json.loads(row[0])


def query_installed(concept_ids, limit=10):
    started = time.monotonic()
    before = source_snapshot()
    dictionary = word_dictionary.Dictionary.load()
    reference_before = dict(dictionary.reference)
    aliases = SourceAliases(dictionary, concept_ids)
    with contextlib.closing(immutable(PANEL/'memory-recall/recall.sqlite')) as db:
        recall.check_database_budget(db)
        meta = recall.get_meta(db)
        models = list(db.execute('SELECT model_digest,dimension,count(*) AS count FROM vectors '
                                'GROUP BY model_digest,dimension'))
        if len(models) > 1:
            raise CandidateError('Stored vector model basis is mixed; source query withheld')
        if models and (not isinstance(models[0]['model_digest'], str)
                       or len(models[0]['model_digest'].removeprefix('sha256:')) != 64
                       or any(char not in '0123456789abcdef' for char in models[0]['model_digest'].removeprefix('sha256:'))
                       or type(models[0]['dimension']) is not int or not 1 <= models[0]['dimension'] <= 4096):
            raise CandidateError('Stored vector model identity/dimension is malformed')
        model = StoredModel(models[0]['model_digest'] if models else None)
        status = recall.status_unlocked(PANEL, [], model)
        recall.ensure_ready(status)
        if not status['embeddingBasisCompatible']:
            raise CandidateError('Existing index embedding basis is incompatible')
        result = match_records(aliases, index_records(db), limit)
        if result['scannedChunks'] != meta['chunkCount']:
            raise CandidateError('Index chunk census disagrees with full scan')
    for hit in result['hits']:
        chunk = hit['chunk']
        checked = recall.evidence(PANEL, [], model,
            {'generation': status['generation'], 'chunkIds': [chunk['chunkId']]})['evidence'][0]
        if any(checked.get(key) != value for key, value in chunk.items()):
            raise CandidateError('Returned candidate changed during citation validation')
        for occurrence in hit['occurrences']:
            occurrence['messageSpan'] = [chunk['start']+value for value in occurrence['span']]
    forms = {occurrence['normalizedForm'] for hit in result['hits'] for occurrence in hit['occurrences']}
    result.update(schema=SCHEMA, queryConceptIds=aliases.concept_ids,
                  queryConcepts={cid: dictionary.concepts[cid] for cid in aliases.concept_ids},
                  aliasCatalog=aliases.catalog, aliasCount=len(aliases.catalog), aliasCatalogSha256=aliases.catalog_sha256,
                  sourceRecords=aliases.source_records(forms), reference=reference_before,
                  recallBasis={'generation': status['generation'], 'historyFingerprint': meta['historyFingerprint'],
                    'notesDigest': meta['notesDigest'], 'embeddingBasisFingerprint': meta['embeddingBasisFingerprint'],
                    'storedModels': [dict(row) for row in models], 'activeModelQueried': False, 'embeddingsUsed': False},
                  evidenceValidatedCount=len(result['hits']),
                  authority={'toolsDispatched': False, 'approvalsGranted': False, 'memoryCommitted': False},
                  qualification='Caller-selected source concepts; all lexical usages and alternatives UNSELECTED. '
                    'No interpreted frame, SenseSnap center, contextual sense selection or relevance confidence is inferred.')
    if len(json.dumps(result, ensure_ascii=False, allow_nan=False).encode('utf-8')) > MAX_OUTPUT_BYTES:
        raise CandidateError('Complete candidate provenance exceeds 8 MiB; no alternatives were dropped')
    after = source_snapshot()
    reference_after = reference_snapshot(reference_before)
    if before != after:
        raise CandidateError('Original archive/index changed; source candidate result withheld')
    result.update(sourceIntegrity=True, referenceIntegrity=True,
                  preservation={'before': before, 'after': after, 'referenceAfter': reference_after},
                  elapsedSeconds=time.monotonic()-started, providerCalls=0, embeddingCalls=0, nativeWrites=0)
    if len(json.dumps(result, ensure_ascii=False, allow_nan=False).encode('utf-8')) > MAX_OUTPUT_BYTES:
        raise CandidateError('Complete preserved provenance exceeds 8 MiB; no alternatives were dropped')
    return result


def write_private(output, result):
    output = Path(output).resolve()
    repo = Path(__file__).resolve().parents[1]
    if output.is_relative_to(repo) or output.is_relative_to(PANEL) or output.exists():
        raise CandidateError('Output must be a new private directory outside Git and native stores')
    encoded = json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False).encode('utf-8')
    if len(encoded) > MAX_OUTPUT_BYTES:
        raise CandidateError('Serialized complete provenance exceeds 8 MiB')
    os.mkdir(output, 0o700)
    fd = os.open(output/'candidates.json', os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(fd, 'wb') as handle:
        handle.write(encoded)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--concept', action='append', required=True)
    parser.add_argument('--limit', type=int, default=10)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    def deadline(_signum, _frame):
        raise CandidateError('Source candidate query exceeded hard 180-second budget')
    signal.signal(signal.SIGALRM, deadline)
    signal.alarm(MAX_SECONDS)
    result = query_installed(args.concept, args.limit)
    signal.alarm(0)
    write_private(args.output, result)
    print(json.dumps({key: result[key] for key in ('schema', 'queryConceptIds', 'aliasCount', 'scannedChunks',
          'candidateCount', 'returnedCount', 'evidenceValidatedCount', 'sourceIntegrity', 'referenceIntegrity',
          'providerCalls', 'embeddingCalls', 'nativeWrites', 'elapsedSeconds')}))

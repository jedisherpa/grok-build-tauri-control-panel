"""Read-only, preregistered recall ablations. Raw results must remain outside Git.

This measures dictionary-derived proxies, not interpreted CDISS on the archive.
Ranking receives query text only. Gold IDs enter the metric stage afterward.
"""
import argparse
import ast
import collections
import contextlib
import hashlib
import inspect
import json
import math
import os
from pathlib import Path
import re
import signal
import time

import memory_recall as recall
import word_dictionary

CONFIG = {
    'schema': 'bomb-code/recall-experiment-config/v1', 'baseline': 'existing-search-limit100-only',
    'rrfK': 60, 'candidateLimit': 100, 'expansionTermLimit': 12,
    'senseWeight': .20, 'referenceRrfWeight': .35, 'negationMismatchPenalty': .12,
    'conditionMismatchPenalty': .12, 'roleMatchWeight': .16, 'geometryWeight': .10,
    'senseSelection': 'exact-lemma; definition-context overlap positive unique maximum; ties abstain',
    'referenceSelection': 'both endpoints asserted equivalent to same concept only',
    'geometry': 'verified original position8 cosine; dictionary-proxy mean; no root equivalence',
    'scope': 'No archive-wide interpreted word shapes; no learned inference; no remote calls or index writes',
}
METHODS = ('baseline', 'sense_domain', 'reference_expansion', 'structure', 'geometry', 'combined')
class QueryDeadline(Exception):
    pass

STOP = frozenset('a an the to of in on at for and or is are was were be been with this that it its i you we they he she what which how about find recall remember my our did does do me please'.split())
NEG = frozenset(('not', 'never', 'no', 'without', "don't", "didn't", "wasn't", "isn't", 'cannot'))
CONDITIONAL = frozenset(('if', 'unless', 'provided', 'assuming', 'would', 'could', 'might'))
VERBS = frozenset('approve approved reject rejected support supported replace replaced help helped pay paid send sent ask asked tell told buy bought sell sold'.split())
CONFIG.update(stopwords=sorted(STOP), negationTokens=sorted(NEG), conditionTokens=sorted(CONDITIONAL),
              roleVerbs=sorted(VERBS), scopeWindowChars=80, scopeWindowLimit=8, selectedSenseLimit=32,
              timingMeaning='whole-query combined workload; local embeddings cached for top20 parity; not per-method latency')


def sha_file(path):
    value = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def search100_function():
    """Compile the actual baseline, changing exactly its public result ceiling."""
    source = inspect.getsource(recall.search)
    tree = ast.parse(source)
    changed = 0
    for node in ast.walk(tree):
        if (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                and node.func.id == 'integer' and len(node.args) == 5
                and isinstance(node.args[4], ast.Constant) and node.args[4].value == 'limit'):
            if not isinstance(node.args[3], ast.Constant) or node.args[3].value != 20:
                raise ValueError('Baseline result ceiling changed; audit required')
            node.args[3] = ast.Constant(value=100)
            changed += 1
    if changed != 1:
        raise ValueError('Baseline adaptation is not exactly one bound change')
    namespace = dict(vars(recall))
    exec(compile(ast.fix_missing_locations(tree), '<existing-search-limit100>', 'exec'), namespace)
    return namespace['search'], recall.digest(source)


SEARCH100, BASELINE_SOURCE_SHA = search100_function()


def tokens(text):
    return re.findall(r"[^\W_]+(?:['’][^\W_]+)*", text.casefold(), re.UNICODE)


def content(text):
    return set(tokens(text)) - STOP


def structure_features(text):
    words = tokens(text)
    clauses = re.split(r'[.!?;\n]', text.casefold())
    roles = []
    for clause in clauses:
        values = [word for word in tokens(clause) if word not in STOP and word not in NEG]
        for index, word in enumerate(values):
            if word in VERBS and index > 0 and index + 1 < len(values):
                roles.append([values[index - 1], word, values[index + 1]])
    return {'negated': bool(set(words) & NEG), 'conditional': bool(set(words) & CONDITIONAL),
            'roles': roles, 'qualification': 'proposed restricted lexical heuristic; not validated sentence analysis'}


class CachedLocalModel(recall.LocalModel):
    def __init__(self):
        super().__init__()
        self.cache = {}
        self.cache_hits = 0
        self.requests = 0

    def embed(self, texts):
        identity = self.identity()
        if not identity:
            raise recall.RecallError('Local model unavailable before cached query embedding')
        key = (identity, tuple(texts))
        if key not in self.cache:
            self.requests += 1
            self.cache[key] = super().embed(texts)
        else:
            self.cache_hits += 1
        return self.cache[key]


class Interpreter:
    def __init__(self, dictionary):
        self.dictionary = dictionary
        self.by_lemma = collections.defaultdict(list)
        for sense in dictionary.senses.values():
            self.by_lemma[word_dictionary.normalize(sense['lemma'])].append(sense)
        self.cache = {}

    def interpret(self, text):
        if text in self.cache:
            return self.cache[text]
        words = tokens(text)
        context = content(text)
        forms = sorted({word_dictionary.normalize(' '.join(words[i:i + length]))
                        for i in range(len(words)) for length in (1, 2, 3)
                        if i + length <= len(words)} - STOP)
        selected, unresolved = [], []
        for form in forms:
            senses = self.by_lemma.get(form, [])
            if not senses:
                continue
            ranked = []
            for sense in senses:
                definition = sense.get('definition') or ''
                overlap = sorted((context - content(form)) & content(definition))
                ranked.append((len(overlap), sense['id'], sense, overlap))
            ranked.sort(key=lambda row: (-row[0], row[1]))
            unique = len(ranked) == 1
            margin = ranked[0][0] - (ranked[1][0] if len(ranked) > 1 else 0)
            if (not unique and (ranked[0][0] == 0 or margin <= 0)) or not ranked[0][2].get('definition'):
                unresolved.append({'form': form, 'candidateCount': len(senses),
                                   'bestOverlap': ranked[0][0], 'reason': 'missing definition or tied/unsupported sense'})
                continue
            score, sid, sense, overlap = ranked[0]
            concepts = sorted({a['concept_id'] for a in self.dictionary.alignments[sid]
                               if a.get('concept_id') and a.get('kind') == 'equivalent' and a.get('asserted') is True})
            selected.append({'form': form, 'senseId': sid, 'definition': sense['definition'],
                             'definitionLanguage': sense.get('definition_language'), 'overlap': overlap,
                             'selection': 'unique-encoded-form' if unique else 'proposed-definition-context-overlap',
                             'assertedConceptIds': concepts, 'sourceId': sense.get('source_id'),
                             'sourceRecordId': sense.get('source_record_id')})
        selected = selected[:32]
        references = []
        vectors = []
        for sense in selected:
            sid = sense['senseId']
            for cid in sense['assertedConceptIds']:
                placement = self.dictionary.placements.get(cid)
                if placement and isinstance(placement.get('position8'), list):
                    vectors.append({'conceptId': cid, 'rootId': placement.get('root_id'), 'position8': placement['position8']})
                for other_id in sorted(self.dictionary.by_concept[cid]):
                    other = self.dictionary.senses[other_id]
                    counterpart = self.dictionary.counterpart(other_id, [cid], sid)
                    if counterpart['linkStatus'] == 'source-equivalent' and other_id != sid:
                        references.append({'lemma': other['lemma'], 'language': other['language'],
                                           'originSenseId': sid, 'senseId': other_id, 'conceptId': cid,
                                           'alignmentIds': counterpart['alignmentIds'], 'status': 'source-equivalent-not-independent-gold'})
        references.sort(key=lambda row: (row['lemma'].casefold(), row['senseId'], row['conceptId']))
        expansion = list(dict.fromkeys(row['lemma'] for row in references if word_dictionary.normalize(row['lemma']) not in forms))[:CONFIG['expansionTermLimit']]
        result = {'selected': selected, 'unresolved': unresolved, 'references': references[:128],
                  'referenceCount': len(references), 'expansionTerms': expansion, 'vectors': vectors,
                  'structure': structure_features(text), 'scope': 'dictionary-derived proxy; not actual archive CDISS'}
        self.cache[text] = result
        return result


def mean_vector(interpretation):
    values = interpretation['vectors']
    if not values:
        return None
    # Deduplicate concepts, not roots; a root collision cannot merge identities.
    unique = {row['conceptId']: row['position8'] for row in values}
    return [math.fsum(v[i] for v in unique.values()) / len(unique) for i in range(8)]


def cosine(left, right):
    if left is None or right is None:
        return None
    denom = math.sqrt(math.fsum(x*x for x in left) * math.fsum(x*x for x in right))
    return math.fsum(a*b for a, b in zip(left, right)) / denom if denom else None


def features(query, chunk, interpreter):
    query_info = interpreter.interpret(query)
    text = chunk['text']
    chunk_info = interpreter.interpret(text)
    selected = query_info['selected']
    evidence_terms = sorted({term for sense in selected for term in content(sense['definition'])} - content(query))
    matched = sorted(set(evidence_terms) & content(text))
    sense_score = len(matched) / len(evidence_terms) if evidence_terms else 0
    # Scope is measured in a bounded window around matching query terms, not
    # silently attributed to an unrelated sentence elsewhere in a chunk.
    qwords = content(query)
    spans, seen = [], set()
    for match in re.finditer(r"[^\W_]+(?:['’][^\W_]+)*", text, re.UNICODE):
        term = match.group().casefold()
        if term in qwords and term not in seen and len(spans) < 8:
            spans.append(match.span())
            seen.add(term)
    windows = ' '.join(text[max(0, start-80):min(len(text), end+80)] for start, end in spans)
    qstruct, cstruct = query_info['structure'], structure_features(windows)
    role_match = any(role in cstruct['roles'] for role in qstruct['roles'])
    role_reverse = any([role[2], role[1], role[0]] in cstruct['roles'] for role in qstruct['roles'])
    return {'sense': sense_score, 'senseEvidenceTerms': evidence_terms, 'matchedSenseTerms': matched,
            'negationMismatch': bool(spans) and qstruct['negated'] != cstruct['negated'],
            'conditionMismatch': bool(spans) and qstruct['conditional'] != cstruct['conditional'],
            'role': int(role_match) - int(role_reverse), 'queryStructure': qstruct,
            'candidateStructure': cstruct, 'scopeWindows': windows,
            'geometryCosine': cosine(mean_vector(query_info), mean_vector(chunk_info)),
            'queryConceptIds': sorted({r['conceptId'] for r in query_info['vectors']}),
            'candidateConceptIds': sorted({r['conceptId'] for r in chunk_info['vectors']}),
            'qualification': 'Derived proposed features; geometry is a dictionary proxy, not observed word-shape similarity'}


def added_score(method, item):
    sense = CONFIG['senseWeight'] * item['sense']
    structure = (-CONFIG['negationMismatchPenalty'] * item['negationMismatch']
                 - CONFIG['conditionMismatchPenalty'] * item['conditionMismatch']
                 + CONFIG['roleMatchWeight'] * item['role'])
    geometry = CONFIG['geometryWeight'] * max(0, item['geometryCosine'] or 0)
    return {'baseline': 0, 'reference_expansion': 0, 'sense_domain': sense,
            'structure': structure, 'geometry': geometry, 'combined': sense + structure + geometry}[method]


def citation(chunk):
    return {key: chunk[key] for key in ('chunkId', 'kind', 'source', 'scope', 'threadId', 'messageId',
                                      'noteId', 'start', 'end', 'messageSha256', 'excerptSha256')}


def message_identity(chunk):
    if chunk['kind'] == 'note':
        return ('note', chunk['scope'], chunk['noteId'])
    return (chunk['source'], chunk['threadId'], chunk['messageId'])


def explanation(query, chunk, method, feature, base_score, expansion_rank, maximum=1):
    ref = CONFIG['referenceRrfWeight']/(60+expansion_rank) if expansion_rank and method in ('reference_expansion', 'combined') else 0
    value = {'schema': 'bomb-code/recall-explanation/v1', 'querySha256': recall.digest(query),
             'citation': citation(chunk), 'method': method, 'configSha256': recall.digest(CONFIG),
             'features': feature, 'baselineRrf': base_score, 'expansionRank': expansion_rank,
             'contribution': added_score(method, feature),
             'normalizationMaximum': maximum, 'referenceRrf': ref,
             'finalScore': base_score/maximum + ref/maximum + added_score(method, feature),
             'sourceTextSha256': recall.digest(chunk['text']), 'notice': recall.NOTICE}
    value['receiptSha256'] = recall.digest(value)
    return value


def verify_explanation(value, query, chunk, interpreter, baseline_score=None, expansion_rank=None, maximum=None):
    receipt = dict(value)
    provided = receipt.pop('receiptSha256', None)
    if provided != recall.digest(receipt) or value.get('configSha256') != recall.digest(CONFIG):
        raise ValueError('Explanation receipt/config changed')
    if value.get('querySha256') != recall.digest(query) or value.get('citation') != citation(chunk):
        raise ValueError('Explanation query/citation changed')
    if recall.digest(chunk['text']) != chunk['excerptSha256'] or value.get('sourceTextSha256') != recall.digest(chunk['text']):
        raise ValueError('Explanation source excerpt changed')
    expected = features(query, chunk, interpreter)
    if value.get('features') != expected or value.get('contribution') != added_score(value['method'], expected):
        raise ValueError('Explanation contributed feature changed or missing')
    if baseline_score is None or maximum is None:
        raise ValueError('Independent ranking components required for verification')
    reconstructed = explanation(query, chunk, value['method'], expected, baseline_score, expansion_rank, maximum)
    if value != reconstructed:
        raise ValueError('Explanation ranking component or final score changed')
    return True


class Evaluator:
    def __init__(self, panel, notes, dictionary, client=None):
        self.panel, self.notes = panel, notes
        self.client = client or CachedLocalModel()
        self.interpreter = Interpreter(dictionary)

    def rank(self, query):
        # Deliberately accepts only a string. Gold, rationale, hypothesis, IDs,
        # labels and source selection are never visible to the ranker.
        if not isinstance(query, str) or not 1 <= len(query) <= 2000:
            raise ValueError('Invalid bounded query')
        started = time.monotonic()
        embedding_requests_before = getattr(self.client, 'requests', 0)
        embedding_hits_before = getattr(self.client, 'cache_hits', 0)
        baseline = SEARCH100(self.panel, self.notes, self.client, {'query': query, 'limit': 100})
        original20 = recall.search(self.panel, self.notes, self.client, {'query': query, 'limit': 20})
        if baseline['hits'][:20] != original20['hits']:
            raise ValueError('Adapted baseline fails exact top20 parity')
        info = self.interpreter.interpret(query)
        expansion_query = query + ' ' + ' '.join(info['expansionTerms'])
        with contextlib.closing(recall.readonly(self.panel / 'memory-recall/recall.sqlite')) as db:
            expanded = recall.lexical(db, expansion_query, limit=100) if info['expansionTerms'] else []
            base = {hit['chunkId']: hit for hit in baseline['hits']}
            records = dict(base)
            for cid in expanded:
                if cid not in records:
                    records[cid] = json.loads(db.execute('SELECT data FROM chunks WHERE chunk_id=?', (cid,)).fetchone()[0])
        expansion_ranks = {cid: rank for rank, cid in enumerate(expanded, 1)}
        maximum = max((hit['rankScore'] for hit in baseline['hits']), default=1)
        calculated = {cid: features(query, chunk, self.interpreter) for cid, chunk in records.items()}
        ranked = {}
        for method in METHODS:
            ids = records if method in ('reference_expansion', 'combined') else base
            values = []
            for cid in ids:
                base_score = base.get(cid, {}).get('rankScore', 0)
                reference = CONFIG['referenceRrfWeight'] / (60 + expansion_ranks[cid]) if cid in expansion_ranks and method in ('reference_expansion', 'combined') else 0
                score = base_score / maximum + reference / maximum + added_score(method, calculated[cid])
                values.append((score, cid))
            values.sort(key=lambda row: (-row[0], row[1]))
            ranked[method] = []
            for index, (score, cid) in enumerate(values):
                value = {'chunkId': cid, 'score': score}
                if index < 10:
                    bscore = base.get(cid, {}).get('rankScore', 0)
                    erank = expansion_ranks.get(cid)
                    receipt = explanation(query, records[cid], method, calculated[cid], bscore, erank, maximum)
                    verify_explanation(receipt, query, records[cid], self.interpreter, bscore, erank, maximum)
                    value['explanation'] = receipt
                ranked[method].append(value)
        # Reuse the installed evidence validator for all displayed explanations:
        # original message metadata, complete-message hash, exact span, chunk
        # identity and excerpt hash. One excerpt keeps packet size bounded.
        shown = {hit['chunkId'] for values in ranked.values() for hit in values[:10]}
        for cid in sorted(shown):
            recall.evidence(self.panel, self.notes, self.client, {'generation': baseline['generation'], 'chunkIds': [cid]})
        duration = time.monotonic() - started
        if duration > 180:
            raise TimeoutError('Per-query 180 second budget exceeded; result excluded')
        return {'methods': ranked, 'queryInterpretation': info, 'elapsedSeconds': duration,
                'timingMeaning': CONFIG['timingMeaning'],
                'embeddingRequests': getattr(self.client, 'requests', 0) - embedding_requests_before,
                'embeddingCacheHits': getattr(self.client, 'cache_hits', 0) - embedding_hits_before,
                'verifiedDisplayedExcerpts': len(shown),
                'model': baseline['model'],
                'baselineCandidateIds': list(base), 'candidateCount': len(records),
                'baselineStatus': {key: baseline.get(key) for key in ('status', 'generation', 'vectorStatus', 'vectorCount', 'pendingVectors', 'vectorCandidatesScanned', 'vectorScanLimited', 'vectorError')},
                'baselineTop20Parity': True, 'explanations': {cid: citation(chunk) for cid, chunk in records.items()}}


def metrics(ids, targets, negatives):
    ranks = [index for index, cid in enumerate(ids, 1) if cid in targets]
    return {'recallAt5': len(set(ids[:5]) & targets) / len(targets),
            'recallAt10': len(set(ids[:10]) & targets) / len(targets),
            'mrr': 1 / min(ranks) if ranks else 0,
            'mrrAt10': 1/min(ranks) if ranks and min(ranks) <= 10 else 0,
            'hitAt5': int(bool(set(ids[:5]) & targets)), 'hitAt10': int(bool(set(ids[:10]) & targets)),
            'wrongSenseAt5': len(set(ids[:5]) & negatives),
            'firstRelevantRank': min(ranks) if ranks else None}


def private_write(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2, allow_nan=False)


def bounded_read(path, maximum):
    with open(path, 'rb') as stream:
        raw = stream.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError('Input file exceeds bounded read budget')
    return raw


def validate_frozen_sources(payload, panel, notes, client):
    """Join case metadata after freezing, separately from query-only ranking."""
    frozen = payload.get('integrity')
    if not isinstance(frozen, dict) or frozen.get('unchanged') is not True or frozen.get('before') != frozen.get('after'):
        raise ValueError('Frozen case source integrity missing or changed')
    expected = frozen['after']
    if expected != {'archive': sha_file(panel/'history/library.sqlite'), 'index': sha_file(panel/'memory-recall/recall.sqlite')}:
        raise ValueError('Frozen target corpus no longer matches prepared cases')
    status = recall.status_unlocked(panel, notes, client)
    recall.ensure_ready(status)
    with contextlib.closing(recall.readonly(panel/'memory-recall/recall.sqlite')) as db:
        for case in payload['cases']:
            source = case.get('source')
            if not isinstance(source, dict) or source.get('chunkId') not in case['targetChunkIds']:
                raise ValueError('Frozen case source identity missing')
            for cid in case['targetChunkIds']:
                row = db.execute('SELECT data FROM chunks WHERE chunk_id=?', (cid,)).fetchone()
                if row is None:
                    raise ValueError('Frozen target chunk missing')
                chunk = json.loads(row[0])
                if cid == source['chunkId']:
                    actual = {'chunkId': cid, 'excerptHash': chunk['excerptSha256'], 'messageId': chunk['messageId'],
                              'source': chunk['source'], 'span': [chunk['start'], chunk['end']], 'threadId': chunk['threadId']}
                    if source != actual or case.get('selectedText') != chunk['text']:
                        raise ValueError('Frozen source metadata, span, hash or selected text changed')
                recall.evidence(panel, notes, client, {'generation': status['generation'], 'chunkIds': [cid]})
    return True


def integrity(panel):
    paths = [panel / 'history/library.sqlite', panel / 'memory-recall/recall.sqlite']
    result = {}
    for path in paths:
        # SHM is transient SQLite coordination, not source content; readonly
        # readers may update reader slots. Preserve data files and WAL bytes.
        for member in (path, Path(str(path)+'-wal')):
            if member.exists():
                result[str(member)] = {'bytes': member.stat().st_size, 'sha256': sha_file(member)}
    return result


def run(cases_path, output, panel, notes, cases_sha256=None):
    cases_path, output = Path(cases_path).resolve(), Path(output).resolve()
    repo = Path(__file__).resolve().parents[1]
    if cases_path.is_relative_to(repo) or output.is_relative_to(repo):
        raise ValueError('Private cases and outputs must remain outside this repository')
    if output.exists():
        raise ValueError('Output must be a new empty directory')
    raw = bounded_read(cases_path, 1024 * 1024)
    if cases_sha256 is not None and hashlib.sha256(raw).hexdigest() != cases_sha256:
        raise ValueError('Case file changed since independent freeze')
    payload = json.loads(raw)
    cases = payload.get('cases')
    if not isinstance(cases, list) or not 1 <= len(cases) <= 40:
        raise ValueError('Expected 1–40 preregistered cases')
    ids = set()
    for case in cases:
        if not isinstance(case, dict) or not isinstance(case.get('id'), str) or not re.fullmatch('[A-Za-z0-9_-]{1,80}', case['id']) or case['id'] in ids:
            raise ValueError('Case identities invalid')
        ids.add(case['id'])
        if not isinstance(case.get('query'), str) or not 1 <= len(case['query']) <= 2000:
            raise ValueError('Case query invalid')
        for key in ('targetChunkIds', 'wrongSenseChunkIds'):
            value = case.get(key, [] if key == 'wrongSenseChunkIds' else None)
            if not isinstance(value, list) or (key == 'targetChunkIds' and not value) or any(not isinstance(v, str) or not re.fullmatch('[0-9a-f]{64}', v) for v in value):
                raise ValueError('Case gold IDs invalid')
    os.mkdir(output, 0o700)
    before = integrity(panel)
    script_pins = {name: sha_file(Path(__file__).with_name(name)) for name in ('recall_experiment.py', 'word_dictionary.py', 'memory_recall.py')}
    private_write(output / 'preregistration.json', {'casesSha256': hashlib.sha256(raw).hexdigest(), 'config': CONFIG, 'configSha256': recall.digest(CONFIG), 'baselineSourceSha256': BASELINE_SOURCE_SHA, 'scriptPins': script_pins, 'caseCount': len(cases)})
    dictionary = word_dictionary.Dictionary.load()
    reference_before = dict(dictionary.reference)
    evaluator = Evaluator(panel, notes, dictionary)
    validate_frozen_sources(payload, panel, notes, evaluator.client)
    results, errors = [], []
    def deadline(_signum, _frame):
        # A distinct exception is not swallowed as an embedding OSError.
        raise QueryDeadline('Per-query hard 180 second budget exceeded')
    previous_handler = signal.signal(signal.SIGALRM, deadline)
    initial_status = recall.status_unlocked(panel, notes, evaluator.client)
    recall.ensure_ready(initial_status)
    with contextlib.closing(recall.readonly(panel / 'memory-recall/recall.sqlite')) as db:
        target_messages = {}
        for case in cases:
            for cid in case['targetChunkIds']:
                row = db.execute('SELECT data FROM chunks WHERE chunk_id=?', (cid,)).fetchone()
                chunk = json.loads(row[0]) if row else None
                target_messages[cid] = message_identity(chunk) if chunk else ()
        target_vectors = {cid: bool(db.execute('SELECT 1 FROM vectors WHERE chunk_id=?', (cid,)).fetchone()) for cid in target_messages}
        matched_vectors = {cid: bool(db.execute('SELECT 1 FROM vectors WHERE chunk_id=? AND model_digest=? AND dimension=?',
                              (cid, initial_status['model']['digest'], initial_status['model']['dimension'])).fetchone())
                           and initial_status['embeddingBasisCompatible'] for cid in target_messages}
    for case in cases:
        try:
            signal.alarm(180)
            ranked = evaluator.rank(case['query'])
            signal.alarm(0)
            targets, negatives = set(case['targetChunkIds']), set(case.get('wrongSenseChunkIds', []))
            values = {method: metrics([hit['chunkId'] for hit in ranked['methods'][method]], targets, negatives) for method in METHODS}
            message_gold = {target_messages[cid] for cid in targets if target_messages[cid]}
            for method in METHODS:
                message_ids = []
                for hit in ranked['methods'][method]:
                    cite = ranked['explanations'][hit['chunkId']]
                    identity = message_identity(cite)
                    if identity not in message_ids:
                        message_ids.append(identity)
                ranks = [i for i, key in enumerate(message_ids, 1) if key in message_gold]
                values[method]['messageMrr'] = 1/min(ranks) if ranks else 0
                values[method]['messageRecallAt10'] = len(set(message_ids[:10]) & message_gold)/len(message_gold) if message_gold else 0
            result = {'caseId': case['id'], 'hypothesis': case.get('hypothesis'), 'query': case['query'], 'metrics': values,
                      'targetEligibility': {'present': sum(bool(target_messages[cid]) for cid in targets), 'withStoredVector': sum(target_vectors[cid] for cid in targets),
                                            'withCurrentModelBasisVector': sum(matched_vectors[cid] for cid in targets), 'targetCount': len(targets)},
                      'baselineCandidateRecallAt100': len(set(ranked['baselineCandidateIds']) & targets)/len(targets), 'ranking': ranked}
            private_write(output / (case['id'] + '.json'), result)
            results.append(result)
        except (OSError, ValueError, recall.RecallError, QueryDeadline) as exc:
            errors.append({'caseId': case['id'], 'error': str(exc)})
        finally:
            signal.alarm(0)
    signal.signal(signal.SIGALRM, previous_handler)
    after = integrity(panel)
    model_after = recall.pin(evaluator.client)
    # Reload verifies original pinned manifest, graph, model and SenseSnap bytes.
    del evaluator, dictionary
    reference_after = word_dictionary.Dictionary.load().reference
    aggregate = {'schema': 'bomb-code/recall-experiment-summary/v1', 'attemptedCases': len(cases), 'completedCases': len(results), 'excludedCases': errors,
                 'casesSha256': hashlib.sha256(raw).hexdigest(), 'config': CONFIG, 'baselineSourceSha256': BASELINE_SOURCE_SHA,
                 'scriptPins': script_pins,
                 'sourceIntegrity': before == after, 'referenceIntegrity': reference_before == reference_after,
                 'modelBefore': initial_status['model']['digest'], 'modelAfter': model_after,
                 'modelIntegrity': bool(model_after) and model_after == initial_status['model']['digest'],
                 'baselineCandidateRecallAt100': math.fsum(r['baselineCandidateRecallAt100'] for r in results)/len(results) if results else None,
                 'methods': {}, 'hypotheses': {}, 'qualification': CONFIG['scope']}
    aggregate['queryCoverage'] = {'selectedSenses': sum(len(r['ranking']['queryInterpretation']['selected']) for r in results),
                                  'unresolvedForms': sum(len(r['ranking']['queryInterpretation']['unresolved']) for r in results),
                                  'queriesWithExpansion': sum(bool(r['ranking']['queryInterpretation']['expansionTerms']) for r in results),
                                  'queriesWithGeometry': sum(bool(r['ranking']['queryInterpretation']['vectors']) for r in results),
                                  'meanElapsedSeconds': math.fsum(r['ranking']['elapsedSeconds'] for r in results)/len(results) if results else None}
    aggregate['pairedBaselineComparisons'] = {method: {metric: {
        'gain': sum(r['metrics'][method][metric] > r['metrics']['baseline'][metric] for r in results),
        'loss': sum(r['metrics'][method][metric] < r['metrics']['baseline'][metric] for r in results),
        'tie': sum(r['metrics'][method][metric] == r['metrics']['baseline'][metric] for r in results)}
        for metric in ('mrrAt10', 'hitAt5', 'hitAt10')} for method in METHODS if method != 'baseline'}
    for group, selected in [('all', results)] + [(str(h), [r for r in results if str(r['hypothesis']) == str(h)]) for h in sorted({str(r['hypothesis']) for r in results})]:
        item = {method: {metric: math.fsum(r['metrics'][method][metric] for r in selected)/len(selected) for metric in ('recallAt5', 'recallAt10', 'mrr', 'mrrAt10', 'hitAt5', 'hitAt10', 'wrongSenseAt5', 'messageMrr', 'messageRecallAt10')} for method in METHODS} if selected else {}
        if group == 'all':
            aggregate['methods'] = item
        else:
            aggregate['hypotheses'][group] = {'cases': len(selected), 'methods': item}
    aggregate['allAttemptMissSensitivity'] = {method: {metric: (None if metric == 'wrongSenseAt5' else value * len(results)/len(cases)) for metric, value in metrics_row.items()}
                                             for method, metrics_row in aggregate['methods'].items()}
    private_write(output / 'integrity.json', {'before': before, 'after': after, 'referenceBefore': reference_before, 'referenceAfter': reference_after})
    private_write(output / 'summary.json', aggregate)
    if not aggregate['sourceIntegrity'] or not aggregate['referenceIntegrity'] or not aggregate['modelIntegrity']:
        raise ValueError('Source, reference or local model changed/unavailable; results remain unqualified')
    return aggregate


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cases', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--panel', default=str(Path.home()/'.grok/control-panel'))
    parser.add_argument('--notes', help='Private existing notes snapshot JSON list (default empty list)')
    parser.add_argument('--cases-sha256', help='Independent frozen case file SHA256')
    options = parser.parse_args()
    notes = recall.notes_snapshot({'notes': json.loads(bounded_read(options.notes, recall.MAX_INPUT)) if options.notes else []})
    summary = run(options.cases, options.output, Path(options.panel), notes, options.cases_sha256)
    print(json.dumps(summary, separators=(',', ':'), allow_nan=False))

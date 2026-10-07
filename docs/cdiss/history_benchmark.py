#!/usr/bin/env python3
"""Read-only private-history grounding benchmark. No linguistic gold or providers."""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import time
import uuid

# Import frozen reference modules without creating or refreshing source caches.
sys.dont_write_bytecode = True

EXPECTED_MANIFEST = '4d466d7d8e830f6a3330e619a497f99aa3b6fa6c7439432c610b1f3485498e83'
SOURCES = ('chatgpt', 'claude_code', 'codex', 'grok')
WORD = re.compile(r"[^\W_]+(?:['’][^\W_]+)?", re.UNICODE)
SECRET = re.compile(r'(?i)(?:\b(?:sk|ghp|gho|github_pat|xai|AKIA)[-_][A-Za-z0-9_-]{12,}|\bBearer\s+\S{12,}|(?:api[_ -]?key|access[_ -]?token|password|secret)\s*[:=]\s*[\"\']?\S{10,}|\bAKIA[A-Z0-9]{16}\b|-----BEGIN [A-Z ]*PRIVATE KEY-----)')
CURRENT_TASK = re.compile(r'(?i)(?:\bcdiss\b|history[-_ ]benchmark|use (?:imported|old) threads (?:as|for) test)')
CONNECTIVITY = re.compile(r'(?i)(?:connectivity|connection test|smoke test|reply (?:only|exactly)|respond (?:only|exactly)|just (?:say|respond)|test (?:the )?(?:api|connection))')
WRAPPER = re.compile(r'(?i)(?:<environment_context>|#\s*AGENTS\.md|<INSTRUCTIONS>|<system(?:_reminder)?>|<developer>|You are (?:Codex|an? AI)|Message Type:\s*(?:NEW_TASK|MESSAGE)|\[Request interrupted by user\])')


def sha_bytes(data):
    return hashlib.sha256(data).hexdigest()


def file_sha(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for b in iter(lambda: f.read(1024 * 1024), b''):
            h.update(b)
    return h.hexdigest()


def private_json(path, value):
    # Output directory is private, and files are created with owner-only access.
    encoded = (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':')) + '\n').encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as f:
        f.write(encoded)
    return sha_bytes(encoded)


def excerpt(text):
    """Return exact original characters, with an explicit selection explanation."""
    start = len(text) - len(text.lstrip())
    stripped_end = len(text.rstrip())
    words = list(WORD.finditer(text, start, stripped_end))
    end = min(stripped_end, start + 500)
    if len(words) > 48:
        end = min(end, words[47].end())
    if end < stripped_end:
        # Prefer an intact sentence or paragraph; otherwise retain a word boundary.
        boundary = list(re.finditer(r'[.!?](?:\s|$)|\n\s*\n', text[start:end]))
        if boundary and boundary[-1].end() >= 60:
            end = start + boundary[-1].start() + 1
        else:
            within = [m.end() for m in words if m.end() <= end]
            end = within[-1] if within else end
    while end > start and text[end - 1].isspace():
        end -= 1
    passage = text[start:end]
    reason = 'whole message except surrounding whitespace' if end == stripped_end else 'exact leading excerpt, ending at sentence/paragraph/word boundary; remainder explicitly excluded'
    return passage, [start, end], reason


def check_sidecars(database):
    result = {}
    for suffix in ['-wal', '-journal']:
        path = Path(str(database)+suffix)
        size = path.stat().st_size if path.exists() else 0
        if size:
            raise RuntimeError('immutable benchmark requires absent or empty SQLite WAL/journal')
        result[suffix] = {'exists': path.exists(), 'bytes': size}
    return result


def select_cases(conn, count):
    rejected = collections.Counter()
    buckets = {source: collections.defaultdict(list) for source in SOURCES}
    total = collections.Counter()
    rows = conn.execute('SELECT t.source,t.id,t.origin_id,t.coverage,t.parent_id,m.message_id,m.seq,m.at,m.text,m.truncated FROM threads t JOIN messages m ON m.thread_id=t.id WHERE m.role=? ORDER BY t.source,t.id,m.seq,m.message_id', ('user',))
    for source, tid, origin, coverage, parent, mid, seq, at, text, truncated in rows:
        total[source] += 1
        if source not in SOURCES:
            rejected['excluded-source'] += 1
            continue
        if parent or '/subagent/' in str(origin).lower() or '/subagent/' in str(tid).lower():
            rejected['subagent-thread'] += 1
            continue
        if truncated:
            rejected['truncated-source-message'] += 1
            continue
        if CURRENT_TASK.search(text):
            rejected['current-task-prompt'] += 1
            continue
        if CONNECTIVITY.search(text):
            rejected['connectivity-or-response-fixture'] += 1
            continue
        if SECRET.search(text):
            rejected['credential-shaped-content'] += 1
            continue
        if WRAPPER.search(text):
            rejected['instruction-or-environment-wrapper'] += 1
            continue
        if text.lstrip().startswith(('```', '<tool', '<function')):
            rejected['code-or-tool-block'] += 1
            continue
        passage, span, reason = excerpt(text)
        if len(passage) < 40 or len(list(WORD.finditer(passage))) < 6:
            rejected['too-short-for-passages'] += 1
            continue
        ident = f'{source}\0{tid}\0{mid}'
        row = {'source': source, 'threadId': tid, 'originThreadId': origin, 'messageId': mid, 'seq': seq, 'at': at,
               'sourceCoverage': coverage, 'messageSha256': sha_bytes(text.encode()), 'messageCharacters': len(text),
               'characterSpan': span, 'excerptReason': reason, 'passage': passage, 'inputSha256': sha_bytes(passage.encode()),
               'contextGroup': tid, 'selectionHash': sha_bytes(ident.encode())}
        buckets[source][tid].append(row)
    chosen = []
    # Each source gets equal allocation. Round one samples distinct main threads;
    # round two retains another message where available for within-thread continuity.
    quotas = {s: count // len(SOURCES) + (i < count % len(SOURCES)) for i, s in enumerate(SOURCES)}
    eligible = {}
    for source in SOURCES:
        groups = buckets[source]
        for group in groups.values():
            group.sort(key=lambda r: r['selectionHash'])
        order = sorted(groups, key=lambda t: sha_bytes((source+'\0'+t).encode()))
        eligible[source] = {'threads': len(groups), 'messages': sum(map(len, groups.values()))}
        active = order[:max(1, (quotas[source] + 1) // 2)]
        selected = []
        for n in range(1000):
            for tid in active:
                if len(groups[tid]) > n and len(selected) < quotas[source]:
                    selected.append(groups[tid][n])
            if len(selected) >= quotas[source]:
                break
            if all(len(groups[t]) <= n + 1 for t in active):
                active = order
                # Rebuild using the same deterministic ordering without duplicates.
                seen = {r['selectionHash'] for r in selected}
                for tid in active:
                    for r in groups[tid]:
                        if r['selectionHash'] not in seen and len(selected) < quotas[source]:
                            selected.append(r); seen.add(r['selectionHash'])
                break
        chosen.extend(selected)
    chosen.sort(key=lambda r: (SOURCES.index(r['source']), r['threadId'], r['seq'], r['messageId']))
    for i, row in enumerate(chosen):
        row['caseId'] = f'history-{i+1:03}'
    return chosen, {'userMessageCounts': dict(total), 'eligible': eligible, 'excluded': dict(rejected),
                    'requested': count, 'selected': len(chosen), 'selection': 'equal source allocation; SHA-256 order; main-thread groups; chronological order inside selected groups'}


def lexical_outline(passage, version):
    atoms = [{'id': f'a{i}', 'surface': m.group(), 'span': [m.start(), m.end()], 'lemma': None, 'lemma_reason': None, 'pos': 'unknown'} for i, m in enumerate(WORD.finditer(passage))]
    return {'schema': version, 'atoms': atoms, 'readings': [{'id': 'r1', 'summary': 'Mechanical lexical inventory; no semantic reading asserted',
        'speech_act': 'unknown', 'events': [], 'event_links': [], 'references': [], 'perspective_observations': [],
        'unresolved': ['Negation, modality, roles, references, intent and contextual sense selection are deliberately unevaluated.'],
        'reason': 'Authored exact Unicode spans with all exact-surface dictionary senses retained; not linguistic gold or a model interpretation.'}]}


def self_test():
    import unittest

    class Boundaries(unittest.TestCase):
        def test_exact_unicode_excerpt(self):
            text = '  Café 🧭 choices remain visible. ' + ('Repeated context words. ' * 50)
            passage, span, reason = excerpt(text)
            self.assertEqual(passage, text[span[0]:span[1]])
            self.assertLessEqual(len(passage), 500)
            self.assertLessEqual(len(list(WORD.finditer(passage))), 48)
            self.assertIn('excerpt', reason)

        def test_wal_journal_guard(self):
            import tempfile
            with tempfile.TemporaryDirectory() as folder:
                database = Path(folder)/'library.sqlite'
                database.write_bytes(b'unchanged test source')
                original = file_sha(database)
                self.assertEqual(check_sidecars(database)['-wal']['bytes'], 0)
                wal = Path(str(database)+'-wal')
                wal.write_bytes(b'')
                self.assertEqual(check_sidecars(database)['-wal']['bytes'], 0)
                wal.write_bytes(b'nonempty')
                with self.assertRaises(RuntimeError):
                    check_sidecars(database)
                wal.unlink()
                Path(str(database)+'-journal').write_bytes(b'nonempty')
                with self.assertRaises(RuntimeError):
                    check_sidecars(database)
                self.assertEqual(file_sha(database), original)

        def test_selection_excludes_sensitive_and_wrapper_material(self):
            conn = sqlite3.connect(':memory:')
            conn.execute('create table threads(id,source,origin_id,coverage,parent_id)')
            conn.execute('create table messages(thread_id,message_id,role,seq,at,text,truncated)')
            conn.execute("insert into threads values('main','codex','original','local transcript','')")
            conn.execute("insert into threads values('child','codex','child-origin','local transcript','main')")
            conn.execute("insert into threads values('orphan','codex','run/subagent/orphan','local transcript','')")
            good = 'Please compare the project choices and preserve the unresolved questions.'
            records = [('main','good',good,0), ('child','agent',good,0), ('orphan','orphan-agent',good,0),
                       ('main','wrapper','<environment_context> '+good,0),
                       ('main','secret',good+' api_key=sk-abcdefghijklmnopqrstuv',0),
                       ('main','truncated',good,1), ('main','current',good+' CDISS',0),
                       ('main','fixture',good+' Reply exactly with READY.',0)]
            for i, (tid, mid, text, truncated) in enumerate(records):
                conn.execute('insert into messages values(?,?,?,?,?,?,?)',(tid,mid,'user',i,'',text,truncated))
            selected, coverage = select_cases(conn, 4)
            self.assertEqual([r['messageId'] for r in selected], ['good'])
            for key in ['subagent-thread','instruction-or-environment-wrapper','credential-shaped-content','truncated-source-message','current-task-prompt','connectivity-or-response-fixture']:
                self.assertEqual(coverage['excluded'][key], 2 if key=='subagent-thread' else 1)
            self.assertTrue(SECRET.search('AKIAABCDEFGHIJKLMNOP'))
            again, _ = select_cases(conn, 4)
            self.assertEqual(selected, again)
            conn.close()

    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Boundaries))
    if not result.wasSuccessful():
        raise SystemExit(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--database', type=Path)
    parser.add_argument('--reference', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--count', type=int, default=48)
    parser.add_argument('--probe', type=Path, help='Already-built history_probe executable; avoids build side effects')
    parser.add_argument('--select-only', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if any(p is None for p in [args.database, args.reference, args.output]):
        parser.error('database, reference and output paths are required for a corpus run')
    if not 1 <= args.count <= 128:
        parser.error('count must be 1..128')
    repo = Path(__file__).resolve().parents[2]
    output = args.output.resolve()
    if output == repo or repo in output.parents:
        parser.error('private output must be outside the git checkout')
    if output.exists() and any(output.iterdir()):
        parser.error('use a new empty output directory; no overwrite of private artifacts')
    output.mkdir(parents=True, mode=0o700, exist_ok=True)
    os.chmod(output, 0o700)
    os.umask(0o077)
    database = args.database.resolve()
    sidecars_before = check_sidecars(database)
    before = {'bytes': database.stat().st_size, 'mtimeNs': database.stat().st_mtime_ns, 'sha256': file_sha(database)}
    conn = sqlite3.connect(database.as_uri()+'?mode=ro&immutable=1', uri=True)
    conn.execute('PRAGMA query_only=ON')
    integrity = conn.execute('PRAGMA quick_check').fetchone()[0]
    if integrity != 'ok':
        raise RuntimeError('source SQLite quick_check failed')
    cases, coverage = select_cases(conn, args.count)
    corpus_hash = private_json(output/'corpus.json', {'schema': 'bomb-code/private-history-corpus/v1', 'coverage': coverage, 'cases': cases})
    conn.close()
    results = []
    reference_pins = {}
    if not args.select_only:
        reference = args.reference.resolve()
        sys.path.insert(0, str(reference))
        graph_path = reference/'semantic_e8/outputs/aligned_graph.json'
        model_path = reference/'semantic_e8/outputs/model.json'
        graph = json.loads(graph_path.read_text())
        model = json.loads(model_path.read_text())
        manifest_path = reference/'round_trip_experiment/PACKAGE_MANIFEST.json'
        manifest_hash = file_sha(manifest_path)
        if manifest_hash != EXPECTED_MANIFEST:
            raise RuntimeError('manifest does not match installed Joe frozen source pin')
        manifest_members = json.loads(manifest_path.read_text())['files']
        for item in manifest_members:
            member = (reference/item['path']).resolve()
            if reference not in member.parents or not member.is_file() or member.stat().st_size != item['bytes'] or file_sha(member) != item['sha256']:
                raise RuntimeError('frozen source manifest member failed integrity check')
        from semantic_e8.interpretation import prepare_request, ground_outline, bind_interpretation, OUTLINE_VERSION, SELECTION_VERSION
        implementation_pins = {name: file_sha(reference/'semantic_e8'/name) for name in ['interpretation.py', 'usage.py', 'geometry.py', 'sense_snap.py']}
        reference_pins = {'implementationHashes': implementation_pins, 'manifestSha256': manifest_hash, 'verifiedManifestMembers': len(manifest_members), 'graphFileSha256': file_sha(graph_path), 'modelFileSha256': file_sha(model_path)}
        bindings_dir = output/'bindings'; bindings_dir.mkdir(mode=0o700)
        for row in cases:
            start = time.perf_counter()
            try:
                context = {'task_anchor': 'Read-only imported-thread lexical grounding benchmark', 'host': {'thread_id': row['threadId']},
                           'benchmark_kind': 'lexical-only authored outline; no linguistic gold', 'context_group': row['contextGroup']}
                request = prepare_request(graph, model, 'eng', row['passage'], context)
                outline = lexical_outline(row['passage'], OUTLINE_VERSION)
                packet = ground_outline(graph, model, request, outline)
                selection = {'schema': SELECTION_VERSION, 'readings': [{'id': 'r1', 'sense_bindings': [
                    {'atom_id': item['atom']['id'], 'sense_ids': [c['sense']['id'] for c in item['candidates']],
                     'reason': 'Retain every exact-surface alternative without contextual selection'} for item in packet['atoms']],
                    'uncertainty': ['No linguistic or human sense-selection accuracy evaluated']}], 'reason': 'Mechanical all-alternatives inventory; not a linguistic interpretation'}
                binding = bind_interpretation(graph, model, request, outline, selection, 'private-history-lexical-benchmark')
                if binding['receipt']['implementation_hashes'] != implementation_pins:
                    raise RuntimeError('binder implementation differs from verified frozen source')
                result = {'schema': 'bomb-code/joe-result/v1', 'requestId': str(uuid.uuid5(uuid.NAMESPACE_URL, row['selectionHash'])),
                    'threadId': row['threadId'], 'sentence': row['passage'], 'language': 'eng', 'provider': 'local-authored-lexical-benchmark',
                    'model': 'exact-surface-all-alternatives-no-linguistic-gold/v1', 'interpretation': {'binding': binding},
                    'reference': {'manifestSha256': manifest_hash}, 'referenceRequested': {'manifestSha256': manifest_hash},
                    'authority': {'toolsDispatched': False, 'approvalsGranted': False, 'memoryCommitted': False}}
                path = bindings_dir/(row['caseId']+'.json')
                digest = private_json(path, result)
                atoms = binding['readings'][0]['bound_usage']['atoms']
                native_activations = binding['readings'][0]['e8_activations']
                # Entire original native geometry objects, not merely positions.
                geometry_digest = sha_bytes(json.dumps(native_activations, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode())
                native_candidate_ids = [[c['sense']['id'] for c in a['all_source_candidates']] for a in atoms]
                results.append({'caseId': row['caseId'], 'status': 'bound', 'path': str(path), 'bindingSha256': digest,
                    'source': row['source'], 'threadId': row['threadId'], 'inputSha256': row['inputSha256'], 'atomCount': len(atoms),
                    'candidateCount': sum(map(len, native_candidate_ids)), 'ambiguousAtoms': sum(len(ids)>1 for ids in native_candidate_ids),
                    'unmappedAtoms': sum(not ids for ids in native_candidate_ids),
                    'nativeContextPinCount': sum(c['origin']=='context-pin' for a in atoms for c in a['sense_snap']['centers']),
                    'senseSnapCenters': sum(len(a['sense_snap']['centers']) for a in atoms), 'geometryCount': len(native_activations),
                    'geometryDigest': geometry_digest, 'elapsedMs': (time.perf_counter()-start)*1000})
            except (ValueError, RuntimeError) as err:
                # Failure is kept verbatim in private receipts; no automatic pruning/retry.
                results.append({'caseId': row['caseId'], 'status': 'rejected', 'source': row['source'], 'threadId': row['threadId'],
                    'inputSha256': row['inputSha256'], 'reason': str(err), 'elapsedMs': (time.perf_counter()-start)*1000})
        private_json(output/'binding-results.json', results)
        if args.probe:
            run = subprocess.run([str(args.probe.resolve()), str(output/'binding-results.json')], capture_output=True, text=True)
            if run.returncode:
                raise RuntimeError('CDISS probe failed: '+run.stderr[:2000])
            probe_result = json.loads(run.stdout)
            private_json(output/'cdiss-results.json', probe_result)
        for item in manifest_members:
            member = (reference/item['path']).resolve()
            if member.stat().st_size != item['bytes'] or file_sha(member) != item['sha256']:
                raise RuntimeError('frozen source manifest member changed during benchmark')
        for name, path in [('graphFileSha256', graph_path), ('modelFileSha256', model_path), ('manifestSha256', manifest_path)]:
            if file_sha(path) != reference_pins[name]:
                raise RuntimeError('source reference changed during benchmark')
    sidecars_after = check_sidecars(database)
    after = {'bytes': database.stat().st_size, 'mtimeNs': database.stat().st_mtime_ns, 'sha256': file_sha(database)}
    unchanged = before == after
    report = {'schema': 'bomb-code/private-history-benchmark/v1', 'verificationClass': 'actual private passages with authored lexical inventory; not linguistic gold',
              'providerCalls': 0, 'authority': {'toolsDispatched': False, 'approvalsGranted': False, 'memoryCommitted': False},
              'sourceDatabase': {'before': before, 'after': after, 'unchanged': unchanged, 'quickCheck': integrity, 'sidecarsBefore': sidecars_before, 'sidecarsAfter': sidecars_after},
              'corpusSha256': corpus_hash, 'coverage': coverage, 'referencePins': reference_pins,
              'bindingStatuses': dict(collections.Counter(r['status'] for r in results)),
              'limitations': ['Short selected passages cannot represent the full library or languages.',
                  'ChatGPT imports may include branches without message-parent metadata; sequence adjacency is not proof of a single conversation path.',
                  'Exact source identities and explicit unknowns support lexical candidate retrieval; relevant-memory ranking, domain clarification quality and actual progress remain unmeasured.',
                  'Subagents, wrappers, credential-shaped text, truncated messages and long remainders are excluded explicitly.',
                  'Native exact-form lookup retains ambiguity and unknowns; no inflection/lemma guessing or contextual sense-selection is performed.',
                  'Context pins are not exercised in lexical cases without an explicit imported pin overlay; context equality with zero pins is vacuous.',
                  'Empty semantic event frames cannot test negation, roles, modality, reference resolution, intent or true completion.',
                  'Source-backed coordinates prove geometric retention, not semantic distance usefulness or human agreement.']}
    private_json(output/'receipt.json', report)
    if not unchanged:
        raise RuntimeError('history database changed during benchmark')
    print(json.dumps({'selected': len(cases), 'sources': dict(collections.Counter(r['source'] for r in cases)),
                      'statuses': report['bindingStatuses'], 'sourceDatabaseUnchanged': unchanged, 'providerCalls': 0}))


if __name__ == '__main__':
    main()

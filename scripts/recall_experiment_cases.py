#!/usr/bin/env python3
"""Freeze independently authored private relevance cases; never run retrieval.

Candidate selection is deterministically stratified by hypothesis and source.
The authoring input and output must remain outside the Git checkout. All original
databases are read-only and immutable; source bytes are hashed before and after.
"""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sqlite3
import sys

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('history_case_policy', REPO / 'docs/cdiss/history_benchmark.py')
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)
SCHEMA = 'bomb-code/recall-experiment-cases/v1'
LABEL_AUTHORITY = 'agent-authored relevance judgments; not human gold'
SEED = 'recall-preregister-v1'
SENSE = re.compile(r'\b(memory|cloud|root|bank|model|plane|voice|sphere|lens|branch|port|kernel|dream|alignment|bond|drop|pillar|cell|block)\b', re.I)
STRUCTURE = re.compile(r'\b(not|never|without|unless|if|only|instead|until)\b', re.I)
EXTRA_WRAPPER = re.compile(r'(?i)(?:BOMB_NATIVE_CONTINUITY|test token|reply with exactly|reply with the test token|<[^>]+>|# In app browser|# Files mentioned by the user|## My request|questionItemId|\[User attached)')
FIXTURE_TITLE = re.compile(r'(?i)(?:test token|local-command|continuity fixture)')
PASTED_LOG = re.compile(r'(?im)(?:fatal:|^\S+@\S+.*%|exit status \d+)')


def sha(value):
    return hashlib.sha256(value.encode()).hexdigest()


def file_sha(path):
    return policy.file_sha(path)


def readonly(path):
    policy.check_sidecars(path)
    database = sqlite3.connect(path.resolve().as_uri() + '?mode=ro&immutable=1', uri=True)
    database.execute('PRAGMA query_only=ON')
    return database


def private_json(path, value):
    path = path.resolve()
    if path == REPO or REPO in path.parents:
        raise ValueError('private cases must remain outside Git')
    path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
    os.chmod(path.parent, 0o700)
    encoded = (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + '\n').encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as target:
        target.write(encoded)
    return hashlib.sha256(encoded).hexdigest()


def verify_chunk(chunk, history):
    original = history.execute('''SELECT m.text,m.truncated,t.parent_id,t.origin_id,t.source
      FROM messages m JOIN threads t ON t.id=m.thread_id
      WHERE m.thread_id=? AND m.message_id=?''', (chunk['threadId'], chunk['messageId'])).fetchone()
    if not original:
        return 'missing-original'
    full, truncated, parent, origin, source = original
    if parent or '/subagent/' in str(origin).lower() or truncated:
        return 'non-main-or-truncated'
    if any(pattern.search(full) for pattern in [policy.SECRET, policy.CURRENT_TASK, policy.CONNECTIVITY, policy.WRAPPER]) or EXTRA_WRAPPER.search(full) or FIXTURE_TITLE.search(chunk['title']):
        return 'benchmark-policy'
    if full.lstrip().startswith(('```', '<tool', '<function')) or PASTED_LOG.search(full):
        return 'code-or-tool'
    if source != chunk['source'] or sha(full) != chunk['messageSha256'] or full[chunk['start']:chunk['end']] != chunk['text'] or sha(chunk['text']) != chunk['excerptSha256']:
        raise ValueError('chunk does not match original message and hashes')
    return None


def select(index, history):
    candidates = collections.defaultdict(list)
    excluded = collections.Counter()
    for raw, in index.execute('SELECT data FROM chunks ORDER BY source,thread_id,chunk_id'):
        chunk = json.loads(raw)
        if chunk['source'] not in policy.SOURCES or chunk['role'] != 'user':
            continue
        reason = verify_chunk(chunk, history)
        if reason:
            excluded[reason] += 1
            continue
        original_length = history.execute('SELECT length(text) FROM messages WHERE thread_id=? AND message_id=?', (chunk['threadId'], chunk['messageId'])).fetchone()[0]
        if original_length > 1000 or chunk['start'] != 0 or not 80 <= len(chunk['text']) <= 900 or not 12 <= len(policy.WORD.findall(chunk['text'])) <= 145:
            excluded['length-or-continuation'] += 1
            continue
        chunk['selectionHash'] = sha(SEED + '\0' + chunk['source'] + '\0' + chunk['threadId'] + '\0' + chunk['messageId'])
        for hypothesis in ('sense', 'reference', 'structure'):
            if hypothesis == 'sense' and not SENSE.search(chunk['text']):
                continue
            if hypothesis == 'structure' and not STRUCTURE.search(re.sub(r'(?i)\bas if\b', ' ', chunk['text'])):
                continue
            candidates[hypothesis, chunk['source']].append(chunk)
    selected, used = [], set()
    for hypothesis in ('sense', 'reference', 'structure'):
        for source in policy.SOURCES:
            groups = collections.defaultdict(list)
            for chunk in candidates[hypothesis, source]:
                groups[chunk['threadId']].append(chunk)
            threads = sorted(groups, key=lambda thread: sha(SEED + '\0' + hypothesis + '\0' + source + '\0' + thread))
            count = 0
            for thread in threads:
                for chunk in sorted(groups[thread], key=lambda item: item['selectionHash']):
                    if chunk['chunkId'] not in used:
                        selected.append(dict(chunk, hypothesis=hypothesis, id=f'{hypothesis}-{source}-{count + 1}'))
                        used.add(chunk['chunkId'])
                        count += 1
                        break
                if count == 2:
                    break
            if count != 2:
                raise ValueError('insufficient eligible source/hypothesis stratum')
    return selected, dict(excluded)


def freeze(selected, labels, index, history):
    if not isinstance(labels, dict) or set(labels) != {chunk['id'] for chunk in selected}:
        raise ValueError('labels must match the independently selected IDs exactly')
    cases = []
    for chunk in selected:
        label = labels[chunk['id']]
        if set(label) != {'query', 'rationale'} or not all(isinstance(value, str) and 8 <= len(value) <= 2000 for value in label.values()):
            raise ValueError('only bounded natural-language query and rationale are permitted')
        if any(value in label['query'] for value in [chunk['chunkId'], chunk['threadId'], chunk['messageId']]):
            raise ValueError('query contains a target identity')
        row = index.execute('SELECT data FROM chunks WHERE chunk_id=?', (chunk['chunkId'],)).fetchone()
        if row is None or json.loads(row[0]) != {key: value for key, value in chunk.items() if key not in ('selectionHash', 'hypothesis', 'id')}:
            raise ValueError('selected source chunk changed')
        if verify_chunk(chunk, history):
            raise ValueError('selected source no longer passes policy')
        cases.append({'id': chunk['id'], 'hypothesis': chunk['hypothesis'], 'query': label['query'],
            'targetChunkIds': [chunk['chunkId']], 'rationale': label['rationale'],
            'source': {'source': chunk['source'], 'threadId': chunk['threadId'], 'messageId': chunk['messageId'],
                       'chunkId': chunk['chunkId'], 'span': [chunk['start'], chunk['end']],
                       'excerptHash': chunk['excerptSha256']},
            'selectedText': chunk['text'], 'labelAuthority': LABEL_AUTHORITY})
    return cases


def main():
    if sys.argv[1:] == ['--self-test']:
        self_test()
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('select', 'freeze'))
    parser.add_argument('--panel', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--labels', type=Path)
    args = parser.parse_args()
    paths = {'index': args.panel / 'memory-recall/recall.sqlite', 'archive': args.panel / 'history/library.sqlite'}
    before = {name: file_sha(path) for name, path in paths.items()}
    index, history = readonly(paths['index']), readonly(paths['archive'])
    try:
        selected, excluded = select(index, history)
        selection = {'seed': SEED, 'requested': 24, 'selected': len(selected), 'sources': list(policy.SOURCES),
                     'hypotheses': ['sense', 'reference', 'structure'], 'perSourcePerHypothesis': 2,
                     'distinctThreads': len({row['threadId'] for row in selected}), 'excluded': excluded,
                     'method': 'SHA-256 thread order then message order; one message per thread within each stratum; distinct target chunks; no retrieval outputs inspected'}
        if args.action == 'select':
            data = {'schema': SCHEMA, 'candidates': selected, 'selection': selection}
        else:
            if args.labels is None:
                parser.error('freeze requires independently authored private labels')
            if REPO in args.labels.resolve().parents:
                raise ValueError('private labels cannot be inside Git')
            labels = json.loads(args.labels.read_text())
            data = {'schema': SCHEMA, 'cases': freeze(selected, labels, index, history), 'selection': selection,
                    'labelAuthority': LABEL_AUTHORITY,
                    'limitations': ['Selected targets are agent-authored known positives, not exhaustive relevance judgments.',
                                    'Domain cases include project-specific senses and proper names; no multilingual or unseen-domain accuracy claim.',
                                    'Structure examples contain different speech acts and scope; negation presence is not automatically contradiction.',
                                    'Explanation faithfulness can be checked on all cases, but explanation usefulness needs human review.']}
    finally:
        index.close()
        history.close()
    after = {name: file_sha(path) for name, path in paths.items()}
    if before != after:
        raise ValueError('source database bytes changed during case preparation')
    for path in paths.values():
        policy.check_sidecars(path)
    data['integrity'] = {'before': before, 'after': after, 'unchanged': True, 'noNetwork': True,
                         'readOnlyOriginals': True, 'labelInputHash': file_sha(args.labels) if args.labels else None}
    output_sha = private_json(args.output, data)
    print(json.dumps({'path': str(args.output), 'sha256': output_sha, 'count': len(selected), 'distinctThreads': selection['distinctThreads'], 'unchanged': True}))


def self_test():
    import tempfile
    import unittest

    class Boundaries(unittest.TestCase):
        def test_original_unicode_source_and_changed_excerpt(self):
            history = sqlite3.connect(':memory:')
            history.execute('CREATE TABLE messages(thread_id,message_id,text,truncated)')
            history.execute('CREATE TABLE threads(id,parent_id,origin_id,source)')
            text = 'Please retain the café 🧭 rendering but do not repeat the old implementation.'
            history.execute('INSERT INTO messages VALUES(?,?,?,0)', ('t', 'm', text))
            history.execute("INSERT INTO threads VALUES('t','','original','codex')")
            chunk = {'threadId': 't', 'messageId': 'm', 'source': 'codex', 'title': 'A project',
                     'text': text, 'start': 0, 'end': len(text), 'messageSha256': sha(text), 'excerptSha256': sha(text)}
            self.assertIsNone(verify_chunk(chunk, history))
            with self.assertRaises(ValueError):
                verify_chunk(dict(chunk, text='Changed source'), history)
            history.close()

        def test_nonempty_sidecar_and_readonly(self):
            with tempfile.TemporaryDirectory() as folder:
                path = Path(folder) / 'archive.sqlite'
                connection = sqlite3.connect(path)
                connection.execute('CREATE TABLE evidence(value)')
                connection.commit()
                connection.close()
                before = file_sha(path)
                connection = readonly(path)
                with self.assertRaises(sqlite3.OperationalError):
                    connection.execute('INSERT INTO evidence VALUES(1)')
                connection.close()
                self.assertEqual(file_sha(path), before)
                Path(str(path) + '-wal').write_bytes(b'pending writes')
                with self.assertRaises(RuntimeError):
                    readonly(path)

        def test_private_exclusive_output_and_git_rejection(self):
            with tempfile.TemporaryDirectory() as folder:
                target = Path(folder) / 'private' / 'cases.json'
                private_json(target, {'case': 'synthetic'})
                self.assertEqual(target.stat().st_mode & 0o777, 0o600)
                self.assertEqual(target.parent.stat().st_mode & 0o777, 0o700)
                with self.assertRaises(FileExistsError):
                    private_json(target, {'case': 'overwrite'})
            with self.assertRaises(ValueError):
                private_json(REPO / 'cases-never-written.json', {})

        def test_wrapper_fixture_and_structure_cues(self):
            for value in ['<in-app-browser-context>ambient</in-app-browser-context>',
                          'Reply with the test token', '[User attached 1 file; file contents were not included]']:
                self.assertIsNotNone(EXTRA_WRAPPER.search(value))
            self.assertIsNotNone(STRUCTURE.search('Do not overwrite unless necessary.'))
            self.assertIsNone(STRUCTURE.search(re.sub(r'(?i)\bas if\b', ' ', 'Explain it as if to a mentor.')))

    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Boundaries))
    if not result.wasSuccessful():
        raise SystemExit(1)


if __name__ == '__main__':
    main()

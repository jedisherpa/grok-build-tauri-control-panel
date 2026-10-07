"""Supplementary individual-channel audit after the frozen primary run.

No scoring, label or configuration changes; local read-only existing search.
"""
import argparse
import ast
import contextlib
import hashlib
import inspect
import json
import math
import os
from pathlib import Path
import signal
import statistics
import time

import memory_recall as recall
import recall_experiment as primary
import word_dictionary


def search_union():
    source = inspect.getsource(recall.search)
    tree = ast.parse(source)
    matches = []
    for node in ast.walk(tree):
        if (isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == 'integer'
                and len(node.args) == 5 and isinstance(node.args[4], ast.Constant) and node.args[4].value == 'limit'):
            if not isinstance(node.args[3], ast.Constant) or node.args[3].value != 20:
                raise ValueError('Baseline bound changed')
            matches.append(node)
    if len(matches) != 1:
        raise ValueError('Exactly one baseline public bound required')
    matches[0].args[3] = ast.Constant(value=200)
    namespace = dict(vars(recall))
    exec(compile(ast.fix_missing_locations(tree), '<existing-search-limit200>', 'exec'), namespace)
    return namespace['search'], recall.digest(source)


def reference_pins(reference):
    paths = {reference['graphPath']: reference['graphFileSha256'], reference['modelPath']: reference['modelFileSha256'],
             word_dictionary.MANIFEST_PATH: reference['manifestSha256'], **reference['senseSnapImplementationHashes']}
    actual = {path: primary.sha_file(word_dictionary.REFERENCE_ROOT/path) for path in paths}
    if actual != paths:
        raise ValueError('Original pinned dictionary references changed')
    return actual


def audit(cases_path, run_folder, output, panel):
    repo = Path(__file__).resolve().parents[1]
    if any(path.resolve().is_relative_to(repo) for path in (cases_path, run_folder, output)):
        raise ValueError('Private data must remain outside Git')
    if output.exists():
        raise ValueError('Supplement output must be new')
    summary = json.loads(primary.bounded_read(run_folder/'summary.json', 1024*1024))
    raw = primary.bounded_read(cases_path, 1024*1024)
    if hashlib.sha256(raw).hexdigest() != summary['casesSha256']:
        raise ValueError('Frozen cases changed')
    payload = json.loads(raw)
    cases = payload['cases']
    if len(cases) != 24:
        raise ValueError('Expected exact original 24 cases')
    search, source_sha = search_union()
    if source_sha != summary['baselineSourceSha256']:
        raise ValueError('Original search source changed')
    for name, sha in summary['scriptPins'].items():
        if primary.sha_file(Path(__file__).with_name(name)) != sha:
            raise ValueError('Frozen primary script changed')
    client = primary.CachedLocalModel()
    if recall.pin(client) != summary['modelBefore']:
        raise ValueError('Original local model changed')
    primary.validate_frozen_sources(payload, panel, [], client)
    saved_integrity = json.loads(primary.bounded_read(run_folder/'integrity.json', 1024*1024))
    before = primary.integrity(panel)
    if before != saved_integrity['after']:
        raise ValueError('Primary corpus changed before supplement')
    refs_before = reference_pins(saved_integrity['referenceAfter'])
    os.mkdir(output, 0o700)
    primary.private_write(output/'preregistration.json', {'supplementAfterPrimaryRun': True, 'casesSha256': summary['casesSha256'],
                         'baselineSourceSha256': source_sha, 'supplementScriptSha256': primary.sha_file(__file__),
                         'method': 'Existing search only public limit20 to200; native lexical100 and vector100 ranks unchanged'})
    results, errors = [], []
    def deadline(_signum, _frame):
        raise primary.QueryDeadline('Supplement per-query 180 second budget exceeded')
    previous = signal.signal(signal.SIGALRM, deadline)
    for case in cases:
        try:
            signal.alarm(180)
            start = time.monotonic()
            result = search(panel, [], client, {'query': case['query'], 'limit': 200})
            if result['vectorStatus'] != 'complete' or result.get('vectorError') or result['vectorScanLimited']:
                raise ValueError('Individual vector channel incomplete/unavailable')
            hits = result['hits']
            frozen = json.loads(primary.bounded_read(run_folder/(case['id']+'.json'), 32*1024*1024))['ranking']['methods']['baseline']
            maximum = max((h['rankScore'] for h in hits), default=1)
            for limit in (20, 100):
                actual = [(h['chunkId'], h['rankScore']/maximum) for h in hits[:limit]]
                expected = [(h['chunkId'], h['score']) for h in frozen[:limit]]
                if len(actual) != len(expected) or any(a[0] != b[0] or not math.isclose(a[1], b[1], abs_tol=1e-12) for a, b in zip(actual, expected)):
                    raise ValueError('Full-union search differs from frozen hybrid top20/top100')
            channels = {name: [h['chunkId'] for h in sorted((h for h in hits if h[key] is not None), key=lambda h:(h[key],h['chunkId']))]
                        for name, key in [('lexical', 'keywordRank'), ('vector', 'vectorRank')]}
            channels['hybrid100'] = [h['chunkId'] for h in hits[:100]]
            channels['union200'] = [h['chunkId'] for h in hits]
            chunks = {h['chunkId']: h for h in hits}
            with contextlib.closing(recall.readonly(panel/'memory-recall/recall.sqlite')) as db:
                gold_chunks = [json.loads(db.execute('SELECT data FROM chunks WHERE chunk_id=?',(cid,)).fetchone()[0]) for cid in case['targetChunkIds']]
            targets = set(case['targetChunkIds'])
            message_targets = {primary.message_identity(h) for h in gold_chunks}
            metric = {}
            for name, ids in channels.items():
                item = primary.metrics(ids, targets, set())
                item.pop('wrongSenseAt5')
                item['candidateCoverage'] = len(set(ids)&targets)/len(targets)
                identities = list(dict.fromkeys(primary.message_identity(chunks[cid]) for cid in ids))
                message = primary.metrics(identities, message_targets, set())
                item['message'] = {k: message[k] for k in ('hitAt5','hitAt10','mrrAt10')}
                metric[name] = item
            record = {'caseId': case['id'], 'metrics': metric, 'channels': channels,
                      'frozenTop20Top100Parity': True, 'elapsedSeconds': time.monotonic()-start,
                      'vectorCandidatesScanned': result['vectorCandidatesScanned']}
            primary.private_write(output/(case['id']+'.json'), record)
            results.append(record)
        except (OSError, ValueError, recall.RecallError, primary.QueryDeadline) as exc:
            errors.append({'caseId': case['id'], 'error': str(exc)})
        finally:
            signal.alarm(0)
    signal.signal(signal.SIGALRM, previous)
    after = primary.integrity(panel)
    refs_after = reference_pins(saved_integrity['referenceAfter'])
    model_after = recall.pin(client)
    output_summary = {'schema': 'bomb-code/recall-channel-supplement/v1', 'supplementAfterPrimaryRun': True,
                      'attemptedCases': len(cases), 'completedCases': len(results), 'excludedCases': errors,
                      'casesSha256': summary['casesSha256'], 'sourceIntegrity': before == after,
                      'referenceIntegrity': refs_before == refs_after, 'modelIntegrity': model_after == summary['modelBefore'],
                      'channels': {}, 'qualification': 'Known-positive case coverage; no exhaustive negatives. '
                      'Union200 order is hybrid RRF and coverage ceiling, not a separate semantic method. Supplement follows frozen primary results.'}
    for name in ('lexical', 'vector', 'hybrid100', 'union200'):
        rows = [r['metrics'][name] for r in results]
        output_summary['channels'][name] = {k: statistics.mean(r[k] for r in rows) if rows else None for k in ('hitAt5','hitAt10','mrrAt10','candidateCoverage')}
        output_summary['channels'][name]['message'] = {k: statistics.mean(r['message'][k] for r in rows) if rows else None for k in ('hitAt5','hitAt10','mrrAt10')}
    primary.private_write(output/'integrity.json', {'before': before, 'after': after, 'referenceBefore': refs_before, 'referenceAfter': refs_after})
    primary.private_write(output/'summary.json', output_summary)
    if not all(output_summary[k] for k in ('sourceIntegrity','referenceIntegrity','modelIntegrity')):
        raise ValueError('Supplement integrity changed; results unqualified')
    return output_summary


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('cases','run','output','panel'):
        parser.add_argument('--'+name, type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(audit(args.cases,args.run,args.output,args.panel)))

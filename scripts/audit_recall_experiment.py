"""Independent saved-run arithmetic/citation audit; no ranking or provider calls.

Private inputs and outputs stay outside Git. Source fidelity of complete messages
was checked by the native evidence validator during ranking; this pass also checks
the current index excerpt and all displayed score receipts independently.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sqlite3
import statistics


def digest(value):
    if not isinstance(value, str):
        value = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'))
    return hashlib.sha256(value.encode()).hexdigest()


def read(path):
    return json.loads(bounded_bytes(path))


def bounded_bytes(path):
    maximum = 32 * 1024 * 1024
    with path.open('rb') as stream:
        raw = stream.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError('Audit input exceeds budget')
    return raw


def validate_citation(provided, source):
    keys = ('chunkId', 'kind', 'source', 'scope', 'threadId', 'messageId',
            'noteId', 'start', 'end', 'messageSha256', 'excerptSha256')
    expected = {key: source[key] for key in keys}
    if provided != expected:
        raise ValueError('Incomplete or changed citation')


def measure(ids, targets):
    ranks = [i for i, value in enumerate(ids, 1) if value in targets]
    rank = min(ranks, default=math.inf)
    return {'hitAt5': int(rank <= 5), 'hitAt10': int(rank <= 10),
            'mrrAt10': 1 / rank if rank <= 10 else 0,
            'mrr': 1 / rank if ranks else 0}


def audit(folder, cases_path, index):
    if not __debug__:
        raise RuntimeError('This research validator requires Python without -O')
    summary = read(folder / 'summary.json')
    cases_raw = bounded_bytes(cases_path)
    assert hashlib.sha256(cases_raw).hexdigest() == summary['casesSha256']
    cases = json.loads(cases_raw)['cases']
    assert len(cases) == summary['attemptedCases'] == summary['completedCases'] == 24
    assert not summary['excludedCases']
    assert all(summary[k] for k in ('sourceIntegrity', 'referenceIntegrity', 'modelIntegrity'))
    config = summary['config']
    methods = list(summary['methods'])
    aggregates = {m: [] for m in methods}
    messages = {m: [] for m in methods}
    receipts = 0
    unique = set()
    elapsed = []
    rows = []
    db = sqlite3.connect(index.resolve().as_uri() + '?mode=ro', uri=True)
    chunks = {}
    def chunk(cid):
        if cid not in chunks:
            row = db.execute('SELECT data FROM chunks WHERE chunk_id=?', (cid,)).fetchone()
            assert row is not None
            chunks[cid] = json.loads(row[0])
        return chunks[cid]
    def identity(value):
        assert value['kind'] == 'history'
        return (value['source'], value['threadId'], value['messageId'])
    for case in cases:
        result = read(folder / (case['id'] + '.json'))
        assert result['query'] == case['query']
        ranking = result['ranking']
        assert ranking['baselineTop20Parity']
        assert ranking['baselineStatus']['vectorStatus'] == 'complete'
        assert ranking['baselineStatus']['vectorCandidatesScanned'] == 29755
        assert not ranking['baselineStatus']['vectorScanLimited']
        gold = set(case['targetChunkIds'])
        assert len(gold) == 1
        message_gold = {identity(chunk(cid)) for cid in gold}
        row = {'caseId': case['id'], 'hypothesis': case['hypothesis'], 'methods': {}}
        for method in methods:
            hits = ranking['methods'][method]
            ids = [hit['chunkId'] for hit in hits]
            assert len(ids) == len(set(ids))
            assert hits == sorted(hits, key=lambda hit: (-hit['score'], hit['chunkId']))
            metric = measure(ids, gold)
            for key, value in metric.items():
                assert math.isclose(result['metrics'][method][key], value, abs_tol=1e-12)
            aggregates[method].append(metric)
            message_ids = list(dict.fromkeys(identity(chunk(cid)) for cid in ids))
            message_metric = measure(message_ids, message_gold)
            messages[method].append(message_metric)
            row['methods'][method] = metric
            for hit in hits[:10]:
                receipt = hit['explanation']
                payload = dict(receipt)
                sha = payload.pop('receiptSha256')
                assert digest(payload) == sha
                assert receipt['querySha256'] == digest(case['query'])
                assert receipt['configSha256'] == digest(config)
                assert receipt['method'] == method
                source = chunk(hit['chunkId'])
                validate_citation(receipt['citation'], source)
                assert digest(source['text']) == receipt['sourceTextSha256'] == source['excerptSha256']
                f = receipt['features']
                evidence = set(f['senseEvidenceTerms'])
                expected_sense = len(set(f['matchedSenseTerms'])) / len(evidence) if evidence else 0
                assert set(f['matchedSenseTerms']) <= evidence
                assert math.isclose(expected_sense, f['sense'], abs_tol=1e-12)
                sense = config['senseWeight'] * f['sense']
                structure = (-config['negationMismatchPenalty'] * f['negationMismatch']
                             -config['conditionMismatchPenalty'] * f['conditionMismatch']
                             +config['roleMatchWeight'] * f['role'])
                geometry = config['geometryWeight'] * max(0, f['geometryCosine'] or 0)
                contribution = {'baseline': 0, 'reference_expansion': 0, 'sense_domain': sense,
                                'structure': structure, 'geometry': geometry,
                                'combined': sense + structure + geometry}[method]
                expansion = receipt['expansionRank']
                reference = (config['referenceRrfWeight'] / (config['rrfK'] + expansion)
                             if expansion and method in ('reference_expansion', 'combined') else 0)
                score = ((receipt['baselineRrf'] + reference) / receipt['normalizationMaximum']
                         + contribution)
                assert math.isclose(contribution, receipt['contribution'], abs_tol=1e-12)
                assert math.isclose(reference, receipt['referenceRrf'], abs_tol=1e-12)
                assert math.isclose(score, receipt['finalScore'], abs_tol=1e-12)
                assert math.isclose(score, hit['score'], abs_tol=1e-12)
                receipts += 1
                unique.add(hit['chunkId'])
        rows.append(row)
        elapsed.append(ranking['elapsedSeconds'])
    db.close()
    totals = {m: {key: statistics.mean(row[key] for row in values) for key in values[0]}
              for m, values in aggregates.items()}
    for method, values in totals.items():
        for key, value in values.items():
            assert math.isclose(value, summary['methods'][method][key], abs_tol=1e-12)
    message_totals = {m: {key: statistics.mean(row[key] for row in values) for key in values[0]}
                      for m, values in messages.items()}
    gates = {}
    for family, method in [('sense', 'sense_domain'), ('reference', 'reference_expansion'), ('structure', 'structure')]:
        selected = [r for r in rows if r['hypothesis'] == family]
        delta = statistics.mean(r['methods'][method]['mrrAt10'] - r['methods']['baseline']['mrrAt10'] for r in selected)
        hit_delta = statistics.mean(r['methods'][method]['hitAt10'] - r['methods']['baseline']['hitAt10'] for r in selected)
        gates[family] = {'cases': len(selected), 'mrrAt10Delta': delta, 'hitAt10Delta': hit_delta,
                         'passesExploratoryGate': delta >= .05 and hit_delta >= 0}
    return {'schema': 'bomb-code/independent-recall-audit/v1', 'checksPassed': True,
            'completedCases': len(rows), 'displayedReceiptsVerified': receipts,
            'distinctDisplayedExcerpts': len(unique), 'chunkMetrics': totals,
            'messageCollapsedMetrics': message_totals, 'familyGates': gates,
            'timingSeconds': {'median': statistics.median(elapsed),
                             'p95NearestRank': sorted(elapsed)[math.ceil(.95 * len(elapsed)) - 1],
                             'sumQueryWorkload': sum(elapsed)},
            'wrongSensePrecision': None, 'humanExplanationUsefulness': None,
            'qualification': 'Known positive labels supplied by an independent agent; no exhaustive negatives. '
                             'Displayed feature fidelity was also recomputed during the frozen run; this independent '
                             'pass audits saved arithmetic, citations, current excerpts and metrics. Timings cover '
                             'combined work and parity scans, not per-method latency.', 'pairedCases': rows}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--index', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = audit(args.run, args.cases, args.index)
    with args.output.open('x') as stream:
        args.output.chmod(0o600)
        stream.write(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({k: result[k] for k in ('checksPassed', 'completedCases', 'displayedReceiptsVerified', 'distinctDisplayedExcerpts', 'familyGates', 'timingSeconds')}))

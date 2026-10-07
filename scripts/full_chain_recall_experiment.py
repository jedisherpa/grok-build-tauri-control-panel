"""PM evaluation of frozen authored full-chain contracts and saved proposals.

No scoring configuration or label tuning. Executes a bounded native probe only;
raw packets and output stay private. Original references/receipts are read-only.
"""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from structured_recall_questions import prepare_questions


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def read(path, maximum=32 * 1024 * 1024):
    with path.open('rb') as stream:
        raw = stream.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError('Experiment input exceeds budget')
    return json.loads(raw)


def private(path, value):
    with path.open('x') as stream:
        path.chmod(0o600)
        json.dump(value, stream, ensure_ascii=False, indent=2, allow_nan=False)
        stream.write('\n')


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'))


def selected_bag(profile, concepts=False, paired=False):
    output = []
    for atom in profile['readings'][0]['occurrences']:
        if concepts:
            output.extend(atom['assertedConceptIds'])
            continue
        for source in atom['selectedSourceBindings']:
            if paired:
                output.extend((source['sense']['id'], cid) for cid in source['concept_ids'])
            else:
                output.append(source['sense']['id'])
    return sorted(output)


def verify_preservation(profile, packet):
    readings = packet['interpretation']['binding']['readings']
    assert len(profile['readings']) == len(readings)
    inventory = {a['atom']['id']: a['candidates'] for a in packet['interpretation']['binding']['source_packet']['atoms']}
    for native, result in zip(readings, profile['readings']):
        assert result['readingId'] == native['id']
        assert result['frame'] == native['frame']
        assert result['geometry'] == native['e8_activations']
        assert result['uncertainty'] == native['selection_uncertainty']
        assert len(result['occurrences']) == len(native['bound_usage']['atoms'])
        for original, occurrence in zip(native['bound_usage']['atoms'], result['occurrences']):
            assert occurrence['surface'] == original['atom']['surface']
            concepts = sorted({a['concept_id'] for b in original['selected_source_bindings'] for a in b['alignments'] if a['sense_id']==b['sense']['id'] and a['concept_id'] in b['concept_ids'] and a['kind']=='equivalent' and a['asserted'] is True})
            assert occurrence['assertedConceptIds'] == concepts
            assert occurrence['atomId'] == original['atom']['id']
            assert occurrence['span'] == original['atom']['span']
            assert occurrence['selectedSourceBindings'] == original['selected_source_bindings']
            assert occurrence['allSourceAlternatives'] == inventory[occurrence['atomId']]
            assert occurrence['senseSnap'] == original['sense_snap']


def contract(expected, result, candidate_id):
    q = result['queryProfile']
    c = result['candidateProfiles'][0]['profile']
    pair = result['retrieval']['candidateComparisons'][0]['comparison']['readingPairs'][0]
    event = pair['eventLocal']
    values = {}
    for key in expected:
        if key == 'sameSourceSenseMultiset':
            values[key] = selected_bag(q) == selected_bag(c)
        elif key == 'sameAssertedConceptMultiset':
            values[key] = selected_bag(q, concepts=True) == selected_bag(c, concepts=True)
        elif key == 'sameSelectedSourceConceptMultiset':
            values[key] = selected_bag(q, paired=True) == selected_bag(c, paired=True)
        elif key in ('sameConceptEvents', 'sameConceptEventsIgnoringLinks'):
            values[key] = sorted(canonical(e['canonical']) for e in event['queryEvents']) == sorted(canonical(e['canonical']) for e in event['candidateEvents'])
        elif key == 'sameFlattenedSelectedRoleBag':
            def roles(events):
                return sorted(canonical(r) for e in events for r in e['canonical']['roles'])
            values[key] = roles(event['queryEvents']) == roles(event['candidateEvents'])
        elif key in ('sameAssertedReferences', 'sameEventLinks'):
            field = 'references' if key == 'sameAssertedReferences' else 'links'
            values[key] = event['queryCanonicalGraph'][field] == event['candidateCanonicalGraph'][field]
        elif key == 'sameContextMeetings':
            def meetings(profile):
                return sorted(center['meeting_id'] for a in profile['readings'][0]['occurrences'] for center in a['senseSnap']['centers'] if center['origin'] == 'context-pin')
            values[key] = meetings(q) == meetings(c)
        elif key == 'contextAddsDictionaryEquivalence':
            def bad(profile):
                return any(center['source_mapping_asserted'] for a in profile['readings'][0]['occurrences'] for center in a['senseSnap']['centers'] if center['origin'] == 'context-pin')
            values[key] = bad(q) or bad(c)
        elif key == 'sameRootIds':
            def roots(profile):
                return sorted(a['placement']['root_id'] for a in profile['readings'][0]['geometry'] if a.get('placement'))
            values[key] = roots(q) == roots(c)
        elif key == 'assertedEquivalentConceptCandidate':
            values[key] = candidate_id in result['retrieval']['matchedCandidateIds']
        elif key == 'repeatedChildOccurrences':
            values[key] = sum(a['surface'].casefold() == 'child' for a in q['readings'][0]['occurrences'])
        else:
            raise ValueError('Unhandled frozen contract: ' + key)
    return {'passed': values == expected, 'expected': expected, 'observed': values,
            'prototypeConceptEventDistance': event['conceptNormalizedDistance'],
            'prototypeMultiplicity': pair['multiplicity']}


def run(binary, manifest_path, expected_sha, output, saved_folder):
    if not __debug__:
        raise ValueError('Run without Python optimization')
    repo = Path(__file__).resolve().parents[1]
    if output.resolve().is_relative_to(repo) or output.exists():
        raise ValueError('Use a new private output directory outside Git')
    if sha(manifest_path) != expected_sha:
        raise ValueError('Authored case manifest changed')
    manifest = read(manifest_path, 1024 * 1024)
    folder = manifest_path.parent
    reference = Path('/Users/paulcooper/.grok/control-panel/wizard-joe/reference')
    before = {name: sha(reference / name) for name in manifest['referencePins']}
    assert before == manifest['referencePins']
    packets = {}
    for row in manifest['profiles']:
        assert Path(row['file']).name == row['file']
        assert sha(folder / row['file']) == row['sha256']
        packets[row['id']] = read(folder / row['file'])
    if len(packets) > 32:
        raise ValueError('Control profile budget exceeded')
    os.mkdir(output, 0o700)
    binary_sha = sha(binary)
    private(output / 'preregistration.json', {'manifestSha256':expected_sha, 'probeBinarySha256':binary_sha,
            'scriptSha256':sha(Path(__file__)), 'sourcePins':before, 'scoringTuned':False})
    def probe(name, query, candidates):
        payload = {'query': query, 'candidates':[{'id':cid, 'packet':packet} for cid, packet in candidates]}
        raw = json.dumps(payload, ensure_ascii=False, allow_nan=False).encode()
        if len(raw) > 32 * 1024 * 1024:
            return {'status':'unavailable', 'reason':'whole probe input budget exceeded'}
        started = time.monotonic()
        try:
            completed = subprocess.run([str(binary)], input=raw, capture_output=True, timeout=60, check=False)
            if len(completed.stdout) > 32 * 1024 * 1024:
                raise ValueError('Whole probe output budget exceeded')
            result = json.loads(completed.stdout)
            private(output / (name + '.json'), result)
            if result['status'] == 'ready':
                private(output / (name + '-questions.json'), prepare_questions(result))
            result['elapsedSeconds'] = time.monotonic() - started
            return result
        except (ValueError, subprocess.TimeoutExpired) as error:
            result = {'status':'unavailable', 'reason':str(error)}
            private(output / (name + '.json'), result)
            return result
    self_rows = []
    for cid, packet in packets.items():
        result = probe('self-' + cid, packet, [(cid,packet)])
        if result['status'] == 'ready':
            verify_preservation(result['queryProfile'], packet)
            verify_preservation(result['candidateProfiles'][0]['profile'], packet)
            comparison = result['retrieval']['candidateComparisons'][0]['comparison']
            assert len(comparison['readingPairs']) == len(packet['interpretation']['binding']['readings']) ** 2
        self_rows.append({'id':cid, 'status':result['status'], 'reason':result.get('reason'), 'allEvidenceRetained':result['status']=='ready'})
    relations = []
    for relation in manifest['expectations']:
        left, right = relation['left'], relation['right']
        if left not in packets or right not in packets:
            relations.append({'id':relation['id'], 'status':'source-binding-unavailable', 'passed':None})
            continue
        result = probe('relation-' + relation['id'], packets[left], [(right,packets[right])])
        item = {'id':relation['id'], 'status':result['status'], 'passed':None, 'reason':result.get('reason')}
        if result['status'] == 'ready':
            verify_preservation(result['queryProfile'], packets[left])
            verify_preservation(result['candidateProfiles'][0]['profile'], packets[right])
            item.update(contract(relation['expected'], result, right))
        relations.append(item)
    saved_rows = []
    files = sorted(saved_folder.glob('*.json'))
    assert len(files) <= 64
    originals = {p.name:sha(p) for p in files}
    for number, path in enumerate(files):
        packet = read(path)
        result = probe('saved-' + str(number), packet, [('same-saved-proposal',packet)])
        item = {'receiptFile':path.name, 'status':result['status'], 'reason':result.get('reason'), 'originalPreserved':sha(path)==originals[path.name]}
        if result['status'] == 'ready':
            verify_preservation(result['queryProfile'], packet)
            verify_preservation(result['candidateProfiles'][0]['profile'], packet)
            assert len(result['retrieval']['candidateComparisons'][0]['comparison']['readingPairs']) == len(packet['interpretation']['binding']['readings']) ** 2
            item['readings'] = len(result['queryProfile']['readings'])
        saved_rows.append(item)
    after = {name:sha(reference / name) for name in manifest['referencePins']}
    assert before == after
    assert sha(binary) == binary_sha
    assert sha(manifest_path) == expected_sha
    assert all(sha(folder / row['file']) == row['sha256'] for row in manifest['profiles'])
    assert all(sha(saved_folder / name) == value for name,value in originals.items())
    summary = {'schema':'bomb-code/full-chain-experiment/v1', 'manifestSha256':expected_sha,
               'profilesAttempted':len(self_rows), 'profilesReady':sum(x['status']=='ready' for x in self_rows),
               'relationsAttempted':len(relations), 'relationsPassed':sum(x['passed'] is True for x in relations),
               'relationsFailed':sum(x['passed'] is False for x in relations),
               'relationsUnavailable':sum(x['passed'] is None for x in relations),
               'profiles':self_rows, 'relations':relations, 'savedProposals':saved_rows,
               'sourceIntegrity':before==after, 'originalsPreserved':True, 'probeBinarySha256':binary_sha,
               'providerCalls':0, 'indexWrites':0,
               'qualification':'Computational source-bound authored contracts and saved proposal fidelity; no automatic language interpretation or archive relevance labels.'}
    private(output / 'summary.json', summary)
    print(json.dumps({k:v for k,v in summary.items() if k not in ('profiles','relations','savedProposals')}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--manifest-sha256', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--saved-receipts', type=Path, required=True)
    args = parser.parse_args()
    run(args.binary,args.manifest,args.manifest_sha256,args.output,args.saved_receipts)

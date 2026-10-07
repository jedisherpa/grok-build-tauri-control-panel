#!/usr/bin/env python3
"""Independently authored source-bound controls, frozen before native retrieval.

Uses the original pinned binder unchanged. No provider, embedding or archive
access. Expectations are authored representation contracts, not linguistic gold.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time

sys.dont_write_bytecode = True
import word_dictionary

SCHEMA = 'bomb-code/full-chain-authored-cases/v1'
REPO = Path(__file__).resolve().parents[1]
REFERENCE = word_dictionary.REFERENCE_ROOT
SOURCE_CHOICES = {
    'parent': 'sense:d31895532cf89acc302620b9',
    'child': 'sense:e39968d0b2783e6a67fbdea9',
    'artifact': 'sense:04720617a464b37904916ef1',
    'approve': 'sense:b3fbe1db5eec693415206ee8',
    'approve-sanction': 'sense:879041cf6dff21d3a57ce411',
    'reject': 'sense:51a80546df8675f3bb66b1ca',
    'able-skill': 'sense:f83ed5c656b77891f1849ff5',
    'able-capacity': 'sense:34be1a3ab97d53ca97de9fad',
}


def sha_file(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def private_json(path, value):
    raw = (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'), allow_nan=False) + '\n').encode()
    if len(raw) > 16 * 1024 * 1024:
        raise ValueError('Authored packet exceeds16MiB budget')
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as target:
        target.write(raw)
    return {'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw)}


def event(predicate, agent=None, patient=None, polarity='positive', modality='asserted', cues=None, event_id='e1'):
    roles = []
    if agent:
        roles.append({'role': 'agent', 'atom_id': agent})
    if patient:
        roles.append({'role': 'patient', 'atom_id': patient})
    return {'id': event_id, 'predicate': predicate, 'roles': roles, 'polarity': polarity,
            'modality': modality, 'cue_spans': cues or []}


def atom_specs(sentence, specifications):
    """Each spec names an exact occurrence, including repeated words separately."""
    atoms, choices = [], {}
    occupied = []
    for atom_id, surface, occurrence, lemma, pos, sense_id in specifications:
        matches = list(re.finditer(re.escape(surface), sentence))
        match = matches[occurrence]
        span = [match.start(), match.end()]
        if any(span[0] < end and start < span[1] for start, end in occupied):
            raise ValueError('Authored atom spans overlap')
        occupied.append(span)
        atoms.append({'id': atom_id, 'surface': surface, 'span': span, 'lemma': lemma,
                      'lemma_reason': 'Authored inflection or phrase-form choice; not automatic parsing' if lemma and lemma != surface else None,
                      'pos': pos})
        choices[atom_id] = [sense_id] if sense_id else []
    # Retain every remaining lexical occurrence even when it lacks source senses.
    for index, match in enumerate(re.finditer(r"[^\W_]+(?:['’][^\W_]+)*", sentence, re.UNICODE)):
        if any(start <= match.start() and match.end() <= end for start, end in occupied):
            continue
        atom_id = 'unselected' + str(index)
        atoms.append({'id': atom_id, 'surface': match.group(), 'span': [match.start(), match.end()],
                      'lemma': None, 'lemma_reason': None, 'pos': 'unknown'})
        choices[atom_id] = []
    atoms.sort(key=lambda item: item['span'])
    return atoms, choices


def context_for(label):
    if label is None:
        return {'provenance': 'Independent authored held-out representation control; not human intent or provider interpretation'}
    meeting = 'meet:personal:heldout-child-' + label
    return {'provenance': 'Authored contextual identity; no dictionary equivalence or approval asserted',
            'sense_snap': {'speaker': 'heldout-speaker', 'frame_id': 'heldout-frame-' + label,
                           'overlay': {'meetings': [{'id': meeting, 'gloss': 'Child in scenario ' + label,
                                                    'definition': 'Authored scenario-specific referent for the child'}],
                                       'pins': [{'id': 'pin:heldout:child-' + label, 'speaker': 'heldout-speaker',
                                                 'language': 'eng', 'lemma': 'child', 'meeting_id': meeting,
                                                 'how': 'explicit', 'frame_id': 'heldout-frame-' + label,
                                                 'evidence': 'Authored contextual identity control, not a source synonym'}],
                                       'frames': [{'id': 'heldout-frame-' + label, 'goal': 'Inspect scenario-specific child identity',
                                                   'goal_words': ['child'], 'meeting_ids': [meeting], 'members': ['heldout-speaker']}]}}}


def blueprint(case_id, sentence, specs, events, language='eng', context=None, links=None, references=None, alternatives=None):
    atoms, choices = atom_specs(sentence, specs)
    reading = {'id': 'r1', 'summary': 'Independent authored control ' + case_id, 'speech_act': 'assertion',
               'events': events, 'event_links': links or [], 'references': references or [],
               'perspective_observations': [], 'unresolved': ['Authored frames and source choices are not independent linguistic gold.'],
               'reason': 'Fixed authored representation contract before computational retrieval outputs'}
    readings = [reading]
    choice_sets = [choices]
    if alternatives:
        for atom_id, sense_id in alternatives:
            extra = copy.deepcopy(reading)
            extra['id'] = 'r' + str(len(readings) + 1)
            readings.append(extra)
            extra_choices = copy.deepcopy(choices)
            extra_choices[atom_id] = [sense_id]
            choice_sets.append(extra_choices)
    return {'id': case_id, 'sentence': sentence, 'language': language, 'atoms': atoms,
            'readings': readings, 'choices': choice_sets, 'contextLabel': context}


def plan(alias_pairs):
    s = SOURCE_CHOICES
    p = lambda form, occurrence=0: ('parent', form, occurrence, 'parent' if form != 'parent' else None, 'n', s['parent'])
    c = lambda form, occurrence=0, atom_id='child': (atom_id, form, occurrence, 'child' if form != 'child' else None, 'n', s['child'])
    h = lambda form, occurrence=0: ('approve', form, occurrence, 'approve' if form != 'approve' else None, 'v', s['approve'])
    j = lambda form: ('reject', form, 0, 'reject' if form != 'reject' else None, 'v', s['reject'])
    person = ('artifact', 'artifact', 0, None, 'n', s['artifact'])
    cases = [
        blueprint('approve-active', 'The parent approves the child.', [p('parent'), h('approves'), c('child')], [event('approve', 'parent', 'child')]),
        blueprint('approve-passive', 'The child is approved by the parent.', [p('parent'), h('approved'), c('child')], [event('approve', 'parent', 'child')]),
        blueprint('approve-reversed', 'The child approves the parent.', [p('parent'), h('approves'), c('child')], [event('approve', 'child', 'parent')]),
    ]
    text = 'The parent does not approve the child.'
    start = text.index('not')
    cases.append(blueprint('approve-negative', text, [p('parent'), h('approve'), c('child')],
                           [event('approve', 'parent', 'child', polarity='negative', cues=[{'kind': 'negation', 'surface': 'not', 'span': [start, start+3]}])]))
    text = 'If the parent approves the child.'
    cases.append(blueprint('approve-conditional', text, [p('parent'), h('approves'), c('child')],
                           [event('approve', 'parent', 'child', modality='conditional', cues=[{'kind': 'condition', 'surface': 'If', 'span': [0,2]}])]))
    multi = 'The parent approves the child, and the child rejects the artifact.'
    specs = [p('parent'), h('approves'), c('child'), c('child', 1, 'child2'), j('rejects'), person]
    reference = [{'source': 'child2', 'target': 'child', 'reason': 'Authored repeated referent to the same child'}]
    correct_events = [event('approve', 'parent', 'child'), event('reject', 'child2', 'artifact', event_id='e2')]
    cases.append(blueprint('multi-correct', multi, specs, correct_events, references=reference))
    text = 'The child approves the artifact, and the parent rejects the child.'
    cases.append(blueprint('multi-reattached', text, [p('parent'), h('approves'), c('child'), c('child',1,'child2'), j('rejects'), person],
                           [event('approve', 'child', 'artifact'), event('reject', 'parent', 'child2', event_id='e2')],
                           references=[{'source': 'child2', 'target': 'child', 'reason': 'Authored repeated referent to the same child'}]))
    text = 'The child is approved by the parent, and the artifact is rejected by the child.'
    cases.append(blueprint('multi-passive', text, [p('parent'), h('approved'), c('child'), c('child',1,'child2'), j('rejected'), person], correct_events, references=reference))
    cases.append(blueprint('multi-no-reference', multi, specs, correct_events))
    for label, cue in [('before','before'), ('after','after')]:
        text = 'The parent approves the child ' + cue + ' the child rejects the artifact.'
        cases.append(blueprint('multi-link-' + label, text, specs, correct_events,
                               links=[{'source': 'e1', 'target': 'e2', 'type': label}], references=reference))
    for label in ('a','b'):
        cases.append(blueprint('context-' + label, 'The parent approves the child.', [p('parent'), h('approves'), c('child')],
                               [event('approve', 'parent', 'child')], context=label))
    for kind in ('skill','capacity'):
        cases.append(blueprint('able-' + kind, 'able', [('able', 'able', 0, None, 'a', s['able-' + kind])], []))
    for pair in alias_pairs:
        for language, lemma, sense_id in [('eng', pair['query'], pair['originSenseId']), ('jpn', pair['targetLemma'], pair['targetSenseId'])]:
            name = 'alias-' + ('entirely' if pair['query'] == 'entirely' else 'active') + '-' + language
            cases.append(blueprint(name, lemma, [('alias',lemma,0,None,pair['originPos'],sense_id)],
                                   [event('alias')] if pair['originPos'] == 'v' else [], language=language))
    cases.append(blueprint('competing-approve', 'The parent approves the child.', [p('parent'), h('approves'), c('child')],
                           [event('approve','parent','child')], alternatives=[('approve',s['approve-sanction'])]))
    cases.append(blueprint('unknown', 'quuxword', [('unknown','quuxword',0,None,'unknown',None)], []))
    return cases


def expectations():
    pairs = [
        ('active-passive','approve-active','approve-passive',{'sameAssertedConceptMultiset':True,'sameConceptEvents':True,'sameSourceSenseMultiset':True}),
        ('role-reversal','approve-active','approve-reversed',{'sameAssertedConceptMultiset':True,'sameConceptEvents':False}),
        ('negative-scope','approve-active','approve-negative',{'sameSelectedSourceConceptMultiset':True,'sameConceptEvents':False}),
        ('condition-scope','approve-active','approve-conditional',{'sameConceptEvents':False}),
        ('event-attachment','multi-correct','multi-reattached',{'sameSelectedSourceConceptMultiset':True,'sameFlattenedSelectedRoleBag':True,'sameConceptEvents':False}),
        ('multi-passive','multi-correct','multi-passive',{'sameConceptEvents':True,'sameAssertedReferences':True,'repeatedChildOccurrences':2}),
        ('reference-retention','multi-correct','multi-no-reference',{'sameConceptEvents':True,'sameAssertedReferences':False}),
        ('event-link-direction','multi-link-before','multi-link-after',{'sameConceptEventsIgnoringLinks':True,'sameEventLinks':False}),
        ('context-identity','context-a','context-b',{'sameSourceSenseMultiset':True,'sameAssertedConceptMultiset':True,'sameContextMeetings':False,'contextAddsDictionaryEquivalence':False}),
        ('same-root-senses','able-skill','able-capacity',{'sameRootIds':True,'sameSourceSenseMultiset':False,'sameAssertedConceptMultiset':False}),
        ('alias-entirely','alias-entirely-eng','alias-entirely-jpn',{'assertedEquivalentConceptCandidate':True,'sameSourceSenseMultiset':False}),
        ('alias-active','alias-active-eng','alias-active-jpn',{'assertedEquivalentConceptCandidate':True,'sameConceptEvents':True,'sameSourceSenseMultiset':False}),
    ]
    return [{'id': name, 'left': left, 'right': right, 'expected': expected,
             'authority': 'Independent authored representation contract, not retrieval accuracy gold'} for name,left,right,expected in pairs]


def main():
    if sys.argv[1:] == ['--self-test']:
        self_test()
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--alias-fixtures', type=Path, required=True)
    parser.add_argument('--reuse-accepted-dir', type=Path, help='Reuse exact original binder packets from a preserved partial generation after checking current source pins and authored outline')
    args = parser.parse_args()
    output = args.output.resolve()
    if output.is_relative_to(REPO) or output.exists():
        raise ValueError('Use new private output directory outside Git')
    output.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
    os.chmod(output.parent,0o700)
    os.mkdir(output,0o700)
    manifest, manifest_sha = word_dictionary.read_pinned(REFERENCE, word_dictionary.MANIFEST_PATH, word_dictionary.MAX_MANIFEST, word_dictionary.MANIFEST_SHA)
    members = {row['path']:row for row in manifest['files']}
    def pinned(relative,maximum):
        row=members[relative]
        return word_dictionary.read_pinned(REFERENCE,relative,maximum,row['sha256'],row['bytes'])
    graph, graph_sha = pinned(word_dictionary.GRAPH_PATH,word_dictionary.MAX_GRAPH)
    model, model_sha = pinned(word_dictionary.MODEL_PATH,word_dictionary.MAX_MODEL)
    pins={word_dictionary.MANIFEST_PATH:manifest_sha,word_dictionary.GRAPH_PATH:graph_sha,word_dictionary.MODEL_PATH:model_sha}
    for relative in word_dictionary.SENSESNAP_PATHS + ('semantic_e8/interpretation.py','semantic_e8/inspection.py','semantic_e8/usage.py','semantic_e8/context.py'):
        row=members[relative]
        _,digest=word_dictionary.read_member(REFERENCE,relative,1024*1024,row['sha256'],row['bytes'])
        pins[relative]=digest
    sys.path.insert(0,str(REFERENCE))
    from semantic_e8.interpretation import prepare_request,ground_outline,bind_interpretation,OUTLINE_VERSION,SELECTION_VERSION,CONTEXT_SELECTION_VERSION
    source_senses={sense['id']:sense for sense in graph['senses']}
    with args.alias_fixtures.open('rb') as alias_input:
        alias_raw=alias_input.read(1024*1024+1)
    if len(alias_raw)>1024*1024:raise ValueError('Alias fixture input exceeds budget')
    aliases=[row for row in json.loads(alias_raw)['fixtures'] if row['targetLanguage']=='jpn']
    if {row['query'] for row in aliases}!={'entirely','be active'}:raise ValueError('Expected the two original truncated Japanese aliases')
    for row in aliases:
        row['originPos']=source_senses[row['originSenseId']]['pos']
        origin_links={r['concept_id'] for r in graph['alignments'] if r['sense_id']==row['originSenseId'] and r['kind']=='equivalent' and r.get('asserted') is True}
        target_links={r['concept_id'] for r in graph['alignments'] if r['sense_id']==row['targetSenseId'] and r['kind']=='equivalent' and r.get('asserted') is True}
        if row['conceptId'] not in origin_links & target_links:raise ValueError('Alias lacks asserted endpoint equivalence')
    cases=plan(aliases)
    expected=expectations()
    private_json(output/'authored-blueprints.json',{'schema':SCHEMA,'cases':cases,'expectations':expected,'referencePins':pins,
                  'basis':'Fresh authored outlines/source choices frozen before original native binder and before retrieval implementation outputs'})
    profiles=[]
    unavailable=[]
    for index,case in enumerate(cases,1):
        started=time.monotonic()
        context=context_for(case['contextLabel'])
        reused = args.reuse_accepted_dir / (case['id'] + '.json') if args.reuse_accepted_dir else None
        if reused and reused.exists():
            with reused.open('rb') as prior:
                raw = prior.read(16 * 1024 * 1024 + 1)
            if len(raw)>16*1024*1024:raise ValueError('Reuse packet exceeds budget')
            result=json.loads(raw)
            binding=result['interpretation']['binding']
            expected_outline={'schema':OUTLINE_VERSION,'atoms':case['atoms'],'readings':case['readings']}
            if result['sentence']!=case['sentence'] or result['language']!=case['language'] or binding['outline']!=expected_outline or binding['request']['context']!=context or result['fixtureProvenance']['sourcePins']!=pins:
                raise ValueError('Previously accepted binder packet differs from current authored/source contract')
            for native, choices in zip(binding['readings'],case['choices']):
                if {atom['atom']['id']:[candidate['sense']['id'] for candidate in atom['selected_source_bindings']] for atom in native['bound_usage']['atoms']}!=choices:
                    raise ValueError('Previously accepted source selection differs')
            receipt=private_json(output/(case['id']+'.json'),result)
            profiles.append({'id':case['id'],'file':case['id']+'.json',**receipt,'readingCount':len(case['readings']),
                             'atomCount':len(case['atoms']),'eventCounts':[len(reading['events']) for reading in case['readings']],
                             'reusedOriginalBinderPacketSha256':hashlib.sha256(raw).hexdigest()})
            print(json.dumps({'case':case['id'],'index':index,'total':len(cases),'bytes':receipt['bytes'],'reusedExactOriginal':True}),flush=True)
            continue
        request=prepare_request(graph,model,case['language'],case['sentence'],context)
        outline={'schema':OUTLINE_VERSION,'atoms':case['atoms'],'readings':case['readings']}
        try:
            packet=ground_outline(graph,model,request,outline)
        except ValueError as error:
            if not str(error).startswith(('Provider source view exceeds', 'Full source receipt exceeds', 'More than 512 source candidates')):
                raise
            failure={'id':case['id'],'status':'source-binding-unavailable','reason':str(error),
                     'selectedSourceIds':case['choices'],'sourceAlternativesDropped':False,
                     'interpretationInvented':False,'nativeBinderChanged':False}
            private_json(output/(case['id']+'.unavailable.json'),failure)
            unavailable.append(failure)
            print(json.dumps({'case':case['id'],'index':index,'total':len(cases),'unavailable':str(error)}),flush=True)
            continue
        selections=[]
        for reading,choice in zip(case['readings'],case['choices']):
            bindings=[]
            for atom in case['atoms']:
                candidate_ids={candidate['sense']['id'] for row in packet['atoms'] if row['atom']['id']==atom['id'] for candidate in row['candidates']}
                if not set(choice[atom['id']])<=candidate_ids:raise ValueError('Authored source selection missing from exact packet')
                item={'atom_id':atom['id'],'sense_ids':choice[atom['id']],'reason':'Independent authored source-ID choice; no automatic inference claim'}
                if case['contextLabel'] is not None and atom['id']=='child':item['context_meeting_ids']=['meet:personal:heldout-child-'+case['contextLabel']]
                bindings.append(item)
            selections.append({'id':reading['id'],'sense_bindings':bindings,'uncertainty':['Authored choices are not linguistic gold.']})
        selection={'schema':CONTEXT_SELECTION_VERSION if case['contextLabel'] else SELECTION_VERSION,'readings':selections,
                   'reason':'Independent authored representation contract before native retrieval scores'}
        binding=bind_interpretation(graph,model,request,outline,selection,'heldout-case-author')
        result={'schema':'bomb-code/joe-result/v1','requestId':f'10000000-0000-4000-8000-{index:012}',
                'threadId':'10000000-0000-4000-8000-000000000100','sentence':case['sentence'],'language':case['language'],
                'status':'grounded-model-proposal','provider':'authored-offline-fixture','model':'heldout-authored-not-live/v1',
                'interpretation':{'status':'grounded-model-proposal','binding':binding},
                'reference':{'root':str(REFERENCE),'manifestSha256':manifest_sha},'referenceRequested':{'manifestSha256':manifest_sha},
                'authority':{'toolsDispatched':False,'approvalsGranted':False,'memoryCommitted':False},'clarifications':[],
                'fixtureProvenance':{'kind':'fresh authored controls bound through original pinned native functions',
                                     'providerCalls':0,'sourcePins':pins,'interpretationGold':False}}
        path=output/(case['id']+'.json')
        receipt=private_json(path,result)
        profiles.append({'id':case['id'],'file':path.name,**receipt,'readingCount':len(case['readings']),
                         'atomCount':len(case['atoms']),'eventCounts':[len(reading['events']) for reading in case['readings']]})
        print(json.dumps({'case':case['id'],'index':index,'total':len(cases),'bytes':receipt['bytes'],'seconds':round(time.monotonic()-started,3)}),flush=True)
    after={relative:sha_file(REFERENCE/relative) for relative in pins}
    if after!=pins:raise ValueError('Native reference changed during authored binding')
    manifest_output={'schema':SCHEMA,'profiles':profiles,'unavailableProfiles':unavailable,'expectations':expected,'referencePins':pins,
                     'sourceSnapshotCounts':{'senses':len(graph['senses']),'concepts':len(graph['concepts']),'placements':len(model['placements']),
                                             'alignments':len(graph['alignments']),'relations':len(graph.get('relations',[]))},
                     'blueprintsSha256':sha_file(output/'authored-blueprints.json'),'aliasInputSha256':hashlib.sha256(alias_raw).hexdigest(),
                     'scriptSha256':sha_file(Path(__file__)),'integrity':{'sourceUnchanged':True,'noNetwork':True,'noArchiveOrIndexOpened':True},
                     'limitations':['Held-out relative to this prototype output; source lexical fixtures and earlier aliases are known seeds.',
                                    'Authored frames/source choices test representation computation, not automatic interpretation or human intent.',
                                    'Unselected function words remain visible and do not create false dictionary identities.',
                                    'No archive retrieval improvement can be inferred without source-bound archive annotations.']}
    receipt=private_json(output/'manifest.json',manifest_output)
    print(json.dumps({'manifest':str(output/'manifest.json'),**receipt,'profiles':len(profiles),'unavailable':len(unavailable),'expectations':len(expected)}),flush=True)


def self_test():
    import collections
    import tempfile
    import unittest

    def role_bag(case):
        values=[]
        for event in case['readings'][0]['events']:
            values.append(('predicate',tuple(case['choices'][0][event['predicate']])))
            values.extend((role['role'],tuple(case['choices'][0][role['atom_id']])) for role in event['roles'])
        return collections.Counter(values)

    def event_graph(case):
        return sorted((tuple(case['choices'][0][event['predicate']]),
                       tuple(sorted((role['role'],tuple(case['choices'][0][role['atom_id']])) for role in event['roles'])),
                       event['polarity'],event['modality']) for event in case['readings'][0]['events'])

    class Contracts(unittest.TestCase):
        def test_equal_bags_but_different_predicate_attachments(self):
            cases={case['id']:case for case in plan([])}
            left,right=cases['multi-correct'],cases['multi-reattached']
            self.assertEqual(collections.Counter(re.findall(r'\w+',left['sentence'].casefold())),
                             collections.Counter(re.findall(r'\w+',right['sentence'].casefold())))
            self.assertEqual(role_bag(left),role_bag(right))
            self.assertNotEqual(event_graph(left),event_graph(right))

        def test_active_passive_keep_event_content(self):
            cases={case['id']:case for case in plan([])}
            self.assertEqual(event_graph(cases['approve-active']),event_graph(cases['approve-passive']))
            self.assertNotEqual(event_graph(cases['approve-active']),event_graph(cases['approve-reversed']))

        def test_unicode_repeated_spans_and_function_occurrences(self):
            sentence='é🧭 child child.'
            atoms,choices=atom_specs(sentence,[('first','child',0,None,'n','source'),('second','child',1,None,'n','source')])
            selected={atom['id']:atom for atom in atoms}
            self.assertEqual(selected['first']['span'],[3,8])
            self.assertEqual(selected['second']['span'],[9,14])
            self.assertEqual(len(atoms),3)
            for atom in atoms:self.assertEqual(sentence[slice(*atom['span'])],atom['surface'])
            self.assertEqual(choices['first'],choices['second'])

        def test_private_exclusive_file_and_budget(self):
            with tempfile.TemporaryDirectory() as folder:
                path=Path(folder)/'case.json'
                private_json(path,{'case':'synthetic'})
                self.assertEqual(path.stat().st_mode & 0o777,0o600)
                with self.assertRaises(FileExistsError):private_json(path,{})
                with self.assertRaises(ValueError):private_json(Path(folder)/'large.json',{'text':'x'*(16*1024*1024)})

    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Contracts))
    if not result.wasSuccessful():raise SystemExit(1)


if __name__=='__main__':main()

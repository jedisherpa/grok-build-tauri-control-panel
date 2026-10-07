"""Prepare bounded, unsent clarification drafts from complete native comparisons.

Consumes an already verified probe result. Questions point back to original
readings/events/occurrences; this module selects no sense and invokes no model.
"""
import json
from collections import Counter


def prepare_questions(result, maximum=8):
    if type(maximum) is not int or not 1 <= maximum <= 16:
        raise ValueError('Question budget must be between 1 and 16')
    if result.get('status') != 'ready':
        return {'schema': 'bomb-code/structured-questions/v1', 'questions': [], 'status': 'unavailable'}
    query = result['queryProfile']
    candidates = {p['candidateId']: p['profile'] for p in result['candidateProfiles']}
    readings = {r['readingId']: r for r in query['readings']}
    questions = []
    seen = set()
    def add(kind, text, evidence):
        key = (kind, json.dumps(evidence, sort_keys=True))
        if key not in seen and len(questions) < maximum:
            seen.add(key)
            questions.append({'kind':kind, 'text':text, 'evidence':evidence, 'sendStatus':'unsent'})
    for row in result['retrieval']['candidateComparisons']:
        other = {r['readingId']: r for r in candidates[row['candidateId']]['readings']}
        for pair in row['comparison']['readingPairs']:
            q = readings[pair['queryReadingId']]
            c = other[pair['candidateReadingId']]
            qa = {a['atomId']:a for a in q['occurrences']}
            ca = {a['atomId']:a for a in c['occurrences']}
            evidence = {'queryReadingId':q['readingId'], 'candidateId':row['candidateId'], 'candidateReadingId':c['readingId']}
            events = pair['eventLocal']['eventPairs']
            query_counts = Counter(e['queryEventId'] for e in events if e['predicateEqual'])
            candidate_counts = Counter(e['candidateEventId'] for e in events if e['predicateEqual'])
            for event in events:
                if not event['predicateEqual']:
                    continue
                # Multiple same-predicate events have ambiguous correspondence.
                if query_counts[event['queryEventId']] != 1:
                    continue
                if candidate_counts[event['candidateEventId']] != 1:
                    continue
                qe = event['query']['sourceEvent']
                ce = event['candidate']['sourceEvent']
                word = qa[qe['predicate']]['surface']
                ev = dict(evidence, queryEventId=event['queryEventId'], candidateEventId=event['candidateEventId'])
                if not event['rolesEqual']:
                    def roles(frame, atoms):
                        return ', '.join(str(r['role']) + '=' + atoms[r['atom_id']]['surface'] for r in frame['roles'])
                    add('event-participants', 'For ' + repr(word) + ', who performs the action and who is affected? Current roles: ' + roles(qe,qa) + '; comparison roles: ' + roles(ce,ca) + '.', ev)
                if not event['polarityEqual']:
                    add('polarity', 'For ' + repr(word) + ', do you mean that the action happens or that it does not happen? The two retained event frames differ in polarity.', ev)
                if not event['modalityEqual']:
                    add('modality', 'For ' + repr(word) + ', is the action actual, possible, required, or conditional? The two retained event frames differ in modality.', ev)
            graph = pair['eventLocal']
            if graph['queryCanonicalGraph']['links'] != graph['candidateCanonicalGraph']['links']:
                add('event-links', 'How are these events related in time or condition? The retained event links differ.', evidence)
            if graph['queryCanonicalGraph']['references'] != graph['candidateCanonicalGraph']['references']:
                add('references', 'Which earlier occurrence does this reference point to? The retained reference links differ.', evidence)
            context = pair['senseSnap']['contextPinDistance']
            if context and context['totalVariation'] > 0:
                add('context', 'Do these uses belong to the same personal or topic context? Their explicitly bound context pins differ.', evidence)
    if len(query['readings']) > 1:
        atoms = {}
        for reading in query['readings']:
            for atom in reading['occurrences']:
                selected = tuple(s['sense']['id'] for s in atom['selectedSourceBindings'])
                if selected:
                    atoms.setdefault(atom['atomId'], []).append((reading['readingId'], atom, selected))
        for atom_id, choices in atoms.items():
            if len({x[2] for x in choices}) > 1:
                add('source-sense', 'Which dictionary sense of ' + repr(choices[0][1]['surface']) + ' fits what you mean here? Review the definitions attached to these retained readings.', {'queryAtomId':atom_id, 'readingSenseChoices':[{'readingId':rid, 'sourceSenseIds':list(senses)} for rid,_,senses in choices]})
    return {'schema':'bomb-code/structured-questions/v1', 'status':'ready', 'questions':questions, 'budget':maximum, 'providerCalls':0, 'executionActions':0}

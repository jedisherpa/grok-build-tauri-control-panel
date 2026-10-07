#!/usr/bin/env python3
"""Bounded bilingual dictionary controls, separate from archive relevance.

Fixtures are selected and written before calling the frozen Interpreter. Only
source-asserted equivalent links qualify. Neither archive nor index is opened.
"""
import argparse
import collections
import json
import os
from pathlib import Path
import sqlite3
import sys

import recall_experiment as experiment
import word_dictionary

QUOTAS = {'jpn': 2, 'ind': 2, 'zsm': 1, 'cmn': 1}
SCHEMA = 'bomb-code/bilingual-expansion-controls/v1'


def equivalent_concepts(dictionary, sense_id):
    return {row['concept_id'] for row in dictionary.alignments[sense_id]
            if row.get('concept_id') and row.get('kind') == 'equivalent' and row.get('asserted') is True}


def select_fixtures(dictionary):
    by_lemma = collections.defaultdict(list)
    for sense in dictionary.senses.values():
        by_lemma[word_dictionary.normalize(sense['lemma'])].append(sense)
    selected, used = [], set()
    counts = collections.Counter()
    for concept_id in sorted(dictionary.concepts):
        linked = [dictionary.senses[sid] for sid in sorted(dictionary.by_concept[concept_id])
                  if concept_id in equivalent_concepts(dictionary, sid)]
        origins = [sense for sense in linked if sense['language'] == 'eng'
                   and isinstance(sense.get('definition'), str) and sense['definition'].strip()
                   and len(by_lemma[word_dictionary.normalize(sense['lemma'])]) == 1
                   and 1 <= len(experiment.tokens(sense['lemma'])) <= 3
                   and 2 <= len(sense['lemma']) <= 64
                   and not experiment.content(sense['lemma']).isdisjoint(experiment.tokens(sense['lemma']))]
        if not origins:
            continue
        origin = origins[0]
        for target in linked:
            language = target['language']
            if language not in QUOTAS or counts[language] >= QUOTAS[language] or target['id'] in used:
                continue
            if not 1 <= len(experiment.tokens(target['lemma'])) <= 3 or not 1 <= len(target['lemma']) <= 64:
                continue
            if set(experiment.tokens(origin['lemma'])) & set(experiment.tokens(target['lemma'])):
                continue
            controls = [sense for _, sense in sorted(dictionary.senses.items())
                        if sense['language'] == language and sense['id'] != target['id']
                        and 1 <= len(sense['lemma']) <= 64 and 1 <= len(experiment.tokens(sense['lemma'])) <= 3
                        and equivalent_concepts(dictionary, sense['id'])
                        and not equivalent_concepts(dictionary, sense['id']) & equivalent_concepts(dictionary, origin['id'])
                        and not set(experiment.tokens(sense['lemma'])) &
                        (set(experiment.tokens(origin['lemma'])) | set(experiment.tokens(target['lemma'])))]
            if not controls:
                continue
            control = controls[0]
            counts[language] += 1
            used.add(target['id'])
            selected.append({'id': f'bilingual-{language}-{counts[language]}', 'conceptId': concept_id,
                             'query': origin['lemma'], 'originSenseId': origin['id'], 'originLanguage': 'eng',
                             'originDefinition': origin['definition'], 'targetSenseId': target['id'],
                             'targetLanguage': language, 'targetLemma': target['lemma'],
                             'targetLink': dictionary.counterpart(target['id'], [concept_id], origin['id']),
                             'controlSenseId': control['id'], 'controlLemma': control['lemma'],
                             'controlConceptIds': sorted(equivalent_concepts(dictionary, control['id']))})
            # The supplemental diversity control freezes one pair per concept.
            # The initial six-form, single-concept run remains a separate receipt.
            break
        if all(counts[language] == quota for language, quota in QUOTAS.items()):
            break
    return selected


def tiny_lexical(query, target, control):
    # Same existing FTS5/BM25 query implementation; ephemeral local documents.
    database = sqlite3.connect(':memory:')
    database.execute('CREATE TABLE chunks(chunk_id TEXT PRIMARY KEY)')
    database.execute("CREATE VIRTUAL TABLE search USING fts5(chunk_id UNINDEXED,text,title,tokenize='unicode61')")
    for chunk_id, text in [('target', target), ('control', control)]:
        database.execute('INSERT INTO chunks VALUES(?)', (chunk_id,))
        database.execute('INSERT INTO search VALUES(?,?,?)', (chunk_id, text, ''))
    try:
        return experiment.recall.lexical(database, query, limit=2)
    finally:
        database.close()


def evaluate_fixture(fixture, interpreter):
    query = fixture['query']
    info = interpreter.interpret(query)
    expanded_query = query + ' ' + ' '.join(info['expansionTerms'])
    baseline = tiny_lexical(query, fixture['targetLemma'], fixture['controlLemma'])
    expanded = tiny_lexical(expanded_query, fixture['targetLemma'], fixture['controlLemma'])
    return {'id': fixture['id'], 'baseline': baseline, 'expanded': expanded,
            'exactQueryTargetTokenOverlap': sorted(set(experiment.tokens(query)) & set(experiment.tokens(fixture['targetLemma']))),
            'expandedTargetTokenOverlap': sorted(set(experiment.tokens(expanded_query)) & set(experiment.tokens(fixture['targetLemma']))),
            'targetInFrozenExpansion': word_dictionary.normalize(fixture['targetLemma']) in
                {word_dictionary.normalize(lemma) for lemma in info['expansionTerms']},
            'originSelected': fixture['originSenseId'] in {row['senseId'] for row in info['selected']},
            'targetHitBaseline': 'target' in baseline, 'targetHitExpanded': 'target' in expanded,
            'controlHitBaseline': 'control' in baseline, 'controlHitExpanded': 'control' in expanded,
            'interpretation': info,
            'qualification': 'Source-asserted bilingual expansion mechanics; not independent translation accuracy or archive relevance'}


def run(output):
    output = Path(output).resolve()
    repo = Path(__file__).resolve().parents[1]
    if output.is_relative_to(repo) or output.exists():
        raise ValueError('Use a new private output directory outside Git')
    os.mkdir(output, 0o700)
    config_before = experiment.recall.digest(experiment.CONFIG)
    dictionary = word_dictionary.Dictionary.load()
    reference_before = dict(dictionary.reference)
    fixtures = select_fixtures(dictionary)
    selection = {'schema': SCHEMA, 'quotas': QUOTAS, 'selected': len(fixtures),
                 'fixtureSelection': 'Sorted concept and sense IDs; at most one pair per concept; first unique-defined English origin; first eligible target and disjoint-concept/token control per language quota; no Interpreter or retrieval output inspected during selection',
                 'reference': reference_before, 'configSha256': config_before, 'fixtures': fixtures}
    experiment.private_write(output / 'fixtures.json', selection)
    # Fixture IDs, targets and controls are frozen before any ranking.
    interpreter = experiment.Interpreter(dictionary)
    results = [evaluate_fixture(fixture, interpreter) for fixture in fixtures]
    config_after = experiment.recall.digest(experiment.CONFIG)
    del interpreter, dictionary
    reference_after = word_dictionary.Dictionary.load().reference
    if reference_before != reference_after or config_before != config_after:
        raise ValueError('Reference/config changed; controls remain unqualified')
    summary = {'schema': SCHEMA, 'fixtureCount': len(fixtures), 'targetLanguages': dict(collections.Counter(row['targetLanguage'] for row in fixtures)),
               'distinctConcepts': len({row['conceptId'] for row in fixtures}),
               'distinctQueries': len({row['query'] for row in fixtures}),
               'baselineTargetHits': sum(row['targetHitBaseline'] for row in results),
               'expandedTargetHits': sum(row['targetHitExpanded'] for row in results),
               'baselineControlHits': sum(row['controlHitBaseline'] for row in results),
               'expandedControlHits': sum(row['controlHitExpanded'] for row in results),
               'originsSelected': sum(row['originSelected'] for row in results),
               'targetsInExpansion': sum(row['targetInFrozenExpansion'] for row in results),
               'fixturesSha256': experiment.sha_file(output / 'fixtures.json'),
               'configSha256': config_before, 'referenceIntegrity': True, 'configIntegrity': True,
               'noNetwork': True, 'noArchiveOrIndexOpened': True,
               'qualification': 'Deterministic source-asserted bilingual equivalence fixtures; local expansion/FTS mechanics only. No independent translation gold, multilingual archive relevance, or meaning accuracy claim.',
               'results': results}
    experiment.private_write(output / 'summary.json', summary)
    return {key: value for key, value in summary.items() if key != 'results'}


def self_test():
    from types import SimpleNamespace
    import unittest
    from unittest import mock

    class Boundaries(unittest.TestCase):
        def test_tiny_unicode_fts_and_unrelated_control(self):
            self.assertEqual(tiny_lexical('entity', '物', '変形'), [])
            self.assertEqual(tiny_lexical('entity 物', '物', '変形'), ['target'])

        def test_selection_does_not_rank_or_promote_candidate(self):
            dictionary = SimpleNamespace(
                senses={'o': {'id': 'o', 'language': 'eng', 'lemma': 'entity', 'definition': 'anything that exists'},
                        't': {'id': 't', 'language': 'jpn', 'lemma': '物'},
                        'a': {'id': 'a', 'language': 'jpn', 'lemma': 'もの'},
                        'c': {'id': 'c', 'language': 'jpn', 'lemma': '変形'}},
                concepts={'origin': {}, 'control': {}},
                by_concept={'origin': {'o', 't', 'a'}, 'control': {'c'}},
                alignments={'o': [{'concept_id': 'origin', 'kind': 'equivalent', 'asserted': True}],
                            't': [{'concept_id': 'origin', 'kind': 'equivalent', 'asserted': True}],
                            'a': [{'concept_id': 'origin', 'kind': 'candidate', 'asserted': False}],
                            'c': [{'concept_id': 'control', 'kind': 'equivalent', 'asserted': True}]},
                counterpart=lambda *_: {'linkStatus': 'source-equivalent'})
            with mock.patch.object(experiment.Interpreter, 'interpret', side_effect=AssertionError('ranking during selection')):
                selected = select_fixtures(dictionary)
                self.assertEqual(len(selected), 1)
                self.assertEqual(selected[0]['targetSenseId'], 't')
                self.assertEqual(selected[0]['controlSenseId'], 'c')
                original = selected
                dictionary.senses = dict(reversed(list(dictionary.senses.items())))
                self.assertEqual(select_fixtures(dictionary), original)

        def test_controls_have_distinct_source_concepts(self):
            # English and target text alone never establish equivalence.
            dictionary = SimpleNamespace(alignments={'candidate': [{'concept_id': 'c', 'kind': 'candidate', 'asserted': False}],
                                                    'asserted': [{'concept_id': 'c', 'kind': 'equivalent', 'asserted': True}]})
            self.assertEqual(equivalent_concepts(dictionary, 'candidate'), set())
            self.assertEqual(equivalent_concepts(dictionary, 'asserted'), {'c'})

    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Boundaries))
    if not result.wasSuccessful():
        raise SystemExit(1)


if __name__ == '__main__':
    if sys.argv[1:] == ['--self-test']:
        self_test()
        raise SystemExit(0)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True)
    options = parser.parse_args()
    print(json.dumps(run(options.output), ensure_ascii=False, allow_nan=False))

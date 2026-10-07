from pathlib import Path
import json
import sqlite3
import tempfile
import unittest
from unittest import mock

import source_concept_candidates as bridge


class DictionaryFixture:
    def __init__(self):
        self.concepts = {'river': {'id': 'river'}, 'finance': {'id': 'finance'}}
        self.senses = {
            'bank-river': {'id': 'bank-river', 'lemma': 'bank', 'language': 'eng', 'definition': 'river edge'},
            'bank-finance': {'id': 'bank-finance', 'lemma': 'bank', 'language': 'eng', 'definition': 'financial institution'},
            'shore': {'id': 'shore', 'lemma': 'shore', 'language': 'eng', 'definition': 'river edge'},
            'japanese': {'id': 'japanese', 'lemma': '岸辺', 'language': 'jpn', 'definition': None},
            'candidate-only': {'id': 'candidate-only', 'lemma': 'coast', 'language': 'eng', 'definition': 'sea edge'},
        }
        self.alignments = {sid: [{'id': 'alignment-'+sid, 'sense_id': sid,
                                 'concept_id': 'finance' if sid == 'bank-finance' else 'river',
                                 'kind': 'candidate' if sid == 'candidate-only' else 'equivalent',
                                 'asserted': sid != 'candidate-only'}] for sid in self.senses}
        self.by_concept = {'river': {'bank-river', 'shore', 'japanese', 'candidate-only'},
                           'finance': {'bank-finance'}}
    def source(self, _source_id):
        return {'id': 'fixture'}


def record(text, cid='b'):
    return {'chunkId': cid, 'text': text, 'start': 100, 'end': 100+len(text)}


class SourceConceptTests(unittest.TestCase):
    def setUp(self):
        self.dictionary = DictionaryFixture()

    def test_complete_asserted_aliases_and_no_candidate_link_promotion(self):
        aliases = bridge.SourceAliases(self.dictionary, ['river'])
        self.assertEqual(set(aliases.forms), {'bank', 'shore', '岸辺'})
        self.assertNotIn('coast', aliases.forms)

    def test_all_exact_form_alternatives_are_retained_unselected(self):
        aliases = bridge.SourceAliases(self.dictionary, ['river'])
        matches = aliases.occurrences('BANK')
        self.assertEqual(matches[0]['alternativeSenseIds'], ['bank-finance', 'bank-river'])
        self.assertEqual(matches[0]['usageSelection'], 'UNSELECTED')
        sources = aliases.source_records({'bank'})
        self.assertEqual(sources['bank-finance']['assertedQueryAlignments'], [])
        self.assertEqual(sources['bank-river']['sense'], self.dictionary.senses['bank-river'])
        self.assertEqual(sources['bank-river']['alignments'], self.dictionary.alignments['bank-river'])

    def test_unicode_spans_cjk_and_repeated_occurrences(self):
        matches = bridge.SourceAliases(self.dictionary, ['river']).occurrences('😀岸辺へ BANK bank')
        self.assertEqual([m['span'] for m in matches], [[1, 3], [5, 9], [10, 14]])
        self.assertIn('not segmentation', matches[0]['matchBasis'])

    def test_latin_word_boundaries_prevent_substring_hits(self):
        aliases = bridge.SourceAliases(self.dictionary, ['river'])
        self.assertEqual(aliases.occurrences('banking embankment'), [])
        self.assertEqual(len(aliases.occurrences('(bank) bank.')), 2)

    def test_combining_casefold_cluster_and_whitespace_offsets(self):
        d = self.dictionary
        d.senses['extra'] = {'id': 'extra', 'lemma': 'é ss river_bank', 'definition': 'fixture'}
        d.alignments['extra'] = [{'id': 'extra-link', 'concept_id': 'river', 'kind': 'equivalent', 'asserted': True}]
        d.by_concept['river'].add('extra')
        text = '🙂 e\u0301 ß river_  bank!'
        matches = bridge.SourceAliases(d, ['river']).occurrences(text)
        full = next(m for m in matches if m['normalizedForm'] == 'é ss river bank')
        self.assertEqual(text[slice(*full['span'])], 'e\u0301 ß river_  bank')
        self.assertEqual(full['span'], [2, 19])

    def test_casefold_expansion_cannot_create_half_cluster_alias(self):
        d = self.dictionary
        d.senses['s'] = {'id': 's', 'lemma': 's', 'definition': 'fixture'}
        d.alignments['s'] = [{'id': 's-link', 'concept_id': 'river', 'kind': 'equivalent', 'asserted': True}]
        d.by_concept['river'].add('s')
        self.assertEqual(bridge.SourceAliases(d, ['river']).occurrences('ß'), [])

    def test_distinct_concepts_not_alias_or_occurrence_counts_order_candidates(self):
        aliases = bridge.SourceAliases(self.dictionary, ['river', 'finance'])
        result = bridge.match_records(aliases, [record('shore shore shore', 'a'), record('bank', 'c'), record('shore', 'b')])
        self.assertEqual([r['chunk']['chunkId'] for r in result['hits']], ['c', 'a', 'b'])
        self.assertEqual(result['candidateCount'], 3)

    def test_notes_scope_filters_after_complete_source_form_matching(self):
        records = [dict(record('bank', 'a'), kind='history', source='codex', scope=''),
                   dict(record('bank', 'b'), kind='note', source='notes', scope='project-a'),
                   dict(record('bank bank', 'c'), kind='note', source='notes', scope='project-b')]
        aliases = bridge.SourceAliases(self.dictionary, ['river'])
        scoped = bridge.match_records(aliases, records, 1, scope='project-a')
        self.assertEqual((scoped['scannedChunks'], scoped['unfilteredCandidateCount'], scoped['candidateCount']), (3, 3, 1))
        self.assertEqual(scoped['hits'][0]['chunk']['chunkId'], 'b')
        history = bridge.match_records(aliases, records, source='history')
        self.assertEqual([h['chunk']['chunkId'] for h in history['hits']], ['a'])
        provider = bridge.match_records(aliases, records, source='codex')
        self.assertEqual([h['chunk']['chunkId'] for h in provider['hits']], ['a'])
        self.assertEqual(bridge.match_records(aliases, records, source='history', scope='project-a')['hits'], [])

    def test_host_options_reject_paths_notes_unknown_and_invalid_limits(self):
        for value in ({'conceptIds': ['river'], 'panel': '/tmp'}, {'conceptIds': ['river'], 'notes': []},
                      {'conceptIds': ['river'], 'limit': 13}, {'conceptIds': [{}]},
                      {'conceptIds': ['river', 'river']}, {'conceptIds': ['river'], 'source': 'unknown'}):
            with self.assertRaises(bridge.CandidateError):
                bridge.request_options(value)
        self.assertEqual(bridge.request_options({'conceptIds': ['river'], 'source': 'notes', 'scope': 'project-a', 'limit': 12}),
                         (['river'], 'notes', 'project-a', 12))

    def test_return_limit_reports_full_census_without_truncating_hit_alternatives(self):
        result = bridge.match_records(bridge.SourceAliases(self.dictionary, ['river']), [record('bank', 'a'), record('shore', 'b')], 1)
        self.assertEqual((result['candidateCount'], result['returnedCount'], result['notReturnedCount']), (2, 1, 1))
        self.assertEqual(len(result['hits'][0]['occurrences'][0]['alternativeSenseIds']), 2)

    def test_invalid_concepts_and_bounded_limit_rejected(self):
        for ids in ([], ['unknown'], ['river', 'river'], ['river']*9, 'river'):
            with self.assertRaises(bridge.CandidateError):
                bridge.SourceAliases(self.dictionary, ids)
        with self.assertRaises(bridge.CandidateError):
            bridge.match_records(bridge.SourceAliases(self.dictionary, ['river']), [], True)

    def test_budget_failures_are_explicit(self):
        with mock.patch.object(bridge, 'MAX_ALIASES', 1):
            with self.assertRaisesRegex(bridge.CandidateError, 'no aliases were dropped'):
                bridge.SourceAliases(self.dictionary, ['river'])
        aliases = bridge.SourceAliases(self.dictionary, ['river'])
        with mock.patch.object(bridge, 'MAX_OCCURRENCES_PER_CHUNK', 1):
            with self.assertRaisesRegex(bridge.CandidateError, 'no hits were dropped'):
                aliases.occurrences('bank bank')
        with mock.patch.object(bridge, 'MAX_CHUNKS', 1):
            with self.assertRaisesRegex(bridge.CandidateError, 'no partial scan'):
                bridge.match_records(aliases, [record('bank'), record('bank')])

    def test_source_only_stored_model_cannot_embed(self):
        model = bridge.StoredModel('stored')
        self.assertEqual(model.identity(), 'stored')
        with self.assertRaises(bridge.CandidateError):
            model.embed(['private source'])

    def test_output_encoding_counts_utf8_and_never_truncates_records(self):
        value = {'definitions': ['岸辺'] * 20}
        self.assertEqual(json.loads(bridge.encode_result(value)), value)
        with mock.patch.object(bridge, 'MAX_OUTPUT_BYTES', 32):
            with self.assertRaisesRegex(bridge.CandidateError, 'no records were dropped'):
                bridge.encode_result(value)

    def test_existing_database_and_serialized_index_budgets(self):
        with sqlite3.connect(':memory:') as db:
            db.execute('CREATE TABLE chunks(chunk_id TEXT, data TEXT)')
            db.executemany('INSERT INTO chunks VALUES (?,?)',
                           [('a', json.dumps(record('bank', 'a'))),
                            ('b', json.dumps(record('shore', 'b')))])
            self.assertEqual(len(list(bridge.index_records(db))), 2)
            with mock.patch.object(bridge.recall, 'MAX_DATABASE_BYTES', 1):
                with self.assertRaises(bridge.recall.RecallError):
                    list(bridge.index_records(db))
            one = len(json.dumps(record('bank', 'a')).encode('utf-8'))
            with mock.patch.object(bridge.recall, 'MAX_INDEX_BYTES', one):
                with self.assertRaisesRegex(bridge.CandidateError, 'no partial scan'):
                    bridge.match_records(bridge.SourceAliases(self.dictionary, ['river']),
                                         bridge.index_records(db))

    def test_private_output_no_overwrite(self):
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder)/'private'
            bridge.write_private(output, {'schema': bridge.SCHEMA})
            self.assertEqual(output.stat().st_mode&0o777, 0o700)
            self.assertEqual((output/'candidates.json').stat().st_mode&0o777, 0o600)
            with self.assertRaises(bridge.CandidateError):
                bridge.write_private(output, {})


@unittest.skipUnless(bridge.word_dictionary.REFERENCE_ROOT.is_dir(), 'installed frozen dictionary unavailable')
class NativeSourceConceptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # The deterministic diversity rule was frozen in the prior experiment.
        from recall_experiment_bilingual import select_fixtures
        from recall_experiment import Interpreter
        cls.dictionary = bridge.word_dictionary.Dictionary.load()
        cls.fixtures = select_fixtures(cls.dictionary)
        cls.previous = Interpreter(cls.dictionary)

    def test_all_six_diverse_native_pairs_including_old_japanese_budget_misses(self):
        self.assertEqual(len(self.fixtures), 6)
        self.assertEqual(len({f['conceptId'] for f in self.fixtures}), 6)
        old_japanese_misses = 0
        for fixture in self.fixtures:
            aliases = bridge.SourceAliases(self.dictionary, [fixture['conceptId']])
            result = bridge.match_records(aliases, [record(fixture['targetLemma'], fixture['id'])])
            self.assertEqual(result['candidateCount'], 1, fixture['id'])
            self.assertTrue(any(fixture['targetSenseId'] in item['alternativeSenseIds']
                                for item in result['hits'][0]['occurrences']), fixture['id'])
            self.assertTrue(all(item['usageSelection'] == 'UNSELECTED'
                                for item in result['hits'][0]['occurrences']))
            previous = self.previous.interpret(fixture['query'])['expansionTerms']
            if fixture['targetLanguage'] == 'jpn' and fixture['targetLemma'] not in previous:
                old_japanese_misses += 1
        self.assertEqual(old_japanese_misses, 2)

    def test_native_source_record_provenance_retained_exactly(self):
        fixture = self.fixtures[0]
        aliases = bridge.SourceAliases(self.dictionary, [fixture['conceptId']])
        form = bridge.word_dictionary.normalize(fixture['targetLemma'])
        records = aliases.source_records({form})
        self.assertEqual(records[fixture['targetSenseId']]['sense'],
                         self.dictionary.senses[fixture['targetSenseId']])
        self.assertEqual(records[fixture['targetSenseId']]['alignments'],
                         self.dictionary.alignments[fixture['targetSenseId']])


if __name__ == '__main__':
    unittest.main()

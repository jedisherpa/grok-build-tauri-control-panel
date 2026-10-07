import copy
import json
import sqlite3
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import recall_experiment as experiment
import memory_recall as recall


class DictionaryFixture:
    def __init__(self):
        self.senses = {
            'finance': {'id': 'finance', 'lemma': 'bank', 'language': 'eng', 'definition': 'financial institution lending money', 'source_id': 'fixture'},
            'river': {'id': 'river', 'lemma': 'bank', 'language': 'eng', 'definition': 'land beside a river', 'source_id': 'fixture'},
            'shore': {'id': 'shore', 'lemma': 'shore', 'language': 'eng', 'definition': 'land beside a river'},
            'candidate': {'id': 'candidate', 'lemma': 'coast', 'language': 'eng', 'definition': 'sea land'},
        }
        self.alignments = {'finance': [], 'river': [{'concept_id': 'river-c', 'kind': 'equivalent', 'asserted': True}],
                           'shore': [{'concept_id': 'river-c', 'kind': 'equivalent', 'asserted': True}],
                           'candidate': [{'concept_id': 'river-c', 'kind': 'candidate', 'asserted': False}]}
        self.placements = {'river-c': {'position8': [1., 0, 0, 0, 0, 0, 0, 0], 'root_id': 'fixture-root'}}
        self.by_concept = {'river-c': {'river', 'shore', 'candidate'}}
    def counterpart(self, other, shared, origin):
        equivalents = {a['concept_id'] for a in self.alignments[other] if a['kind'] == 'equivalent' and a['asserted']}
        return {'linkStatus': 'source-equivalent' if set(shared) & equivalents else 'shared-concept-candidate', 'alignmentIds': [other]}


def chunk(text='river bank', cid='a'*64):
    return dict(chunkId=cid, kind='history', source='fixture', scope='', threadId='thread', messageId='message', noteId='',
                start=0, end=len(text), messageSha256=recall.digest(text), excerptSha256=recall.digest(text), text=text)


class RecallExperimentTests(unittest.TestCase):
    def setUp(self):
        self.interpreter = experiment.Interpreter(DictionaryFixture())

    def test_baseline_adaptation_only_relaxes_result_limit(self):
        self.assertEqual(experiment.BASELINE_SOURCE_SHA, recall.digest(__import__('inspect').getsource(recall.search)))
        with mock.patch.dict(experiment.SEARCH100.__globals__, status_unlocked=lambda *_: {'status': 'ready', 'model': {'digest': None, 'dimension': None}}):
            with self.assertRaises(recall.RecallError):
                experiment.SEARCH100(None, [], None, {'query': 'river', 'limit': 101})

    def test_context_selects_supported_sense(self):
        result = self.interpreter.interpret('river bank')
        self.assertEqual(result['selected'][0]['senseId'], 'river')
        self.assertIn('shore', result['expansionTerms'])

    def test_candidate_link_never_promoted_to_equivalence(self):
        self.assertNotIn('coast', self.interpreter.interpret('river bank')['expansionTerms'])

    def test_ambiguous_form_abstains(self):
        result = self.interpreter.interpret('bank')
        self.assertFalse(result['selected'])
        self.assertEqual(result['unresolved'][0]['candidateCount'], 2)

    def test_missing_definition_abstains(self):
        self.interpreter.dictionary.senses['river']['definition'] = ''
        self.assertFalse(self.interpreter.interpret('river bank')['selected'])

    def test_geometry_missing_is_none(self):
        self.assertIsNone(experiment.features('unknown words', chunk(), self.interpreter)['geometryCosine'])

    def test_root_collision_does_not_merge_concepts(self):
        info = {'vectors': [{'conceptId': 'a', 'rootId': 'same', 'position8': [1, 0, 0, 0, 0, 0, 0, 0]},
                            {'conceptId': 'b', 'rootId': 'same', 'position8': [0, 1, 0, 0, 0, 0, 0, 0]}]}
        self.assertEqual(experiment.mean_vector(info)[:2], [.5, .5])

    def test_negation_condition_and_role_are_explicit_proxies(self):
        result = experiment.features('Alice approved Bob', chunk('Bob approved Alice unless approved'), self.interpreter)
        self.assertEqual(result['role'], -1)
        self.assertTrue(result['conditionMismatch'])
        self.assertIn('proposed', result['qualification'])

    def test_unrelated_negation_not_attributed_outside_window(self):
        text = 'never ' + 'x '*200 + 'river bank'
        self.assertFalse(experiment.features('river bank', chunk(text), self.interpreter)['negationMismatch'])

    def test_faithful_explanation_and_mutations(self):
        query, original = 'river bank', chunk()
        features = experiment.features(query, original, self.interpreter)
        value = experiment.explanation(query, original, 'combined', features, .03, 1, .04)
        self.assertTrue(experiment.verify_explanation(value, query, original, self.interpreter, .03, 1, .04))
        for key in ('features', 'finalScore', 'baselineRrf', 'referenceRrf', 'contribution', 'citation'):
            changed = copy.deepcopy(value)
            changed.pop(key)
            changed['receiptSha256'] = recall.digest({k:v for k,v in changed.items() if k != 'receiptSha256'})
            with self.assertRaises(ValueError, msg=key):
                experiment.verify_explanation(changed, query, original, self.interpreter, .03, 1, .04)
        modified = dict(original, text='altered')
        with self.assertRaises(ValueError):
            experiment.verify_explanation(value, query, modified, self.interpreter, .03, 1, .04)
        with self.assertRaises(ValueError):
            experiment.verify_explanation(value, query, original, self.interpreter, .02, 1, .04)

    def test_unknown_gold_never_reaches_rank_interface(self):
        evaluator = experiment.Evaluator(Path('/tmp'), [], DictionaryFixture(), client=object())
        with self.assertRaises(ValueError):
            evaluator.rank({'query': 'river bank', 'targetChunkIds': ['a'*64]})

    def test_note_and_message_identity_remain_distinct(self):
        first = dict(chunk(), kind='note', scope='project', noteId='one')
        second = dict(first, noteId='two')
        self.assertNotEqual(experiment.message_identity(first), experiment.message_identity(second))
        self.assertEqual(experiment.message_identity(chunk()), ('fixture', 'thread', 'message'))

    def test_metrics_distinguish_mrr_at10(self):
        value = experiment.metrics(['x']*10+['gold'], {'gold'}, {'x'})
        self.assertEqual(value['mrrAt10'], 0)
        self.assertEqual(value['mrr'], 1/11)
        self.assertEqual(value['wrongSenseAt5'], 1)

    def test_private_output_no_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)/'receipt.json'
            experiment.private_write(target, {'ok': True})
            self.assertEqual(target.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                experiment.private_write(target, {})

    def test_cases_path_traversal_rejected_before_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'cases.json'
            path.write_text(json.dumps({'cases': [{'id': '../escape', 'query': 'river', 'targetChunkIds': ['a'*64]}]}))
            with self.assertRaises(ValueError):
                experiment.run(path, Path(directory)/'output', Path(directory), [])
            self.assertFalse((Path(directory)/'output').exists())

    def test_bounded_read_and_frozen_hash_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'input'
            path.write_bytes(b'12345')
            with self.assertRaises(ValueError):
                experiment.bounded_read(path, 4)
            self.assertEqual(experiment.bounded_read(path, 5), b'12345')
            with self.assertRaises(ValueError):
                experiment.run(path, Path(directory)/'output', Path(directory), [], '0'*64)

    def test_stale_or_missing_case_source_integrity_rejected(self):
        with self.assertRaises(ValueError):
            experiment.validate_frozen_sources({'cases': []}, Path('/tmp'), [], object())
        with self.assertRaises(ValueError):
            experiment.validate_frozen_sources({'cases': [], 'integrity': {'unchanged': False}}, Path('/tmp'), [], object())

    def test_frozen_metadata_span_and_hash_join(self):
        with tempfile.TemporaryDirectory() as directory:
            panel = Path(directory)
            (panel/'history').mkdir()
            (panel/'memory-recall').mkdir()
            (panel/'history/library.sqlite').write_bytes(b'fixture source')
            original = chunk()
            with sqlite3.connect(panel/'memory-recall/recall.sqlite') as db:
                db.execute('CREATE TABLE chunks(chunk_id TEXT,data TEXT)')
                db.execute('INSERT INTO chunks VALUES (?,?)', (original['chunkId'], json.dumps(original)))
            hashes = {'archive': experiment.sha_file(panel/'history/library.sqlite'), 'index': experiment.sha_file(panel/'memory-recall/recall.sqlite')}
            source = {'chunkId': original['chunkId'], 'excerptHash': original['excerptSha256'], 'messageId': original['messageId'],
                      'source': original['source'], 'span': [original['start'], original['end']], 'threadId': original['threadId']}
            payload = {'integrity': {'before': hashes, 'after': hashes, 'unchanged': True},
                       'cases': [{'targetChunkIds': [original['chunkId']], 'source': source, 'selectedText': original['text']}]}
            with mock.patch.object(recall, 'status_unlocked', return_value={'status': 'ready', 'generation': 'fixture'}), mock.patch.object(recall, 'evidence') as evidence:
                self.assertTrue(experiment.validate_frozen_sources(payload, panel, [], object()))
                evidence.assert_called_once()
                for key in ('span', 'excerptHash', 'threadId'):
                    corrupted = copy.deepcopy(payload)
                    corrupted['cases'][0]['source'][key] = 'changed'
                    with self.assertRaises(ValueError):
                        experiment.validate_frozen_sources(corrupted, panel, [], object())
                (panel/'history/library.sqlite').write_bytes(b'changed source')
                with self.assertRaises(ValueError):
                    experiment.validate_frozen_sources(payload, panel, [], object())

    def test_embedding_cache_includes_model_identity(self):
        client = experiment.CachedLocalModel()
        with mock.patch.object(client, 'identity', side_effect=['a'*64, 'a'*64, 'b'*64]), mock.patch.object(recall.LocalModel, 'embed', side_effect=[[[1, 0]], [[0, 1]]]) as underlying:
            self.assertEqual(client.embed(['query']), [[1, 0]])
            self.assertEqual(client.embed(['query']), [[1, 0]])
            self.assertEqual(client.embed(['query']), [[0, 1]])
            self.assertEqual(underlying.call_count, 2)
            self.assertEqual(client.cache_hits, 1)


if __name__ == '__main__':
    unittest.main()

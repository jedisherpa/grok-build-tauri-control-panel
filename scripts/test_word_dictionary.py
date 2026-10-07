"""Offline dictionary protocol and frozen inventory tests; no providers."""
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import word_dictionary as w


def fixture():
    graph = {
        'schema': 'semantic-e8-alignment-graph/v1',
        'sources': [{'id': 'test', 'language': 'eng', 'sha256': 'a'*64, 'relative_file': 'source.tab'}],
        'concepts': [
            {'id': 'pwn30:c1', 'label': 'bank', 'definition': 'shared river edge', 'definitions_by_language': {'eng': ['river edge']}, 'evidence_refs': [{'source_id': 'test', 'line_number': 1}]},
            {'id': 'c2', 'label': 'unplaced', 'definition': 'shared absent shape'},
        ],
        'senses': [
            {'id': 'sense:bank-eng', 'language': 'eng', 'lemma': 'bank', 'pos': 'n', 'definition': 'river edge', 'definition_language': 'eng', 'source_id': 'test', 'source_record_id': 's1', 'candidate_concept_ids': ['pwn30:c1'], 'evidence_refs': [{'source_id': 'test', 'line_number': 1}]},
            {'id': 'sense:bank-jpn', 'language': 'jpn', 'lemma': '岸', 'definition': None, 'candidate_concept_ids': ['pwn30:c1'], 'source_id': 'test'},
            {'id': 'sense:bank-cmn', 'language': 'cmn', 'lemma': '岸', 'definition': '河流的边缘', 'definition_language': 'cmn', 'candidate_concept_ids': ['pwn30:c1'], 'source_id': 'test'},
            {'id': 'sense:no-position', 'language': 'ind', 'lemma': 'tanpa', 'definition': None, 'candidate_concept_ids': ['c2'], 'source_id': 'test'},
            {'id': 'sense:unresolved', 'language': 'zsm', 'lemma': 'word', 'definition': '', 'candidate_concept_ids': [], 'source_id': 'test'},
        ],
        'alignments': [
            {'id': 'alignment:1', 'sense_id': 'sense:bank-eng', 'concept_id': 'pwn30:c1', 'kind': 'equivalent', 'asserted': True, 'human_gold': False, 'evidence_refs': [{'line_number': 1}]},
            {'id': 'alignment:2', 'sense_id': 'sense:bank-jpn', 'concept_id': 'pwn30:c1', 'kind': 'equivalent', 'asserted': True, 'human_gold': False},
        ],
    }
    model = {'schema': 'semantic-e8/typed-relation-fit/v1', 'source_graph_hash': w.canonical_hash(graph), 'model_id': 'fixture-model',
             'placements': [{'concept_id': 'pwn30:c1', 'status': 'fitted', 'position8': [0.125]*8, 'residual8': [0.01]*8, 'root_id': 'e8-root:1', 'root_index': 0}],
             'unplaced_concepts': [{'concept_id': 'c2', 'reason': 'no source relations'}]}
    return graph, model


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.graph, self.model = fixture()
        self.dictionary = w.Dictionary(self.graph, self.model, {})

    def query(self, **kwargs):
        request = {'referenceRoot': str(w.REFERENCE_ROOT), **kwargs}
        return self.dictionary.respond(w.request_options(request))

    def test_picker_asserted_concepts_require_exact_equivalent_source_join(self):
        self.assertEqual(self.query(query="sense:bank-eng")["hits"][0]["assertedConceptIds"], ["pwn30:c1"])
        self.assertEqual(self.query(query="sense:bank-cmn")["hits"][0]["assertedConceptIds"], [])
        original = self.dictionary.alignments["sense:bank-eng"][0]
        for changed in ({"kind":"language_specific"}, {"asserted":False}, {"sense_id":"other"}):
            saved = dict(original)
            original.update(changed)
            self.assertEqual(self.query(query="sense:bank-eng")["hits"][0]["assertedConceptIds"], [])
            original.clear(); original.update(saved)

    def test_exact_sense_identity_query(self):
        response = self.query(query='sense:no-position')
        self.assertEqual(response['resultCount'], 1)
        self.assertEqual(response['hits'][0]['senseId'], 'sense:no-position')

    def test_literal_definition_and_unicode(self):
        self.assertEqual(self.query(query='river edge')['resultCount'], 1)
        self.assertEqual(self.query(query='河流')['resultCount'], 1)
        self.assertEqual(self.query(query='岸')['resultCount'], 2)
        self.assertEqual(self.query(query='banks')['resultCount'], 0)

    def test_concept_inverse_keeps_unplaced_and_candidate_records(self):
        response = self.query(query='pwn30:c1')
        self.assertEqual(response['queryKind'], 'concept-id')
        self.assertEqual(response['resultCount'], 3)
        self.assertEqual({h['senseId'] for h in response['hits']}, {'sense:bank-eng', 'sense:bank-jpn', 'sense:bank-cmn'})
        self.assertEqual(self.query(query='c2')['hits'][0]['senseId'], 'sense:no-position')
        self.assertIsNone(self.query(query='c2')['hits'][0]['concepts'][0]['placement'])
        self.assertEqual(self.query(query='pwn30:unknown')['resultCount'], 0)

    def test_exact_root_inverse_paging_and_language_filters(self):
        first = self.query(query='e8-root:1', limit=2)
        self.assertEqual(first['queryKind'], 'root-id')
        self.assertEqual(first['resultCount'], 3)
        self.assertTrue(first['hasMore'])
        self.assertIn('does not establish synonymy', first['inverseNotice'])
        second = self.query(query='e8-root:1', offset=2, limit=2)
        self.assertEqual({h['senseId'] for h in first['hits'] + second['hits']}, {'sense:bank-eng', 'sense:bank-jpn', 'sense:bank-cmn'})
        self.assertEqual(self.query(query='e8-root:1', language='jpn')['resultCount'], 1)
        for query in ('e8-root:999', 'e8-root:01', 'E8-root:1'):
            self.assertEqual(self.query(query=query)['resultCount'], 0)
        self.assertEqual(self.query(query='bank')['queryKind'], 'literal')
        self.assertEqual(self.query(query='sense:bank-eng')['queryKind'], 'sense-id')

    def test_coverage_keeps_missing_definition_and_unplaced(self):
        coverage = self.query(action='coverage')['coverage']
        self.assertEqual(coverage['totalSenses'], 5)
        self.assertEqual(coverage['missingSenseLocalDefinitions'], 3)
        self.assertEqual(coverage['unplacedConcepts'], 1)
        missing = self.query(query='sense:bank-jpn')['hits'][0]
        self.assertIsNone(missing['definition'])
        self.assertEqual(missing['definitionStatus'], 'missing-sense-local-definition')
        self.assertEqual(missing['concepts'][0]['sharedConceptGloss'], 'shared river edge')

    def test_unplaced_never_gets_fabricated_vector(self):
        concept = self.query(query='sense:no-position')['hits'][0]['concepts'][0]
        self.assertIsNone(concept['placement'])
        self.assertEqual(concept['placementStatus'], 'unavailable')
        self.assertEqual(concept['unavailableReason'], 'no source relations')

    def test_native_placement_is_unchanged(self):
        placement = self.query(query='bank', language='eng')['hits'][0]['concepts'][0]['placement']
        self.assertEqual(placement['position8'], self.model['placements'][0]['position8'])
        self.assertEqual(placement['residual8'], self.model['placements'][0]['residual8'])

    def test_counterparts_are_cross_language_and_paged(self):
        hit = self.query(query='sense:bank-eng', counterpartLimit=1)['hits'][0]
        self.assertEqual(hit['counterpartCount'], 2)
        self.assertEqual(len(hit['counterparts']), 1)
        self.assertTrue(hit['counterpartsTruncated'])
        self.assertEqual(hit['nextCounterpartOffset'], 1)
        second = self.query(query='sense:bank-eng', counterpartOffset=1, counterpartLimit=1)['hits'][0]
        self.assertNotEqual(hit['counterparts'][0]['senseId'], second['counterparts'][0]['senseId'])
        self.assertEqual(hit['alignments'][0], self.graph['alignments'][0])
        self.assertEqual(hit['counterparts'][0]['sharedConceptIds'], ['pwn30:c1'])

    def test_candidate_counterpart_is_not_asserted_equivalence(self):
        hit = self.query(query='sense:bank-eng')['hits'][0]
        statuses = {c['language']: c for c in hit['counterparts']}
        self.assertEqual(statuses['jpn']['linkStatus'], 'source-equivalent')
        self.assertEqual(statuses['jpn']['assertedEquivalentSharedConceptIds'], ['pwn30:c1'])
        self.assertEqual(statuses['cmn']['linkStatus'], 'shared-concept-candidate')
        self.assertEqual(statuses['cmn']['assertedEquivalentSharedConceptIds'], [])
        self.assertEqual(statuses['cmn']['candidateSharedConceptIds'], ['pwn30:c1'])

    def test_empty_query_enumerates_all_records_with_exact_count(self):
        first = self.query(limit=2)
        self.assertEqual(first['resultCount'], 5)
        self.assertEqual(first['returnedCount'], 2)
        self.assertEqual(first['nextOffset'], 2)
        self.assertTrue(first['hasMore'])
        self.assertTrue(first['truncated'])
        pages = [self.query(offset=i, limit=2)['hits'] for i in (0, 2, 4)]
        self.assertEqual({h['senseId'] for page in pages for h in page}, set(self.dictionary.senses))

    def test_invalid_protocol_parameters(self):
        for changes in ({'referenceRoot': '/tmp'}, {'path': '/tmp'}, {'limit': 21}, {'limit': True}, {'limit': 0}, {'offset': -1}, {'language': 'xx'}, {'action': 'embed'}, {'query': 'a'*257}, {'counterpartLimit': 21}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                w.request_options({'referenceRoot': str(w.REFERENCE_ROOT), **changes})

    def test_graph_model_mismatch_rejected(self):
        self.model['source_graph_hash'] = 'bad'
        with self.assertRaisesRegex(ValueError, 'identity mismatch'):
            w.Dictionary(self.graph, self.model, {})

    def test_nonfinite_geometry_rejected(self):
        self.model['placements'][0]['position8'][0] = float('inf')
        with self.assertRaises(ValueError):
            w.Dictionary(self.graph, self.model, {})

    def test_unknown_alignment_rejected(self):
        self.graph['alignments'][0]['sense_id'] = 'missing'
        self.model['source_graph_hash'] = w.canonical_hash(self.graph)
        with self.assertRaisesRegex(ValueError, 'unknown sense'):
            w.Dictionary(self.graph, self.model, {})

    def test_duplicate_identity_rejected(self):
        self.graph['senses'].append(copy.deepcopy(self.graph['senses'][0]))
        self.model['source_graph_hash'] = w.canonical_hash(self.graph)
        with self.assertRaisesRegex(ValueError, 'identity invalid'):
            w.Dictionary(self.graph, self.model, {})

    def test_member_pin_size_hash_symlink_containment(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            member = root/'member.json'
            raw = b'{"fixture":true}'
            member.write_bytes(raw)
            digest = hashlib.sha256(raw).hexdigest()
            self.assertEqual(w.read_pinned(root, 'member.json', 100, digest, len(raw))[0], {'fixture': True})
            for maximum, pin, size in ((2, digest, len(raw)), (100, 'bad', len(raw)), (100, digest, 99)):
                with self.assertRaises(ValueError):
                    w.read_pinned(root, 'member.json', maximum, pin, size)
            (root/'symlink.json').symlink_to(member)
            with self.assertRaises(ValueError):
                w.read_pinned(root, 'symlink.json', 100)
            with self.assertRaises(ValueError):
                w.read_pinned(root, '../outside.json', 100)

    def test_ascii_parse_buffer_preserves_unicode_and_non_bmp(self):
        value = {'definition': '岸 café 𠮷 😀', 'literal': '\\u1234', 'newline': 'one\ntwo'}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            raw = json.dumps(value, ensure_ascii=False).encode('utf-8')
            (root/'unicode.json').write_bytes(raw)
            decoded, digest = w.read_pinned(root, 'unicode.json', 1024)
            self.assertEqual(decoded, value)
            self.assertEqual(digest, hashlib.sha256(raw).hexdigest())

    def test_manifest_verified_before_member_reads(self):
        with patch.object(w, 'read_pinned', side_effect=ValueError('manifest changed')) as reader:
            with self.assertRaisesRegex(ValueError, 'manifest changed'):
                w.Dictionary.load()
            reader.assert_called_once_with(w.REFERENCE_ROOT, w.MANIFEST_PATH, w.MAX_MANIFEST, w.MANIFEST_SHA)

    def test_synthetic_manifest_members_load_and_mutation_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            members = []
            for relative, data in ((w.GRAPH_PATH, self.graph), (w.MODEL_PATH, self.model)):
                path = root/relative
                path.parent.mkdir(parents=True, exist_ok=True)
                raw = json.dumps(data, ensure_ascii=False).encode()
                path.write_bytes(raw)
                members.append({'path': relative, 'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest()})
            for relative in w.SENSESNAP_PATHS:
                path = root/relative
                path.parent.mkdir(parents=True, exist_ok=True)
                raw = b'# fixture implementation\n'
                path.write_bytes(raw)
                members.append({'path': relative, 'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest()})
            manifest = root/w.MANIFEST_PATH
            manifest.parent.mkdir(parents=True)
            raw = json.dumps({'files': members}).encode()
            manifest.write_bytes(raw)
            with patch.object(w, 'REFERENCE_ROOT', root), patch.object(w, 'MANIFEST_SHA', hashlib.sha256(raw).hexdigest()):
                self.assertEqual(w.Dictionary.load().coverage['totalSenses'], 5)
                (root/w.GRAPH_PATH).write_bytes(b'changed')
                with self.assertRaisesRegex(ValueError, 'size invalid'):
                    w.Dictionary.load()
                (root/w.GRAPH_PATH).write_bytes(json.dumps(self.graph, ensure_ascii=False).encode())
                implementation = root/w.SENSESNAP_PATHS[0]
                implementation.write_bytes(implementation.read_bytes().replace(b'fixture', b'changed'))
                with self.assertRaisesRegex(ValueError, 'hash changed'):
                    w.Dictionary.load()

    def test_response_output_budget(self):
        with self.assertRaisesRegex(ValueError, '1 MiB'):
            w.encode_response({'text': 'x'*(w.MAX_OUTPUT+1)})
        self.assertLess(len(w.encode_response(self.query())), w.MAX_OUTPUT)

    def test_malformed_and_oversized_cli_do_not_load_reference(self):
        for payload in (b'{', b'x'*(w.MAX_INPUT+1), b'{"limit":NaN}'):
            result = subprocess.run([sys.executable, '-B', str(Path(w.__file__))], input=payload, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(json.loads(result.stdout)['status'], 'unavailable')
            self.assertLess(len(result.stdout), 1000)


@unittest.skipUnless((w.REFERENCE_ROOT/w.MANIFEST_PATH).is_file(), 'Installed frozen dictionary unavailable')
class InstalledInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dictionary = w.Dictionary.load()

    def test_complete_frozen_census(self):
        coverage = self.dictionary.coverage
        self.assertEqual(coverage['totalSenses'], 34801)
        self.assertEqual(coverage['totalConcepts'], 2205)
        self.assertEqual(coverage['fittedConcepts'], 2007)
        self.assertEqual(coverage['senseLocalDefinitions'], 16435)
        self.assertEqual(coverage['missingSenseLocalDefinitions'], 18366)
        self.assertEqual(sum(c['senses'] for c in coverage['languages'].values()), 34801)

    def test_bank_exact_forms_and_literal_related_matches(self):
        response = self.dictionary.respond(w.request_options({'referenceRoot': str(w.REFERENCE_ROOT), 'query': 'bank', 'language': 'eng'}))
        self.assertEqual(sum(s['lemma'] == 'bank' and s['language'] == 'eng' for s in self.dictionary.senses.values()), 18)
        self.assertGreaterEqual(response['resultCount'], 18)
        self.assertEqual(sum(h['lemma'] == 'bank' for h in response['hits']), 18)
        self.assertLess(len(w.encode_response(response)), w.MAX_OUTPUT)

    def test_actual_root_inverse_enumerates_exact_original_ids(self):
        dictionary = self.dictionary
        root_id = 'e8-root:120'
        concepts = {cid for cid, placement in dictionary.placements.items() if placement['root_id'] == root_id}
        expected = {s['id'] for s in dictionary.senses.values() if set(dictionary.concept_ids(s)) & concepts}
        self.assertTrue(expected)
        actual = set()
        for offset in range(0, len(expected), 20):
            response = dictionary.respond(w.request_options({'referenceRoot': str(w.REFERENCE_ROOT), 'query': root_id, 'offset': offset}))
            self.assertEqual(response['queryKind'], 'root-id')
            self.assertEqual(response['resultCount'], len(expected))
            actual.update(h['senseId'] for h in response['hits'])
            self.assertLessEqual(len(w.encode_response(response)), w.MAX_OUTPUT)
        self.assertEqual(actual, expected)
        for language in w.LANGUAGES:
            response = dictionary.respond(w.request_options({'referenceRoot': str(w.REFERENCE_ROOT), 'query': root_id, 'language': language}))
            self.assertEqual(response['resultCount'], sum(dictionary.senses[sid]['language'] == language for sid in expected))
        unplaced = next(cid for cid in dictionary.unplaced if dictionary.by_concept[cid])
        response = dictionary.respond(w.request_options({'referenceRoot': str(w.REFERENCE_ROOT), 'query': unplaced}))
        self.assertEqual(response['queryKind'], 'concept-id')
        self.assertEqual(response['resultCount'], len(dictionary.by_concept[unplaced]))
        self.assertTrue(all(next(c for c in h['concepts'] if c['conceptId'] == unplaced)['placement'] is None for h in response['hits']))

    def test_all_languages_ids_and_gaps_queryable(self):
        for language in w.LANGUAGES:
            language_senses = [s for s in self.dictionary.senses.values() if s["language"] == language]
            sense = next((s for s in language_senses if not s.get("definition")), language_senses[0])
            response = self.dictionary.respond(w.request_options({'referenceRoot': str(w.REFERENCE_ROOT), 'query': sense['id'], 'language': language}))
            self.assertEqual(response['resultCount'], 1)
            self.assertEqual(response['hits'][0]['senseId'], sense['id'])
        unplaced = next(s for s in self.dictionary.senses.values() if self.dictionary.concept_ids(s) and all(c not in self.dictionary.placements for c in self.dictionary.concept_ids(s)))
        response = self.dictionary.respond(w.request_options({'referenceRoot': str(w.REFERENCE_ROOT), 'query': unplaced['id']}))
        self.assertTrue(all(c['placement'] is None for c in response['hits'][0]['concepts']))


if __name__ == '__main__':
    unittest.main()

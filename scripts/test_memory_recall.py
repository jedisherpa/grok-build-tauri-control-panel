"""Regression tests use private temporary sources and deterministic model doubles."""
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import sqlite3
import struct
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('memory_recall', Path(__file__).with_name('memory_recall.py'))
recall = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recall)


class FakeModel:
    def __init__(self, available=True):
        self.available = available
        self.digest = 'a' * 64
        self.calls = []

    def identity(self):
        return self.digest if self.available else None

    def embed(self, texts):
        self.calls.append(texts)
        # This deterministic double verifies rank mechanics, not model accuracy.
        vectors = []
        for text in texts:
            lower = text.lower()
            if any(word in lower for word in ('car', 'automobile', 'transport')):
                vectors.append([1.0, 0.0, 0.0])
            elif any(word in lower for word in ('garden', 'flowers')):
                vectors.append([0.0, 1.0, 0.0])
            else:
                vectors.append([0.0, 0.0, 1.0])
        return vectors


class RecallTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.panel = Path(self.temp.name)
        (self.panel / 'history').mkdir()
        self.history = self.panel / 'history/library.sqlite'
        with sqlite3.connect(self.history) as db:
            db.executescript('''CREATE TABLE threads(id TEXT PRIMARY KEY,source TEXT,origin_id TEXT,
                title TEXT,parent_id TEXT,coverage TEXT);
                CREATE TABLE messages(thread_id TEXT,message_id TEXT,role TEXT,text TEXT,at TEXT,
                seq INTEGER,truncated INTEGER, PRIMARY KEY(thread_id,message_id));''')
        self.model = FakeModel()
        self.notes = [{'id': 'n1', 'scope': 'project-A', 'content': 'Garden flowers need water.',
                       'tags': ['garden'], 'created_at': '2026-01-01', 'updated_at': '2026-01-01'},
                      {'id': 'n2', 'scope': 'project-B', 'content': 'Garden flowers need sunlight.',
                       'tags': [], 'created_at': '', 'updated_at': ''}]
        self.add('t1', 'm1', 'A car is useful for transport.', source='codex')
        self.add('t2', 'm2', 'Garden flowers bloom in spring.', source='chatgpt')

    def tearDown(self):
        self.temp.cleanup()

    def add(self, tid, mid, text, source='codex', role='user', parent='', truncated=0, origin=None):
        with sqlite3.connect(self.history) as db:
            db.execute('INSERT OR IGNORE INTO threads VALUES (?,?,?,?,?,?)',
                       (tid, source, origin or tid, 'Title ' + tid, parent, 'local transcript'))
            db.execute('INSERT INTO messages VALUES (?,?,?,?,?,?,?)',
                       (tid, mid, role, text, '2026-01-01T00:00:00Z', 1, truncated))

    def call(self, action, **kwargs):
        return recall.dispatch(action, str(self.panel), dict(notes=self.notes, **kwargs), self.model)

    def test_status_does_not_create_or_embed(self):
        result = self.call('status')
        self.assertEqual(result['status'], 'missing')
        self.assertFalse((self.panel / 'memory-recall').exists())
        self.assertEqual(self.model.calls, [])

    def test_original_unchanged_private_permissions_and_counts(self):
        before = self.history.read_bytes()
        result = self.call('index')
        self.assertEqual(result['chunkCount'], 4)
        self.assertEqual(result['historyChunks'], 2)
        self.assertEqual(result['noteChunks'], 2)
        self.assertEqual(before, self.history.read_bytes())
        self.assertEqual((self.panel / 'memory-recall').stat().st_mode & 0o777, 0o700)
        self.assertEqual((self.panel / 'memory-recall/recall.sqlite').stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.model.calls, [])

    def test_live_wal_source_readonly_snapshot_and_freshness(self):
        with sqlite3.connect(self.history) as writer:
            writer.execute('PRAGMA journal_mode=WAL')
            writer.execute('INSERT INTO messages VALUES (?,?,?,?,?,?,?)',
                           ('t1', 'wal1', 'user', 'A new car statement.', '', 2, 0))
            writer.commit()
            before_db = self.history.read_bytes()
            before_wal = Path(str(self.history) + '-wal').read_bytes()
            result = self.call('index')
            self.assertEqual(result['historyChunks'], 3)
            self.assertEqual(before_db, self.history.read_bytes())
            self.assertEqual(before_wal, Path(str(self.history) + '-wal').read_bytes())
            writer.execute('UPDATE messages SET text=? WHERE message_id=?', ('A changed car statement.', 'wal1'))
            writer.commit()
            self.assertEqual(self.call('status')['status'], 'stale')

    def test_duplicate_note_identity_and_index_byte_budget(self):
        self.notes.append(dict(self.notes[0]))
        with self.assertRaisesRegex(recall.RecallError, 'duplicate'):
            self.call('index')
        self.notes.pop()
        old = self.call('index')
        with mock.patch.object(recall, 'MAX_INDEX_BYTES', 1):
            with self.assertRaisesRegex(recall.RecallError, 'text budget'):
                self.call('index')
        self.assertEqual(self.call('status')['generation'], old['generation'])

    def test_compact_float32_storage_and_decoding_validation(self):
        self.call('index')
        self.call('embed_batch', maxDocuments=128)
        with sqlite3.connect(self.panel / 'memory-recall/recall.sqlite') as db:
            rows = list(db.execute('SELECT typeof(vector),length(vector),dimension,vector FROM vectors'))
        self.assertTrue(rows)
        for storage, size, dimension, blob in rows:
            self.assertEqual(storage, 'blob')
            self.assertEqual(size, dimension * 4)
            vector = recall.decode_vector(blob, dimension)
            self.assertAlmostEqual(math.fsum(x * x for x in vector), 1.0)
        encoded = recall.encode_vector([1e308, -1e308])
        decoded = recall.decode_vector(encoded, 2)
        self.assertAlmostEqual(decoded[0], math.sqrt(0.5))
        self.assertAlmostEqual(decoded[1], -math.sqrt(0.5))
        for blob, dimension in ((b'bad', 3), (struct.pack('<f', math.nan), 1),
                                (struct.pack('<f', math.inf), 1), (b'\0' * 4, 1)):
            with self.assertRaises(recall.RecallError):
                recall.decode_vector(blob, dimension)
        self.assertEqual(self.call('status')['embeddingBasis']['vectorStorage'], 'float32-le/v1')

    def test_actual_sqlite_page_budget_rejects_rebuild_and_preserves_old_file(self):
        old = self.call('index')
        path = self.panel / 'memory-recall/recall.sqlite'
        before = path.read_bytes()
        actual_size = old['databaseBytes']
        self.assertEqual(actual_size, path.stat().st_size)
        self.assertEqual(old['databaseBudgetBytes'], recall.MAX_DATABASE_BYTES)
        for number in range(60):
            self.add('extra' + str(number), 'mx' + str(number), 'new text ' * 110)
        with mock.patch.object(recall, 'MAX_DATABASE_BYTES', actual_size):
            with self.assertRaises((recall.RecallError, sqlite3.Error)):
                self.call('index')
        self.assertEqual(before, path.read_bytes())
        self.assertEqual(self.call('status')['generation'], old['generation'])
        self.assertEqual(list((self.panel / 'memory-recall').glob('rebuild-*')), [])

    def test_actual_sqlite_page_budget_rolls_back_only_failed_embedding_group(self):
        self.call('index')
        self.model.embed = lambda texts: [[1.0] + [0.0] * 4095 for _ in texts]
        first = self.call('embed_batch', maxDocuments=1)
        self.assertEqual(first['embedded'], 1)
        path = self.panel / 'memory-recall/recall.sqlite'
        before = path.read_bytes()
        with mock.patch.object(recall, 'MAX_DATABASE_BYTES', first['databaseBytes']):
            failed = self.call('embed_batch', maxDocuments=128)
        self.assertEqual(failed['embedded'], 0)
        self.assertEqual(failed['storedVectorCount'], 1)
        self.assertIn('embeddingError', failed)
        self.assertEqual(before, path.read_bytes())
        self.assertEqual(self.call('status')['vectorCount'], 1)

    def test_free_disk_reserve_refuses_writes_without_cleanup_or_provider_calls(self):
        old = self.call('index')
        path = self.panel / 'memory-recall/recall.sqlite'
        before = path.read_bytes()
        usage = recall.shutil.disk_usage(self.panel)
        low = usage._replace(free=recall.MIN_FREE_DISK_BYTES)
        with mock.patch.object(recall.shutil, 'disk_usage', return_value=low):
            with self.assertRaisesRegex(recall.RecallError, 'free disk reserve'):
                self.call('index')
            failed = self.call('embed_batch', maxDocuments=128)
        self.assertEqual(self.model.calls, [])
        self.assertEqual(failed['embedded'], 0)
        self.assertIn('free disk reserve', failed['embeddingError'])
        self.assertEqual(before, path.read_bytes())
        self.assertEqual(self.call('status')['generation'], old['generation'])

    def test_lexical_recall_without_local_provider(self):
        self.model.available = False
        self.call('index')
        result = self.call('search', query='transport')
        self.assertEqual(result['hits'][0]['messageId'], 'm1')
        self.assertEqual(result['vectorStatus'], 'unavailable')
        self.assertIsNone(result['hits'][0]['vectorRank'])
        self.assertEqual(self.model.calls, [])

    def test_paraphrase_rank_rrf_and_duplicate_merge(self):
        self.call('index')
        self.call('embed_batch', maxDocuments=128)
        result = self.call('search', query='automobile', source='history')
        self.assertEqual(result['hits'][0]['messageId'], 'm1')
        self.assertIsNone(result['hits'][0]['keywordRank'])
        self.assertEqual(result['hits'][0]['vectorRank'], 1)
        exact = self.call('search', query='transport', source='history')
        car = exact['hits'][0]
        self.assertEqual(car['rankScore'], 2 / 61)
        self.assertEqual(len([hit for hit in exact['hits'] if hit['messageId'] == 'm1']), 1)
        self.assertTrue(all(text.startswith(('search_document: ', 'search_query: '))
                            for batch in self.model.calls for text in batch))

    def test_unicode_half_open_spans_and_source_hash(self):
        text = '🌊' * 1001 + ' café 車'
        self.add('unicode', 'um', text)
        self.call('index')
        path = self.panel / 'memory-recall/recall.sqlite'
        with sqlite3.connect(path) as db:
            data = [json.loads(row[0]) for row in db.execute('SELECT data FROM chunks')]
        chunks = sorted((row for row in data if row['messageId'] == 'um'), key=lambda row: row['start'])
        self.assertEqual([row['start'] for row in chunks], [0, 1000])
        self.assertEqual(''.join(row['text'] for row in chunks), text)
        self.assertEqual(chunks[0]['messageSha256'], hashlib.sha256(text.encode()).hexdigest())
        self.assertEqual(chunks[0]['end'], 1000)
        result = self.call('evidence', generation=self.call('status')['generation'],
                           chunkIds=[row['chunkId'] for row in chunks])
        self.assertIn('🌊', result['passage'])
        self.assertEqual(result['charCount'], len(result['passage']))

    def test_filters_and_no_scope_leak(self):
        self.call('index')
        self.call('embed_batch', maxDocuments=128)
        result = self.call('search', query='garden', source='notes', scope='project-A')
        self.assertEqual([hit['noteId'] for hit in result['hits']], ['n1'])
        history = self.call('search', query='garden', source='chatgpt')
        self.assertTrue(all(hit['source'] == 'chatgpt' for hit in history['hits']))
        thread = self.call('search', query='garden car', threadId='t1')
        self.assertEqual([hit['threadId'] for hit in thread['hits']], ['t1'])
        topic = self.call('search', query='car garden', topic='spring', source='history')
        self.assertEqual([hit['messageId'] for hit in topic['hits']], ['m2'])
        self.assertEqual(topic['topicFilter'], 'explicit lexical terms')

    def test_claude_source_filter_matches_indexed_provider(self):
        self.add('claude-thread', 'claude-message', 'Garden flowers from Claude history.', source='claude')
        self.call('index')
        result = self.call('search', query='flowers', source='claude')
        self.assertEqual([hit['messageId'] for hit in result['hits']], ['claude-message'])
        self.assertTrue(all(hit['source'] == 'claude' for hit in result['hits']))

    def test_saved_note_metadata_is_revalidated_against_host_snapshot(self):
        index = self.call('index')
        hit = self.call('search', query='garden', source='notes', scope='project-A')['hits'][0]
        path = self.panel / 'memory-recall/recall.sqlite'
        with sqlite3.connect(path) as db:
            original = json.loads(db.execute('SELECT data FROM chunks WHERE chunk_id=?', (hit['chunkId'],)).fetchone()[0])
        changes = {'title': 'A forged title', 'at': '2030-01-01', 'role': 'user',
                   'source': 'codex', 'threadId': 'forged-thread', 'messageId': 'forged-message',
                   'coverage': 'Verified fact'}
        for field, value in changes.items():
            with self.subTest(field=field):
                corrupted = dict(original, **{field: value})
                with sqlite3.connect(path) as db:
                    db.execute('UPDATE chunks SET data=? WHERE chunk_id=?', (json.dumps(corrupted), hit['chunkId']))
                with self.assertRaisesRegex(recall.RecallError, 'saved note provenance'):
                    self.call('evidence', generation=index['generation'], chunkIds=[hit['chunkId']])
        with sqlite3.connect(path) as db:
            db.execute('UPDATE chunks SET data=? WHERE chunk_id=?', (json.dumps(original), hit['chunkId']))
        clean = self.call('evidence', generation=index['generation'], chunkIds=[hit['chunkId']])
        self.assertEqual(clean['evidence'][0]['title'], self.notes[0]['content'].splitlines()[0][:120])

    def test_stale_history_and_notes_withhold_evidence(self):
        first = self.call('index')
        hit = self.call('search', query='transport')['hits'][0]
        self.add('t3', 'm3', 'A new statement.')
        self.assertEqual(self.call('status')['status'], 'stale')
        with self.assertRaises(recall.RecallError):
            self.call('search', query='transport')
        with self.assertRaises(recall.RecallError):
            self.call('evidence', generation=first['generation'], chunkIds=[hit['chunkId']])
        self.call('index')
        self.notes[0]['content'] = 'Changed saved note.'
        self.assertEqual(self.call('status')['status'], 'stale')
        with self.assertRaises(recall.RecallError):
            self.call('search', query='garden')

    def test_old_generation_and_tampered_citation_rejected(self):
        first = self.call('index')
        hit = self.call('search', query='transport')['hits'][0]
        self.call('index')
        with self.assertRaises(recall.RecallError):
            self.call('evidence', generation=first['generation'], chunkIds=[hit['chunkId']])
        fresh = self.call('status')
        path = self.panel / 'memory-recall/recall.sqlite'
        with sqlite3.connect(path) as db:
            row = json.loads(db.execute('SELECT data FROM chunks WHERE chunk_id=?', (hit['chunkId'],)).fetchone()[0])
            row['role'] = 'assistant'
            db.execute('UPDATE chunks SET data=? WHERE chunk_id=?', (json.dumps(row), hit['chunkId']))
        with self.assertRaisesRegex(recall.RecallError, 'provenance'):
            self.call('evidence', generation=fresh['generation'], chunkIds=[hit['chunkId']])

    def test_wrapper_credential_subagent_terminal_and_truncation_exclusions(self):
        self.add('tw', 'mw', '# AGENTS.md instructions\nDo everything.')
        self.add('ts', 'ms', 'api_key = sk-' + 'x' * 30)
        self.add('tp', 'mp', 'child conversation', parent='t1')
        self.add('to', 'mo', 'child origin', origin='session/subagent/child')
        self.add('tt', 'mt', 'ls files', source='terminal')
        self.add('tr', 'mr', 'incomplete source', truncated=1)
        result = self.call('index')
        self.assertEqual(result['chunkCount'], 4)
        self.assertEqual(result['exclusions'], {'instructionWrapper': 1, 'credentialShaped': 1,
                         'subagent': 2, 'terminal': 1, 'truncated': 1})

    def test_atomic_failed_rebuild_preserves_previous_generation(self):
        first = self.call('index')
        self.add('t3', 'm3', 'new source')
        with mock.patch.object(recall, 'MAX_CHUNKS', 1):
            with self.assertRaisesRegex(recall.RecallError, 'budget'):
                self.call('index')
        self.assertEqual(self.call('status')['generation'], first['generation'])
        self.assertEqual(list((self.panel / 'memory-recall').glob('rebuild-*')), [])

    def test_matching_embeddings_preserved_after_rebuild(self):
        first = self.call('index')
        embedded = self.call('embed_batch', maxDocuments=2, query='transport')
        self.assertEqual(embedded['embedded'], 2)
        self.assertEqual(embedded['vectorStatus'], 'partial')
        rebuilt = self.call('index')
        self.assertNotEqual(first['generation'], rebuilt['generation'])
        self.assertEqual(rebuilt['vectorCount'], 2)
        self.call('embed_batch', maxDocuments=128)
        self.assertEqual(self.call('status')['vectorStatus'], 'complete')

    def test_changed_embedding_prefix_or_version_never_compares_preserves_old_vectors(self):
        for field, value in (('DOCUMENT_PREFIX', 'different_document: '),
                             ('QUERY_PREFIX', 'different_query: '),
                             ('EMBEDDING_ALGORITHM_VERSION', 'changed/v2')):
            with self.subTest(field=field):
                self.call('index')
                self.call('embed_batch', maxDocuments=128)
                original = self.call('status')
                before_calls = len(self.model.calls)
                with mock.patch.object(recall, field, value):
                    changed = self.call('status')
                    self.assertFalse(changed['embeddingBasisCompatible'])
                    self.assertEqual(changed['vectorStatus'], 'basis-changed')
                    self.assertEqual(changed['vectorCount'], 0)
                    self.assertNotEqual(changed['embeddingBasis']['fingerprint'], original['embeddingBasis']['fingerprint'])
                    self.assertEqual(self.call('search', query='automobile')['hits'], [])
                    lexical = self.call('search', query='transport')['hits']
                    self.assertTrue(lexical)
                    self.assertTrue(all(hit['vectorRank'] is None for hit in lexical))
                    self.assertEqual(len(self.model.calls), before_calls)
                    with self.assertRaisesRegex(recall.RecallError, 'basis changed'):
                        self.call('embed_batch', maxDocuments=128)
                    rebuilt = self.call('index')
                    self.assertTrue(rebuilt['embeddingBasisCompatible'])
                    self.assertEqual(rebuilt['storedVectorCount'], 0)
                    self.assertEqual(rebuilt['vectorCount'], 0)

    def test_legacy_embedding_basis_requires_explicit_rebuild(self):
        self.call('index')
        self.call('embed_batch', maxDocuments=128)
        path = self.panel / 'memory-recall/recall.sqlite'
        with sqlite3.connect(path) as db:
            metadata = json.loads(db.execute('SELECT value FROM metadata WHERE key="index"').fetchone()[0])
            metadata.pop('embeddingBasisFingerprint')
            db.execute('UPDATE metadata SET value=? WHERE key="index"', (json.dumps(metadata),))
        result = self.call('status')
        self.assertFalse(result['embeddingBasisCompatible'])
        self.assertEqual(result['vectorCount'], 0)
        self.assertEqual(result['vectorStatus'], 'basis-changed')
        self.assertEqual(self.call('search', query='automobile')['hits'], [])
        with self.assertRaises(recall.RecallError):
            self.call('embed_batch', maxDocuments=128)
        rebuilt = self.call('index')
        self.assertTrue(rebuilt['embeddingBasisCompatible'])
        self.assertEqual(rebuilt['storedVectorCount'], 0)

    def test_model_change_never_compares_incompatible_vectors(self):
        self.call('index')
        self.call('embed_batch', maxDocuments=128)
        self.model.digest = 'b' * 64
        result = self.call('search', query='automobile')
        self.assertEqual(result['vectorStatus'], 'model-changed')
        self.assertEqual(result['hits'], [])
        self.assertEqual(result['vectorCount'], 0)

    def test_model_race_withholds_new_vectors_and_uses_lexical_only(self):
        self.call('index')
        original = self.model.embed
        def changed(texts):
            value = original(texts)
            self.model.digest = 'b' * 64
            return value
        self.model.embed = changed
        result = self.call('embed_batch', maxDocuments=128)
        self.assertEqual(result['embedded'], 0)
        self.assertIn('changed', result['embeddingError'])
        self.assertEqual(result['storedVectorCount'], 0)
        self.model.embed = original
        self.call('embed_batch', maxDocuments=128)
        self.model.embed = changed
        self.model.digest = 'a' * 64
        # Re-embed using a pinned identity before racing a query.
        self.model.embed = original
        self.call('embed_batch', maxDocuments=128)
        self.model.embed = changed
        queried = self.call('search', query='transport')
        self.assertTrue(queried['hits'])
        self.assertTrue(all(hit['vectorRank'] is None for hit in queried['hits']))
        self.assertEqual(queried['vectorStatus'], 'query-unavailable')

    def test_invalid_vectors_rejected_without_partial_batch(self):
        self.call('index')
        for invalid in ([0.0, 0.0], [math.nan, 1], [math.inf], [True, 1], []):
            self.model.embed = lambda texts: [invalid for _ in texts]
            result = self.call('embed_batch', maxDocuments=128)
            self.assertEqual(result['embedded'], 0)
            self.assertEqual(result['storedVectorCount'], 0)
        self.assertAlmostEqual(recall.normalized([1e308, 1e308])[0], math.sqrt(0.5))
        self.assertAlmostEqual(recall.normalized([5e-324, 5e-324])[0], math.sqrt(0.5))

    def test_dimension_changes_rejected(self):
        self.call('index')
        self.call('embed_batch', maxDocuments=1)
        self.model.embed = lambda texts: [[1, 0] for _ in texts]
        result = self.call('embed_batch', maxDocuments=128)
        self.assertIn('dimension', result['embeddingError'])
        self.assertEqual(result['vectorCount'], 1)

    def test_bounds_and_literal_fts_input(self):
        self.call('index')
        for maximum in (0, 129, True):
            with self.assertRaises(recall.RecallError):
                self.call('embed_batch', maxDocuments=maximum)
        for limit in (0, 21, True):
            with self.assertRaises(recall.RecallError):
                self.call('search', query='car', limit=limit)
        literal = self.call('search', query='"car" OR NEAR(transport, 5)*')
        self.assertTrue(literal['hits'])
        with self.assertRaises(recall.RecallError):
            self.call('search', query='x', source='unknown')
        with self.assertRaises(recall.RecallError):
            self.call('evidence', generation=self.call('status')['generation'], chunkIds=[{}])

    def test_unicode_topic_filter_and_vector_scan_limit_are_explicit(self):
        self.add('unicode', 'um', 'CAFÉ 車 flowers')
        self.call('index')
        self.call('embed_batch', maxDocuments=128)
        result = self.call('search', query='flowers', topic='café')
        self.assertEqual([hit['messageId'] for hit in result['hits']], ['um'])
        with mock.patch.object(recall, 'MAX_VECTOR_SCAN', 1):
            limited = self.call('search', query='transport')
        self.assertTrue(limited['vectorScanLimited'])
        self.assertEqual(limited['vectorCandidatesScanned'], 1)
        self.assertEqual(limited['vectorStatus'], 'retrieval-limited')

    def test_source_changes_during_query_and_embedding_withhold_results(self):
        self.call('index')
        self.call('embed_batch', maxDocuments=128)
        actual = self.model.embed
        def changed(texts):
            value = actual(texts)
            self.add('changed', 'changed', 'history changed during query')
            return value
        self.model.embed = changed
        with self.assertRaisesRegex(recall.RecallError, 'during search'):
            self.call('search', query='transport')
        self.model.embed = actual
        self.call('index')
        self.model.embed = changed
        # Change another record to keep the injected write unique.
        with sqlite3.connect(self.history) as db:
            db.execute('DELETE FROM messages WHERE message_id="changed"')
        self.add('pending', 'pending', 'An unembedded source message.')
        self.model.embed = actual
        self.call('index')
        self.model.embed = changed
        result = self.call('embed_batch', maxDocuments=128)
        self.assertEqual(result['embedded'], 0)
        self.assertIn('history changed', result['embeddingError'])

    def test_evidence_character_budget_rejects_whole_packet(self):
        for number in range(8):
            self.add('long' + str(number), 'ml' + str(number), 'x' * 1000)
        self.call('index')
        with sqlite3.connect(self.panel / 'memory-recall/recall.sqlite') as db:
            ids = [row[0] for row in db.execute('SELECT chunk_id FROM chunks WHERE thread_id LIKE "long%"')]
        with self.assertRaisesRegex(recall.RecallError, '12000'):
            self.call('evidence', generation=self.call('status')['generation'], chunkIds=ids)

    def test_source_changed_during_rebuild_preserves_old_index(self):
        old = self.call('index')
        actual = recall.fingerprint
        calls = [0]
        def changing(path):
            calls[0] += 1
            return actual(path) if calls[0] == 1 else 'changed'
        with mock.patch.object(recall, 'fingerprint', side_effect=changing):
            with self.assertRaisesRegex(recall.RecallError, 'during rebuild'):
                self.call('index')
        self.assertEqual(self.call('status')['generation'], old['generation'])

    def test_local_client_does_not_use_environment_proxies_or_redirects(self):
        with mock.patch.object(recall.urllib.request, 'build_opener') as opener:
            recall.LocalModel()
        handlers = opener.call_args[0]
        self.assertEqual(handlers[0].proxies, {})
        self.assertIsInstance(handlers[1], recall.NoRedirect)
        with self.assertRaises(recall.RecallError):
            handlers[1].redirect_request(None, None, 302, '', {}, 'https://example.com')


if __name__ == '__main__':
    unittest.main()

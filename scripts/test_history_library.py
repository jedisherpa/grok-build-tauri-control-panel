"""Migration checks use temporary homes and never inspect real credentials."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
import history_library as h


class HistoryChecks(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home = Path(self.tmp.name) / 'home'
        self.home.mkdir()
        self.c = h.connect(Path(self.tmp.name) / 'index/library.sqlite')

    def tearDown(self):
        self.c.close()
        self.tmp.cleanup()

    def write(self, path, rows):
        p = self.home / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(''.join(json.dumps(r) + '\n' for r in rows))
        return p

    def codex(self, text='Preserve the originals'):
        return self.write('.codex/sessions/rollout-one.jsonl', [
            {'type': 'session_meta', 'payload': {'id': 'one', 'cwd': '/example'}},
            {'type': 'response_item', 'payload': {'type': 'message', 'id': 'm1',
                'role': 'user', 'content': [{'type': 'input_text', 'text': text}]}},
        ])

    def test_scan_is_idempotent_and_sources_unchanged(self):
        p = self.codex()
        original = p.read_bytes()
        first = h.scan(self.c, self.home)
        second = h.scan(self.c, self.home)
        self.assertEqual(first['receipt']['imported'], 1)
        self.assertEqual(second['receipt']['unchanged'], 1)
        self.assertEqual(p.read_bytes(), original)
        self.assertEqual(self.c.execute('SELECT digest FROM files').fetchone()[0],
                         hashlib.sha256(original).hexdigest())
        self.assertEqual(h.read(self.c, {'id': 'codex:one'})['total'], 1)

    def test_changed_sources_replace_old_messages(self):
        self.codex('first'); h.scan(self.c, self.home)
        self.codex('second replacement'); h.scan(self.c, self.home)
        data = h.read(self.c, {'id': 'codex:one'})
        self.assertEqual(data['total'], 1)
        self.assertEqual(data['messages'][0]['text'], 'second replacement')

    def test_provider_namespaces_do_not_collide(self):
        self.codex()
        self.write('.claude/projects/example/one.jsonl', [{
            'type': 'user', 'sessionId': 'one', 'uuid': 'm1',
            'message': {'content': 'Claude conversation'}}])
        h.scan(self.c, self.home)
        self.assertEqual(h.read(self.c, {'id': 'claude_code:one'})['total'], 1)
        self.assertEqual(h.read(self.c, {'id': 'codex:one'})['total'], 1)

    def test_subagent_kept_separate_from_parent(self):
        for path, text in [('one.jsonl', 'parent'), ('one/subagents/agent-a.jsonl', 'child')]:
            self.write('.claude/projects/example/' + path, [{'type': 'user',
                'sessionId': 'one', 'uuid': 'm1', 'message': {'content': text}}])
        h.scan(self.c, self.home)
        parent = h.read(self.c, {'id': 'claude_code:one'})
        child = h.read(self.c, {'id': 'claude_code:one/subagent/agent-a'})
        self.assertEqual(parent['messages'][0]['text'], 'parent')
        self.assertEqual(child['thread']['parent_id'], 'one')

    def test_malformed_records_are_reported(self):
        p = self.codex()
        with p.open('a') as f: f.write('not-json\n')
        result = h.scan(self.c, self.home)
        self.assertEqual(result['skipped_records'], 1)
        self.assertIn('skipped', h.read(self.c, {'id': 'codex:one'})['thread']['coverage'])

    def test_export_keeps_all_branches_and_import_is_idempotent(self):
        p = self.home / 'conversations.json'
        p.write_text(json.dumps([{'id': 'export-one', 'title': 'Branches', 'mapping': {
            k: {'message': {'author': {'role': 'assistant'}, 'content': {'parts': [k]},
                             'create_time': n}} for n, k in enumerate(['a', 'b', 'c'])}}]))
        h.import_export(self.c, p); h.import_export(self.c, p)
        data = h.read(self.c, {'id': 'chatgpt:export-one'})
        self.assertEqual(data['total'], 3)
        self.assertIn('all branches', data['thread']['coverage'])

    def test_zip_does_not_extract_untrusted_paths(self):
        p = self.home / 'export.zip'
        with zipfile.ZipFile(p, 'w') as z:
            z.writestr('../../escape.txt', 'untrusted')
            z.writestr('conversations.json', json.dumps([{'uuid': 'c1', 'name': 'Claude',
                'chat_messages': [{'uuid': 'm', 'sender': 'human', 'text': 'hello'}]}]))
        h.import_export(self.c, p)
        self.assertFalse((self.home.parent / 'escape.txt').exists())
        self.assertEqual(h.read(self.c, {'id': 'claude:c1'})['total'], 1)

    def test_search_treats_syntax_as_literal_tokens(self):
        self.codex('alpha beta'); h.scan(self.c, self.home)
        self.assertEqual(h.search(self.c, {'query': 'alpha "beta"'})['total'], 1)
        self.assertEqual(h.search(self.c, {'query': 'alpha OR nonexistent'})['total'], 0)
        self.assertEqual(h.search(self.c, {'query': '***'})['total'], 0)

    def test_related_threads_require_explicit_filter(self):
        h.put_thread(self.c, 'codex', 'parent', title='Parent')
        h.put_thread(self.c, 'codex', 'child', title='Child', parent_id='parent')
        h.refresh_search(self.c); self.c.commit()
        self.assertEqual(h.search(self.c, {})['total'], 1)
        self.assertEqual(h.search(self.c, {'include_subagents': True})['total'], 2)

    def test_hidden_reasoning_and_binary_content_stay_in_source(self):
        self.assertEqual(h.content_text([{'type': 'thinking', 'text': 'private'},
            {'type': 'input_image', 'text': 'binary'}, {'type': 'text', 'text': 'visible'}]).strip(), 'visible')

    def test_terminal_redaction_and_coverage(self):
        p = self.home / '.zsh_history'
        p.write_text(': 100:0;run --token abc123 API_KEY=secret123\n')
        h.scan(self.c, self.home)
        data = h.read(self.c, {'id': 'terminal:zsh-history'})
        self.assertNotIn('abc123', data['messages'][0]['text'])
        self.assertNotIn('secret123', data['messages'][0]['text'])
        self.assertIn('output unavailable', data['thread']['coverage'])

    def test_metadata_only_is_not_claimed_as_a_transcript(self):
        self.write('Library/Application Support/Claude/claude-code-sessions/one.json', [])
        p = self.home / 'Library/Application Support/Claude/claude-code-sessions/one.json'
        p.write_text(json.dumps({'cliSessionId': 'missing', 'title': 'Known title'}))
        h.scan(self.c, self.home)
        data = h.read(self.c, {'id': 'claude_code:missing'})
        self.assertEqual(data['total'], 0)
        self.assertIn('metadata only', data['thread']['coverage'])

    def test_full_reference_ignores_reader_pages_and_recovers_index_caps(self):
        long = 'begin ' + 'x' * (h.MAX_TEXT + 100) + ' final marker'
        p = self.write('.codex/sessions/rollout-one.jsonl', [
            {'type': 'session_meta', 'payload': {'id': 'one', 'cwd': str(self.home)}}
        ] + [{'type': 'response_item', 'payload': {'type': 'message', 'id': str(i),
             'role': 'user', 'content': long if i == 90 else 'message ' + str(i)}} for i in range(100)])
        original = p.read_bytes(); h.scan(self.c, self.home)
        self.assertEqual(len(h.read(self.c, {'id': 'codex:one'})['messages']), 80)
        result = h.prepare(self.c, {'id': 'codex:one'}, Path(self.tmp.name) / 'index/library.sqlite')
        full = json.loads(Path(result['json_path']).read_text())
        self.assertEqual(len(full['messages']), 100)
        self.assertEqual(full['messages'][90]['text'], long)
        self.assertEqual(result['remaining_truncated'], 0)
        self.assertEqual(result['recovered_messages'], 1)
        self.assertIn('message 99', result['recent_context'])
        self.assertEqual(p.read_bytes(), original)
        self.assertEqual(Path(result['markdown_path']).stat().st_mode & 0o777, 0o600)
        self.assertEqual(Path(result['markdown_path']).parent.stat().st_mode & 0o777, 0o700)

    def test_partial_snapshot_stays_labelled_in_complete_reference(self):
        p = self.home / 'chatgpt-accessible.json'
        p.write_text(json.dumps({'format': 'bombcode-accessible-chatgpt/v1', 'conversations': [
            {'id': 'cloud', 'messages': [{'id': 'm', 'role': 'user', 'text': 'available only', 'truncated': True}]}]}))
        h.import_export(self.c, p)
        result = h.prepare(self.c, {'id': 'chatgpt:cloud'}, Path(self.tmp.name) / 'index/library.sqlite')
        self.assertEqual(result['remaining_truncated'], 1)
        self.assertIn('remain truncated', result['notice'])
        self.assertFalse(result['native_candidate'])

    def test_native_candidate_requires_main_engine_history_and_existing_project(self):
        import uuid
        origin = str(uuid.uuid4())
        self.write('.claude/projects/example/' + origin + '.jsonl', [{
            'type': 'user', 'sessionId': origin, 'cwd': str(self.home), 'uuid': 'm',
            'message': {'content': 'main history'}}])
        self.write('.claude/projects/example/' + origin + '/subagents/agent-child.jsonl', [{
            'type': 'user', 'sessionId': origin, 'cwd': str(self.home), 'uuid': 'm',
            'message': {'content': 'child history'}}])
        h.scan(self.c, self.home)
        db = Path(self.tmp.name) / 'index/library.sqlite'
        self.assertTrue(h.prepare(self.c, {'id': 'claude_code:' + origin}, db)['native_candidate'])
        self.assertFalse(h.prepare(self.c, {'id': 'claude_code:' + origin + '/subagent/agent-child'}, db)['native_candidate'])

    def test_changed_truncated_source_requires_rescan(self):
        p = self.codex('x' * (h.MAX_TEXT + 10)); h.scan(self.c, self.home)
        p.write_text(p.read_text() + '\n')
        with self.assertRaisesRegex(ValueError, 'Source changed'):
            h.prepare(self.c, {'id': 'codex:one'}, Path(self.tmp.name) / 'index/library.sqlite')


if __name__ == '__main__':
    unittest.main()

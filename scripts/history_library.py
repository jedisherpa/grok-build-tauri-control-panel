"""Local, provenance-preserving conversation index. Never changes source histories.

Embedded by the desktop bridge; also callable for migration and fixture checks.
Imported text is a reference library, never an instruction or approval.
"""
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import sys
import zipfile
from urllib.parse import unquote

MAX_TEXT = 262144
MAX_LINE = 32 * 1024 * 1024


def connect(db):
    Path(db).parent.mkdir(parents=True, exist_ok=True)
    os.chmod(Path(db).parent, 0o700)
    c = sqlite3.connect(db, timeout=30)
    os.chmod(db, 0o600)
    c.row_factory = sqlite3.Row
    c.executescript("""
    PRAGMA journal_mode=DELETE;
    CREATE TABLE IF NOT EXISTS threads (
      id TEXT PRIMARY KEY, source TEXT NOT NULL, origin_id TEXT NOT NULL,
      title TEXT NOT NULL, cwd TEXT NOT NULL DEFAULT '', model TEXT NOT NULL DEFAULT '',
      created TEXT NOT NULL DEFAULT '', updated TEXT NOT NULL DEFAULT '',
      origin_url TEXT NOT NULL DEFAULT '', coverage TEXT NOT NULL DEFAULT 'local transcript',
      file_path TEXT NOT NULL DEFAULT '', parent_id TEXT NOT NULL DEFAULT '');
    CREATE TABLE IF NOT EXISTS messages (
      thread_id TEXT NOT NULL, message_id TEXT NOT NULL, role TEXT NOT NULL,
      text TEXT NOT NULL, at TEXT NOT NULL DEFAULT '', seq INTEGER NOT NULL,
      file_path TEXT NOT NULL DEFAULT '', byte_offset INTEGER NOT NULL DEFAULT 0,
      truncated INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(thread_id,message_id));
    CREATE INDEX IF NOT EXISTS messages_thread ON messages(thread_id,seq);
    CREATE TABLE IF NOT EXISTS files (
      path TEXT PRIMARY KEY,size INTEGER NOT NULL,mtime INTEGER NOT NULL,
      digest TEXT NOT NULL, malformed INTEGER NOT NULL,oversized INTEGER NOT NULL);
    CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(thread_id UNINDEXED,text,
      tokenize='unicode61');
    CREATE TABLE IF NOT EXISTS receipts(id INTEGER PRIMARY KEY,at TEXT,payload TEXT);
    """)
    return c


def content_text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return '\n'.join(content_text(x) for x in content if isinstance(x, (str, dict)))
    if isinstance(content, dict):
        # Never import binary image/audio payloads, hidden reasoning, or tool arguments.
        if content.get('type') in ('image', 'image_url', 'input_image', 'thinking', 'reasoning'):
            return ''
        return str(content.get('text') or content.get('input_text') or '')
    return ''


def stamp(value):
    if isinstance(value, (int, float)):
        return datetime.datetime.fromtimestamp(value, datetime.timezone.utc).isoformat()
    return str(value or '')


def put_thread(c, source, origin_id, **kw):
    tid = source + ':' + str(origin_id)
    old = c.execute('SELECT * FROM threads WHERE id=?', (tid,)).fetchone()
    vals = dict(old) if old else dict(id=tid, source=source, origin_id=str(origin_id),
        title='', cwd='', model='', created='', updated='', origin_url='',
        coverage='local transcript', file_path='', parent_id='')
    for k, v in kw.items():
        if v is not None and str(v):
            vals[k] = str(v)
    vals['title'] = vals['title'] or str(origin_id)
    keys = list(vals)
    c.execute('INSERT OR REPLACE INTO threads ('+','.join(keys)+') VALUES ('+
        ','.join('?' for _ in keys)+')', [vals[k] for k in keys])
    return tid


def put_message(c, tid, mid, role, text, at='', seq=0, path='', offset=0, truncated=False):
    text = str(text or '').strip()
    if not text:
        return
    if not mid:
        mid = hashlib.sha256((role+'\0'+text+'\0'+str(seq)).encode()).hexdigest()
    c.execute('INSERT OR REPLACE INTO messages VALUES (?,?,?,?,?,?,?,?,?)',
        (tid, str(mid), role, text[:MAX_TEXT], stamp(at), seq, path, offset,
         int(truncated or len(text) > MAX_TEXT)))
    row = c.execute('SELECT title,origin_id FROM threads WHERE id=?', (tid,)).fetchone()
    if role == 'user' and row and row['title'] == row['origin_id']:
        title = re.sub(r'\s+', ' ', text).strip()[:110]
        # Instruction wrappers are source context, not useful conversation titles.
        if not title.startswith(('# AGENTS.md', '<environment_context>', '<permissions')):
            c.execute('UPDATE threads SET title=? WHERE id=?', (title, tid))


def line_records(p):
    with p.open('rb') as f:
        seq = 0
        while True:
            offset = f.tell()
            raw = f.readline(MAX_LINE + 1)
            if not raw:
                break
            seq += 1
            if len(raw) > MAX_LINE:
                while raw and not raw.endswith(b'\n'):
                    raw = f.readline(MAX_LINE + 1)
                yield seq, offset, None, 'oversized'
                continue
            try:
                yield seq, offset, json.loads(raw), None
            except (ValueError, UnicodeError):
                yield seq, offset, None, 'malformed'


def codex_file(c, p, titles):
    origin = p.stem.split('rollout-')[-1][-36:]
    tid = put_thread(c, 'codex', origin, title=titles.get(origin), file_path=str(p))
    errors = dict(malformed=0, oversized=0)
    for seq, offset, v, error in line_records(p):
        if error:
            errors[error] += 1
            continue
        payload = v.get('payload') or {}
        at = v.get('timestamp') or ''
        if v.get('type') == 'session_meta':
            origin = payload.get('id') or payload.get('session_id') or origin
            tid = put_thread(c, 'codex', origin, title=titles.get(origin),
                cwd=payload.get('cwd'), created=stamp(payload.get('timestamp') or at),
                file_path=str(p), parent_id=payload.get('parent_thread_id'))
        elif v.get('type') == 'turn_context':
            put_thread(c, 'codex', origin, model=payload.get('model'), updated=stamp(at))
        elif v.get('type') == 'response_item' and payload.get('type') == 'message':
            role = payload.get('role')
            if role in ('user', 'assistant'):
                text = content_text(payload.get('content'))
                # Tool-only messages are deliberately kept in the original source.
                put_message(c, tid, payload.get('id') or 'line:'+str(seq), role, text,
                    at, seq, str(p), offset)
    if errors['malformed'] or errors['oversized']:
        c.execute("UPDATE threads SET coverage='local transcript; skipped records' WHERE id=?", (tid,))
    return errors


def claude_file(c, p):
    origin = p.stem
    tid = None
    errors = dict(malformed=0, oversized=0)
    for seq, offset, v, error in line_records(p):
        if error:
            errors[error] += 1
            continue
        if v.get('type') not in ('user', 'assistant', 'summary'):
            continue
        native_origin = v.get('sessionId') or origin
        origin = native_origin + '/subagent/' + p.stem if 'subagents' in p.parts else native_origin
        tid = put_thread(c, 'claude_code', origin, cwd=v.get('cwd'),
            model=(v.get('message') or {}).get('model'), updated=stamp(v.get('timestamp')),
            file_path=str(p), parent_id=native_origin if 'subagents' in p.parts else v.get('parentSessionId'))
        role = v.get('type')
        if role in ('user', 'assistant'):
            put_message(c, tid, v.get('uuid') or 'line:'+str(seq), role,
                content_text((v.get('message') or {}).get('content')), v.get('timestamp'),
                seq, str(p), offset)
    if tid and (errors['malformed'] or errors['oversized']):
        c.execute("UPDATE threads SET coverage='local transcript; skipped records' WHERE id=?", (tid,))
    return errors


def grok_file(c, p):
    summary_path = p.parent / 'summary.json'
    summary = json.loads(summary_path.read_text()) if summary_path.exists() else {}
    info = summary.get('info') or {}
    origin = p.parent.name
    tid = put_thread(c, 'grok', origin, title=summary.get('generated_title'),
        cwd=info.get('cwd') or info.get('working_directory') or unquote(p.parent.parent.name),
        model=summary.get('current_model_id'), created=stamp(summary.get('created_at')),
        updated=stamp(summary.get('updated_at')), file_path=str(p))
    errors = dict(malformed=0, oversized=0)
    for seq, offset, v, error in line_records(p):
        if error:
            errors[error] += 1
            continue
        role = v.get('role') or v.get('type')
        if role in ('user', 'assistant') and not v.get('synthetic_reason'):
            put_message(c, tid, 'line:'+str(seq), role, content_text(v.get('content')),
                v.get('timestamp'), seq, str(p), offset)
    return errors


def refresh_search(c):
    # FTS rows retain thread IDs; rebuilding is transactional and deterministic.
    c.execute('DELETE FROM search')
    c.execute('INSERT INTO search(thread_id,text) SELECT id,title || char(10) || cwd FROM threads')
    c.execute('INSERT INTO search(thread_id,text) SELECT thread_id,text FROM messages')


def terminal_file(c, p):
    origin = 'zsh-history' if p.name == '.zsh_history' else p.stem
    tid = put_thread(c, 'terminal', origin, title='Terminal · '+origin,
        file_path=str(p), coverage='saved shell commands; credential-shaped values redacted; terminal output unavailable')
    with p.open('rb') as f:
        for seq, raw in enumerate(f):
            text=raw.decode('utf-8',errors='replace').strip()
            match=re.match(r'^: (\d+):\d+;(.*)',text)
            at=stamp(int(match[1])) if match else ''
            text=match[2] if match else text
            text=re.sub(r'(?i)((?:api[_-]?key|access[_-]?token|password|secret|authorization)\s*[=:]\s*)[^\s;]+',r'\1[REDACTED]',text)
            text=re.sub(r'(?i)(--(?:token|password|api-key|secret)\s+)[^\s;]+',r'\1[REDACTED]',text)
            text=re.sub(r'\b(?:sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9_]{20,})\b','[REDACTED]',text)
            text=re.sub(r'(?i)(bearer\s+)[A-Za-z0-9_.-]+',r'\1[REDACTED]',text)
            put_message(c,tid,'line:'+str(seq),'command',text,at,seq,str(p))
    return dict(malformed=0,oversized=0)


def stats(c):
    return {'sources': [dict(r) for r in c.execute('''SELECT source,count(*) AS threads,
      sum(CASE WHEN instr(origin_id,'/subagent/')>0 THEN 1 ELSE 0 END) AS subagents,
      sum(CASE WHEN coverage LIKE '%metadata only%' THEN 1 ELSE 0 END) AS metadata_only,
      (SELECT count(*) FROM messages m JOIN threads t ON t.id=m.thread_id
       WHERE t.source=threads.source) AS messages FROM threads GROUP BY source''')],
      'files': c.execute('SELECT count(*) FROM files').fetchone()[0],
      'skipped_records': c.execute('SELECT coalesce(sum(malformed+oversized),0) FROM files').fetchone()[0],
      'last_scan': (c.execute('SELECT payload FROM receipts ORDER BY id DESC LIMIT 1').fetchone() or [''])[0]}


def scan(c, home):
    home = Path(home)
    titles = {}
    title_file = home / '.codex/session_index.jsonl'
    if title_file.exists():
        for _, _, v, err in line_records(title_file):
            if not err:
                titles[v.get('id') or v.get('session_id')] = v.get('thread_name') or v.get('title')
    files = []
    for root in (home/'.codex/sessions', home/'.codex/archived_sessions'):
        if root.exists():
            files.extend(('codex', p) for p in root.rglob('*.jsonl'))
    root = home/'.claude/projects'
    if root.exists():
        files.extend(('claude', p) for p in root.rglob('*.jsonl'))
    root = home/'.grok/sessions'
    if root.exists():
        files.extend(('grok', p) for p in root.rglob('chat_history.jsonl'))
    if (home/'.zsh_history').is_file():
        files.append(('terminal',home/'.zsh_history'))
    root=home/'.zsh_sessions'
    if root.exists():
        files.extend(('terminal',p) for p in root.glob('*.history'))
    imported, unchanged, failures = 0, 0, []
    for provider, p in sorted(files, key=lambda x: str(x[1])):
        try:
            before = p.stat()
            old = c.execute('SELECT size,mtime FROM files WHERE path=?', (str(p),)).fetchone()
            if old and tuple(old) == (before.st_size, before.st_mtime_ns):
                unchanged += 1
                continue
            c.execute('SAVEPOINT importing')
            c.execute('DELETE FROM messages WHERE file_path=?', (str(p),))
            errors = {'codex': lambda: codex_file(c,p,titles),
                      'claude': lambda: claude_file(c,p), 'grok': lambda: grok_file(c,p),
                      'terminal': lambda: terminal_file(c,p)}[provider]()
            digest = hashlib.sha256()
            with p.open('rb') as f:
                for block in iter(lambda: f.read(1024*1024), b''):
                    digest.update(block)
            after = p.stat()
            if (before.st_size,before.st_mtime_ns) != (after.st_size,after.st_mtime_ns):
                raise ValueError('source changed during scan; retry after its turn finishes')
            c.execute('INSERT OR REPLACE INTO files VALUES (?,?,?,?,?,?)',
                (str(p),after.st_size,after.st_mtime_ns,digest.hexdigest(),errors['malformed'],errors['oversized']))
            c.execute('RELEASE importing')
            c.commit()
            imported += 1
        except Exception as e:
            try:
                c.execute('ROLLBACK TO importing');c.execute('RELEASE importing')
            except sqlite3.Error:
                pass
            failures.append({'path':str(p),'error':str(e)})
    # Desktop coding-session names enrich CLI records; they are not chat transcripts.
    root = home/'Library/Application Support/Claude/claude-code-sessions'
    if root.exists():
        for p in root.rglob('*.json'):
            try:
                v = json.loads(p.read_text());origin=v.get('cliSessionId')
                if origin:
                    exists=c.execute('SELECT 1 FROM threads WHERE id=?', ('claude_code:'+origin,)).fetchone()
                    extra={} if exists else {'coverage':'desktop coding-session metadata only; transcript unavailable locally','file_path':str(p)}
                    put_thread(c,'claude_code',origin,title=v.get('title'),cwd=v.get('cwd'),**extra)
            except (OSError,ValueError):
                continue
    refresh_search(c)
    receipt = dict(at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        scanned=len(files),imported=imported,unchanged=unchanged,failures=failures,
        scope='local Codex, Claude Code, Grok transcripts and saved terminal commands; cloud chat completeness unknown')
    c.execute('INSERT INTO receipts(at,payload) VALUES (?,?)',(receipt['at'],json.dumps(receipt)))
    c.commit()
    return dict(receipt=receipt,**stats(c))


def import_export(c, path):
    p = Path(path)
    if p.suffix.lower() == '.zip':
        # Read recognized JSON entries directly. Never extract paths or attachments.
        with zipfile.ZipFile(p) as z:
            names=[i for i in z.infolist() if Path(i.filename).name in ('conversations.json','chatgpt-accessible.json')]
            if not names:
                raise ValueError('No conversations.json in this archive')
            if any(i.file_size > 512*1024*1024 for i in names):
                raise ValueError('Conversation export exceeds the 512 MiB import limit')
            values=[json.loads(z.read(i)) for i in names]
    else:
        if p.stat().st_size > 512*1024*1024:
            raise ValueError('Conversation export exceeds the 512 MiB import limit')
        values=[json.loads(p.read_text())]
    imported=0
    for value in values:
        if isinstance(value,dict) and value.get('format')=='bombcode-accessible-chatgpt/v1':
            for v in value['conversations']:
                tid=put_thread(c,'chatgpt',v['id'],title=v.get('title'),
                    origin_url='https://chatgpt.com/c/'+v['id'],file_path=str(p),
                    created=stamp(v.get('created_at')),updated=stamp(v.get('updated_at')),
                    coverage=v.get('coverage') or 'accessible account snapshot; completeness unknown')
                for n,m in enumerate(v.get('messages',[])):
                    put_message(c,tid,m.get('id') or str(n),m['role'],m['text'],
                        m.get('at'),n,str(p),truncated=m.get('truncated',False))
                imported+=1
            continue
        rows=value if isinstance(value,list) else value.get('conversations',[]) if isinstance(value,dict) else []
        if not rows:
            raise ValueError('Expected a ChatGPT or Claude conversations.json export')
        for v in rows:
            if 'mapping' in v:
                source='chatgpt';origin=v.get('conversation_id') or v.get('id')
                if not origin: continue
                tid=put_thread(c,source,origin,title=v.get('title'),file_path=str(p),
                    origin_url='https://chatgpt.com/c/'+str(origin),coverage='account export; all branches retained',
                    created=stamp(v.get('create_time')),updated=stamp(v.get('update_time')))
                # Retain every branch, not only the currently selected path.
                nodes=list(v['mapping'].items())
                nodes.sort(key=lambda x: ((x[1].get('message') or {}).get('create_time') or 0,x[0]))
                for n,(mid,node) in enumerate(nodes):
                    m=node.get('message') or {};role=(m.get('author') or {}).get('role')
                    if role in ('user','assistant'):
                        put_message(c,tid,mid,role,content_text((m.get('content') or {}).get('parts')),
                            m.get('create_time'),n,str(p))
            elif 'chat_messages' in v:
                origin=v.get('uuid') or v.get('id')
                if not origin: continue
                tid=put_thread(c,'claude',origin,title=v.get('name'),file_path=str(p),
                    origin_url='https://claude.ai/chat/'+str(origin),coverage='account export',
                    created=stamp(v.get('created_at')),updated=stamp(v.get('updated_at')))
                for n,m in enumerate(v['chat_messages']):
                    role='user' if m.get('sender')=='human' else 'assistant'
                    put_message(c,tid,m.get('uuid') or str(n),role,
                        m.get('text') or content_text(m.get('content')),m.get('created_at'),n,str(p))
            else:
                continue
            imported+=1
    if not imported:
        raise ValueError('No recognized conversations found')
    refresh_search(c);c.commit()
    return dict(imported=imported,**stats(c))


def search(c, payload):
    query=payload.get('query','').strip()
    source=payload.get('source','')
    params=[];conditions=[]
    if not payload.get('include_subagents',False):
        conditions.append("instr(t.origin_id,'/subagent/')=0")
    if source:
        conditions.append('t.source=?');params.append(source)
    if query:
        terms=re.findall(r'\w+',query,flags=re.UNICODE)
        if not terms: return {'threads':[],'total':0}
        # Quote each token rather than interpreting user input as FTS syntax.
        conditions.append('t.id IN (SELECT thread_id FROM search WHERE search MATCH ?)')
        params.append(' AND '.join('"'+x+'"' for x in terms))
    where=' WHERE '+' AND '.join(conditions) if conditions else ''
    total=c.execute('SELECT count(*) FROM threads t'+where,params).fetchone()[0]
    offset=max(0,int(payload.get('offset',0)))
    rows=c.execute('''SELECT t.*,(SELECT count(*) FROM messages m WHERE m.thread_id=t.id) AS message_count
        FROM threads t'''+where+' ORDER BY t.updated DESC,t.title LIMIT 100 OFFSET ?',params+[offset])
    return {'threads':[dict(r) for r in rows],'total':total,'offset':offset}


def read(c, payload):
    tid=payload['id'];row=c.execute('SELECT * FROM threads WHERE id=?',(tid,)).fetchone()
    if not row: raise ValueError('Conversation not found')
    offset=max(0,int(payload.get('offset',0)))
    rows=c.execute('SELECT * FROM messages WHERE thread_id=? ORDER BY at,seq,message_id LIMIT 80 OFFSET ?', (tid,offset))
    thread=dict(row);thread['source_available']=bool(thread['file_path'] and Path(thread['file_path']).exists())
    total=c.execute('SELECT count(*) FROM messages WHERE thread_id=?',(tid,)).fetchone()[0]
    return {'thread':thread,'messages':[dict(r) for r in rows],'total':total,'offset':offset}


def main():
    action,db=sys.argv[1:3]
    payload=json.loads(sys.argv[3]) if len(sys.argv)>3 else {}
    c=connect(db)
    try:
        if action=='scan': result=scan(c,payload['home'])
        elif action=='import': result=import_export(c,payload['path'])
        elif action=='search': result=search(c,payload)
        elif action=='read': result=read(c,payload)
        elif action=='stats': result=stats(c)
        else: raise ValueError('Unknown history action')
        print(json.dumps(result,ensure_ascii=False))
    finally:
        c.close()


if __name__=='__main__':
    try:
        main()
    except Exception as e:
        print(json.dumps({'error':str(e)}));sys.exit(1)

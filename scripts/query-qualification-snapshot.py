#!/usr/bin/env python3
"""Read-only metadata evidence from the isolated query qualification namespace."""
import json
import pathlib
import sqlite3
import sys

path = pathlib.Path(sys.argv[1]).resolve()
if path.name != 'gitjuke.sqlite3' or path.parent.name != 'dev.hmrguez.gitjuke.queryqualification':
    raise SystemExit('Refusing non-qualification database')
conn = sqlite3.connect(path.as_uri() + '?mode=ro', uri=True)
conn.execute('PRAGMA query_only=ON')
conn.execute('BEGIN')
result = {
    'schema': conn.execute('PRAGMA user_version').fetchone()[0],
    'queries': [dict(zip(['id', 'name', 'search', 'version', 'limited'], row)) for row in conn.execute('SELECT id,name,search,version,limited FROM saved_query ORDER BY id')],
    'settings': json.loads(conn.execute('SELECT json FROM settings').fetchone()[0]),
    'identity': conn.execute('SELECT host,user_id,login FROM identity').fetchall(),
    'query_pages': [],
    'selection': conn.execute("SELECT key,value FROM progress WHERE key='selection'").fetchall(),
    'selected_pr_cached': conn.execute("SELECT key,json_extract(json,'$.title') FROM content WHERE key='pr:a/b:101'").fetchall(),
    'watch_reasons': conn.execute('SELECT id,source FROM watch_source ORDER BY id,source').fetchall(),
    'pending_publication': conn.execute('SELECT COUNT(*) FROM publication').fetchone()[0],
}
for key, value in conn.execute("SELECT key,json FROM content WHERE key LIKE 'query:%' ORDER BY key"):
    page = json.loads(value)
    result['query_pages'].append({
        'key': key, 'items': len(page['items']),
        'first': page['items'][0]['number'] if page['items'] else None,
        'last': page['items'][-1]['number'] if page['items'] else None,
        'total': page['total'], 'run': page['run'],
        'generation': page['generation'], 'source_complete': page['sourceComplete'],
    })
conn.rollback()
print(json.dumps(result, indent=2))

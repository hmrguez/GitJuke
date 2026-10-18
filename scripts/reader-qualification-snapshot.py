#!/usr/bin/env python3
"""Read-only state evidence for the explicitly isolated local qualification app."""
import json
import pathlib
import sqlite3
import sys

path = pathlib.Path(sys.argv[1])
if path.name != 'gitjuke.sqlite3' or path.parent.name not in ('dev.hmrguez.gitjuke.qualification', 'dev.hmrguez.gitjuke.qualification.accessibility'):
    raise SystemExit('Only the isolated qualification database is accepted.')
connection = sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True)
connection.execute('PRAGMA query_only=ON')
print(json.dumps({
    'step': sys.argv[2],
    'identity': connection.execute('SELECT user_id,login FROM identity').fetchall(),
    'seen': dict(connection.execute('SELECT source_id,seen FROM seen ORDER BY source_id')),
    'generation': connection.execute("SELECT value FROM meta WHERE key='generation'").fetchone()[0],
    'revision': connection.execute("SELECT value FROM meta WHERE key='revision'").fetchone()[0],
    'sources': connection.execute("SELECT key,json_extract(json,'$.source'),json_extract(json,'$.messages[0].id'),json_extract(json,'$.messages[0].state'),json_extract(json,'$.messages[0].bytes'),json_extract(json,'$.messages[0].digest'),json_extract(json,'$.destinationAnchor.line') FROM content WHERE key LIKE '%/gitjuke-conversation/%' ORDER BY key").fetchall(),
}))

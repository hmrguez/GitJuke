#!/usr/bin/env python3
"""Read only the isolated local log fixture DB; report metadata, never bodies."""
import hashlib, json, pathlib, shutil, sqlite3, sys, tempfile
root = pathlib.Path(sys.argv[1])
if not root.is_absolute() or root.name != 'dev.hmrguez.gitjuke.logqualification':
    raise SystemExit('Only the isolated log qualification namespace is accepted.')
files = sorted(root.glob('gitjuke.sqlite3*'))
if not any(p.name == 'gitjuke.sqlite3' for p in files):
    raise SystemExit('Qualification database missing.')
canary = b'LOCAL SYNTHETIC ACTIONS LOG'
images = {p.name: p.read_bytes() for p in files}
with tempfile.TemporaryDirectory(prefix='gitjuke-log-snapshot-') as directory:
    for name, data in images.items():
        pathlib.Path(directory, name).write_bytes(data)
    # The copied WAL/SHM form a consistent SQLite snapshot after app Quit. Original
    # files are never opened by SQLite, locked, checkpointed or written here.
    conn = sqlite3.connect('file:' + str(pathlib.Path(directory, 'gitjuke.sqlite3')) + '?mode=ro', uri=True)
    conn.execute('PRAGMA query_only=ON')
    result = {'schema': conn.execute('PRAGMA user_version').fetchone()[0],
              'log_pending_jobs': conn.execute("SELECT count(*) FROM pending WHERE job LIKE '%Logs%'").fetchone()[0],
              'integrity': conn.execute('PRAGMA quick_check').fetchone()[0],
              'files': [{'file': name, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(), 'log_body_canary_present': canary in data} for name, data in images.items()],
              'inspection': 'Read-only copied SQLite main/WAL/SHM after native Quit; original files never opened by SQLite.'}
    conn.close()
if any(p.read_bytes() != images[p.name] for p in files):
    raise SystemExit('Source changed during snapshot; stop the qualification app and retry.')
if result['log_pending_jobs'] or result['integrity'] != 'ok' or any(p['log_body_canary_present'] for p in result['files']):
    raise SystemExit('Qualification storage invariant failed.')
print(json.dumps(result, indent=2))

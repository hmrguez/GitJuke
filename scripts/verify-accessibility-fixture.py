#!/usr/bin/env python3
"""Validate generated local fixture contracts; does not interact with native UI."""
import json
import pathlib
import sys

checks = 0
def check(condition):
    global checks
    assert condition
    checks += 1

root = pathlib.Path(sys.argv[1])
s = json.loads((root / 'scenario.json').read_text())
d = s['destinations']
body = d['local-unicode-limit']['node']['body']
check(len(body.encode()) == 131072)
check(body[32767] == '😀')
check(len(d['local-oversized']['node']['body'].encode()) == 131073)
check(len(d) == 13)
check(s['conversation_count'] == 55 and s['files_count'] == 22)
check(len(s['patch'].splitlines()) == 403)
check(s['patch_additions'] == s['patch_deletions'] == 201)
left = d['local-left']['coordinates']
check((left['side'], left['start_side'], left['start_line'], left['line']) == ('LEFT', 'LEFT', 199, 201))
check(left['commit_id'] == 'b' * 40)
check(d['local-negative-line']['coordinates']['line'] == -1)
check(d['local-outdated']['coordinates']['position'] is None)
check(d['local-mixed-side']['coordinates']['start_side'] == 'LEFT')
check(d['local-wrong-commit']['coordinates']['commit_id'] != s['head'])
check(d['local-wrong-file']['coordinates']['path'] != 'file-0.txt')
print(json.dumps({'checks': checks, 'profile': 'accessibility', 'records': len(d), 'utf8Bytes': len(body.encode()), 'patchRows': 403, 'live': False}))

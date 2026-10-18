#!/usr/bin/env python3
"""Check captured local native evidence. This verifies observations, not a fresh UI run."""
import collections
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
local_ids = {f'github.com:a/b:1:local-{name}' for name in ('short', 'segments', 'anchor', 'oversized', 'empty', 'neighbor')}
expected = {key: 0 for key in local_ids}
checks = 0
for file, additions in (
    ('initial.json', []), ('segment-1.json', []), ('segment-2.json', []),
    ('repeated-segments.json', []), ('segment-3.json', ['segments']),
    ('short.json', ['short']), ('oversized.json', []),
    ('empty-review.json', ['empty']), ('current-anchor.json', ['anchor']),
    ('before-restart.json', []), ('after-restart.json', []),
):
    for name in additions:
        expected[f'github.com:a/b:1:local-{name}'] = 1
    snapshot = json.loads((root / file).read_text())
    assert {key: value for key, value in snapshot['seen'].items() if key in local_ids} == expected, file
    assert snapshot['identity'] == [[42, 'reader']], file
    checks += 1

states = {
    'native-segment-1-ax.txt': ['segment 1/3', 'Characters 1–32768'],
    'native-segment-2-ax.txt': ['segment 2/3', 'Characters 32769–65536'],
    'native-segment-3-ax.txt': ['segment 3/3', 'Characters 65537–70571', 'Seen comment · LOCAL: Three explicit body segments'],
    'native-oversized-ax.txt': ['Placeholder remains unseen', 'Unseen comment · LOCAL: Oversized'],
    'native-empty-review-ax.txt': ['0 UTF-8 bytes', 'metadata-only event is complete'],
    'native-current-anchor-ax.txt': ['Attached · right line 201', 'Page 2 of 2', 'LOCAL FIXTURE LINE 201'],
    'native-row-page-return-ax.txt': ['Page 2 of 2', 'The focused UI element is'],
    'native-restart-activity-ax.txt': ['Unseen comment · LOCAL: Oversized', 'Unseen comment · LOCAL: Unopened neighbor'],
}
for file, fragments in states.items():
    text = (root / file).read_text()
    assert all(fragment in text for fragment in fragments), file
    checks += 1

reads = [json.loads(line) for line in (root / 'native-gh-reads.jsonl').read_text().splitlines()]
queries = collections.Counter(read['query'] for read in reads if read['query'])
assert queries['query Conversation'] == 0, 'Exact destinations must not scan source histories'
coordinates = [read for read in reads if 'repos/a/b/pulls/comments/103' in read['args']]
assert len(coordinates) == 1, 'Expected one exact current-anchor REST point lookup'
assert all('first' not in read['variables'] and 'after' not in read['variables']
           for read in reads if read['query'] == 'query ConversationDestination')
checks += 3
print(json.dumps({'result': 'pass', 'observationChecks': checks, 'scope': 'captured native LOCAL SYNTHETIC fixture evidence',
                  'seenLocalRecords': sorted(key for key, value in expected.items() if value),
                  'unseenLocalRecords': sorted(key for key, value in expected.items() if not value),
                  'currentAnchorPointReads': len(coordinates), 'sourceHistoryScans': queries['query Conversation']}))

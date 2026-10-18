#!/usr/bin/env python3
"""Verify captured synthetic-native observations; does not drive or rerun the UI."""
import json
import pathlib
import re
import struct
import sys

root = pathlib.Path(sys.argv[1])
checks = 0

def check(value, label):
    global checks
    if not value:
        raise AssertionError(label)
    checks += 1

def read(name):
    return (root / name).read_text()

def seen(name):
    values = json.loads(read(name + '.json'))['seen']
    return {k.rsplit(':', 1)[-1]: v for k, v in values.items() if ':local-' in k}

initial = seen('initial')
check(len(initial) == 13 and not any(initial.values()), 'thirteen initially unseen exact identities')
for name in ['unicode-segment-1', 'unicode-segment-2', 'unicode-repeated']:
    check(seen(name) == initial, name + ' does not complete a receipt')
    sources = json.loads(read(name + '.json'))['sources']
    check(any(s[2:5] == ['local-unicode-limit', 'complete', 131072] and s[5] for s in sources), name + ' has complete inclusive body metadata')
complete = dict(initial, **{'local-unicode-limit': 1})
check(seen('unicode-complete') == complete, 'only the completed Unicode identity becomes seen')
check(seen('oversized') == complete, 'one-over body and all neighbors stay unseen')
check('😀é' in read('native-unicode-segment-2-ax.txt'), 'native crossing emoji is intact')
check('Characters 65536–81920' in read('native-unicode-segment-3-ax.txt'), 'native final segment is complete')
check(seen('left-body') == dict(complete, **{'local-left': 1}), 'exact LEFT source receipt is isolated')
expected = dict(initial, **{k: 1 for k in ['local-unicode-limit', 'local-left', 'local-wrong-commit', 'local-mixed-side', 'local-negative-line', 'local-outdated', 'local-wrong-file']})
for name in ['after-discussion-browse', 'before-final-restart', 'final-restart', 'final']:
    check(seen(name) == expected, name + ' preserves exact seen identities')
check(json.loads(read('before-final-restart.json'))['seen'] == json.loads(read('final-restart.json'))['seen'], 'all captured identities survive final restart')
for name, label in [('wrong-commit', 'Recorded commit does not match'), ('mixed-side', 'Ambiguous multi-side range'), ('negative-line', 'File-level or unavailable line'), ('outdated', 'Outdated · original'), ('wrong-file', 'Different file · anchor unavailable')]:
    ax = read('native-' + name + '-ax.txt')
    check(label in ax and '403 rows' in ax and 'button Return to diff' not in ax, name + ' fails safely against a real complete patch')
for name, label in [('left-attached', 'Rows 1–200 / 403'), ('patch-page-2', 'Rows 201–400 / 403'), ('left-return-page-3', 'Rows 401–403 / 403')]:
    ax = read('native-' + name + '-ax.txt')
    check(label in ax, name + ' supplied semantic range')
check('Attached · left line 201' in read('native-left-return-page-3-ax.txt'), 'current LEFT range attaches on its exact side')
for name in ['left-return-page-3', 'patch-page-2', 'patch-page-1', 'continuous-end', 'changes-200-minimum']:
    check(re.search(r'The focused UI element is .*container file-0.txt unified diff', read('native-' + name + '-ax.txt')), name + ' keeps stable diff focus')
check('403 / 403' in read('native-continuous-end-ax.txt'), 'continuous mode reaches the terminal supplied row')
check('Text size 200%' in read('native-changes-200-minimum-ax.txt'), 'native enlarged continuous reading')
for name, count in [('comments-first', 50), ('comments-last', 5), ('reviews-first', 50), ('reviews-last', 5), ('threads-first', 20), ('threads-page-2', 20), ('threads-page-3', 15), ('replies-first', 50), ('replies-last', 5)]:
    check(f'{count} loaded / 55 known' in read('native-' + name + '-ax.txt'), name + ' bounded native page')
check('button (disabled) Next replies' in read('native-replies-last-ax.txt'), 'terminal replies stop')
check(re.search(r'The focused UI element is .*button First page', read('native-comment-keyboard-first-control-ax.txt')), 'keyboard reaches First page')
check(re.search(r'The focused UI element is .*container Conversation', read('native-comments-keyboard-first-ax.txt')), 'keyboard First page restores the panel')
check(re.search(r'The focused UI element is .*button file-40.txt', read('native-thread-origin-return-ax.txt')), 'reply return restores the exact third-page thread')
check('CONTENT – 960 – 640' in read('native-content-measurement-ax.txt'), 'actual undocked native content viewport')
check('\\"root\\":\\"12px\\",\\"body\\":\\"13px\\"' in read('native-font-100-ax.txt'), 'native baseline computed fonts')
check('font-size: 26px;' in read('native-inspector-200-ax.txt'), 'actual body typography doubles to 26px')
check('\\"reduced\\":true' in read('native-font-100-ax.txt') and 'transition-duration: 0s;' in read('native-inspector-200-ax.txt'), 'system reduced motion is honored natively')
check(re.search(r'The focused UI element is .*button Preferences', read('native-preferences-escape-return-ax.txt')), 'Escape returns to the Preferences opener')
check('Reader text size, Value: 200%' in read('native-preferences-appearance-200-ax.txt'), 'native Preferences exposes actual enlarged size')
for name in ['monitoring', 'github', 'storage']:
    check(f'toggle button {name.capitalize() if name != "github" else "GitHub"}, Value: on' in read('native-preferences-' + name + '-200-ax.txt'), name + ' Preferences section is reachable')
check(re.search(r'The focused UI element is .*container Activity source body', read('native-activity-source-body-focus-ax.txt')), 'Activity source has a named keyboard reading target')
check(re.search(r'The focused UI element is .*Current LEFT range', read('native-final-browse-back-origin-ax.txt')), 'final Browse/Back restores the exact Activity origin')
check('Value: off' in read('system-motion-restored.txt') and 'Value: off' in read('voiceover-final-restored.txt'), 'temporary system settings restored')
check('Value: on' in read('voiceover-final-attempt.txt') and not read('voiceover-process.txt').strip(), 'VoiceOver attempt is recorded without a qualification claim')

def jpeg_size(path):
    data = path.read_bytes()
    check(data[:2] == b'\xff\xd8', path.name + ' is a captured JPEG')
    at = 2
    while at < len(data):
        if data[at] != 255:
            at += 1
            continue
        marker = data[at + 1]
        at += 2
        if marker in (0xD8, 0xD9):
            continue
        length = struct.unpack('>H', data[at:at + 2])[0]
        if marker in (0xC0, 0xC1, 0xC2):
            height, width = struct.unpack('>HH', data[at + 3:at + 7])
            return width, height
        at += length
    raise AssertionError('JPEG dimensions unavailable')

for name in ['native-final-minimum-before.jpg', 'native-final-minimum-after.jpg']:
    check(jpeg_size(root / name) == (960, 672), name + ' preserves the content floor with title chrome')
print(json.dumps({'observationChecks': checks, 'freshUiRerun': False, 'source': 'captured synthetic-local native WebKit/IPC/SQLite', 'voiceOver': 'unqualified; process did not start during enabled attempt', 'performance': 'not qualified', 'releaseScenariosPromoted': 0}, indent=2))

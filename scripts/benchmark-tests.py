"""Compare identical test workloads; run from any directory with Python 3."""
import argparse
import json
import platform
import statistics
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--js-runs', type=int, default=30)
parser.add_argument('--rust-runs', type=int, default=1)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
if args.js_runs < 1 or args.rust_runs < 0:
    parser.error('js-runs must be positive; rust-runs must be nonnegative')
args.output.mkdir(parents=True, exist_ok=True)


def run(command, log):
    start = time.perf_counter()
    with log.open('w') as output:
        result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
    elapsed = time.perf_counter() - start
    if result.returncode:
        raise SystemExit(f'{command} failed ({result.returncode}); see {log}')
    return elapsed


results = {
    'platform': platform.platform(),
    'versions': {tool: subprocess.check_output([tool, '--version'], text=True).strip()
                 for tool in ['bun', 'node', 'npm', 'cargo']},
    'method': 'Sequential wall time including startup; alternating command order; JS has one warmup per command. Rust runs the fast suite with test-result caching disabled, using existing Cargo artifacts and no extra warmup. All commands must pass.',
    'suites': {},
}
for suite, count, commands in [
    ('javascript', args.js_runs, {'npm_node': ['npm', 'run', 'test:diff:node'],
                                'bun': ['bun', 'test'],
                                'node': ['node', '--experimental-strip-types', '--test', 'scripts/diff.test.mjs']}),
    ('rust', args.rust_runs, {'npm': ['npm', 'test', '--', '--no-cache'],
                            'bun': ['bun', 'run', 'test:rust', '--no-cache']}),
]:
    if not count:
        continue
    samples = {name: [] for name in commands}
    if suite == 'javascript':
        for name, command in commands.items():
            run(command, args.output / f'{suite}-{name}-warmup.log')
    for iteration in range(count):
        order = list(commands) if iteration % 2 == 0 else list(reversed(commands))
        for name in order:
            elapsed = run(commands[name], args.output / f'{suite}-{name}.log')
            samples[name].append(elapsed)
            print(f'{suite} {name} {iteration + 1}/{count}: {elapsed:.3f}s', flush=True)
    results['suites'][suite] = {
        name: {'command': commands[name], 'seconds': values,
               'median_seconds': statistics.median(values),
               'min_seconds': min(values), 'max_seconds': max(values)}
        for name, values in samples.items()
    }
    (args.output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')

"""Checks source metadata across cold, warm, and edited incremental builds."""

import json
import hashlib
import copy
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

from importlib.util import module_from_spec, spec_from_file_location

spec = spec_from_file_location('source_cfg_mir', Path(__file__).with_name('source-cfg-mir.py'))
mir_check = module_from_spec(spec)
spec.loader.exec_module(mir_check)
root = Path(__file__).resolve().parent.parent
work = Path(sys.argv[1]).resolve()
sysroot = sys.argv[2]
work.mkdir()
channel = re.search(r'channel\s*=\s*"([^"]+)"', (root / 'rust-toolchain.toml').read_text())[1]
suffix = 'dylib' if platform.system() == 'Darwin' else 'so'
producer = hashlib.sha256((root / f'target/release/librustc_codegen_mires.{suffix}').read_bytes()).hexdigest()
base = [
    'rustc', '+' + channel,
    '-Zcodegen-backend=' + str(root / f'target/release/librustc_codegen_mires.{suffix}'),
    '-Cllvm-args=js-producer=' + producer,
    '--edition', '2021', '--target', 'wasm32-unknown-unknown', '--sysroot', sysroot,
    '-Ccodegen-units=1', '-Cpanic=abort', '-Coverflow-checks=off',
    '-Cdebug-assertions=off', '-Cdebuginfo=0', '--remap-path-prefix', str(root) + '/=',
    '-Cllvm-args=js-opt=off js-minify=on js-ecma-target=2018 js-lane=jir',
]
provider = work / 'suspension_provider.rs'
original = (root / 'examples/cross-crate-tests/suspension_provider.rs').read_text()
provider.write_text(original)
liveness_source = work / '111_async_source_liveness.rs'
liveness_original = (root / 'examples/core-tests/111_async_source_liveness.rs').read_text()
liveness_source.write_text(liveness_original)
for name in ['prelude.rs', '111_async_source_liveness.expected']:
    shutil.copy2(root / 'examples/core-tests' / name, work / name)
initialization_source = work / '112_async_initialization.rs'
initialization_original = (root / 'examples/core-tests/112_async_initialization.rs').read_text()
initialization_source.write_text(initialization_original)
initialization_expected = (root / 'examples/core-tests/112_async_initialization.expected').read_text()
initialization_source.with_suffix('.expected').write_text(initialization_expected)
rows = []
rejected_liveness_controls = 0
rejected_frame_controls = 0
rejected_initialization_controls = 0
for phase in ['cold', 'warm', 'edited']:
    if phase == 'edited':
        provider.write_text(original.replace(
            '    core::future::ready(()).await;',
            '    core::future::ready(()).await;\n    core::future::ready(()).await;', 1))
        liveness_source.write_text(liveness_original.replace('let view = &retained;', 'let view = retained;'))
        initialization_source.write_text(initialization_original.replace('if flag { conditional = Guard(92); }', 'if flag { conditional = Guard(92); } else { conditional = Guard(93); }'))
        initialization_source.with_suffix('.expected').write_text(initialization_expected + '93\n')
    for name in ['provider', 'generic', 'local', 'liveness', 'initialization']:
        library = name == 'provider'
        source_file = provider if library else initialization_source if name == 'initialization' else liveness_source if name == 'liveness' else root / {
            'generic': 'examples/cross-crate-tests/suspension_client.rs',
            'local': 'examples/core-tests/26_async.rs',
            'liveness': 'examples/core-tests/111_async_source_liveness.rs',
        }[name]
        output = work / ('libsuspension_provider.rlib' if library else name + '.js')
        crate_name = {'provider': 'suspension_provider', 'generic': 'suspension_client', 'local': 'async_local', 'liveness': 'async_liveness', 'initialization': 'async_initialization'}[name]
        command = base + [
            '--crate-type', 'rlib' if library else 'cdylib', '--crate-name', crate_name,
            '-Cincremental=' + str(work / (name + '-cache')), '-o', str(output), str(source_file),
        ]
        if phase != 'warm':
            command += ['-Zdump-mir=analysis|runtime', '-Zdump-mir-dir=' + str(work / (phase + '-mir'))]
        if name == 'generic':
            command += ['--extern', 'suspension_provider=' + str(work / 'libsuspension_provider.rlib')]
        result = subprocess.run(command, cwd=work, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        log = work / f'{phase}-{name}.log'
        log.write_bytes(result.stdout)
        assert result.returncode == 0, log
        if library:
            shutil.copy2(output, work / f'{phase}-provider.rlib')
            continue
        report = json.loads((work / f'{name}.js.lane.json').read_text())
        expected = ({'initialization': 1} if name == 'initialization' else {'immediate': 0, 'one_await': 1, 'two_awaits': 2, 'holds_a_struct': 1}
                    if name == 'local' else {'borrowed': 1, 'moved_guard': 1, 'held_guard': 1, 'two_states': 2}
                    if name == 'liveness' else {'immediate': 0, 'relay': 2 if phase == 'edited' else 1, 'repeated': 1})
        observed = {}
        for graph in report['async']['graphs']:
            key = next(key for key in expected if '$' + key + '$' in graph['item'])
            source = graph['source_suspensions']
            assert source['status'] == 'captured', (phase, graph)
            assert source['yield_count'] == expected[key], (phase, graph)
            assert all(point['liveness'] is not None for point in source['yields'])
            observed.setdefault(key, []).append(source['yield_count'])
            assert not graph['selected']
        assert set(observed) == set(expected)
        if phase != 'warm':
            mir_rows = mir_check.check(work / (phase + '-mir'), work / f'{name}.js.lane.json')
            (work / f'{phase}-{name}-mir.json').write_text(json.dumps(mir_rows, indent=2) + '\n')
        if phase == 'cold' and name == 'liveness':
            for field in ['live', 'borrowed']:
                wrong = copy.deepcopy(report)
                graph = next(graph for graph in wrong['async']['graphs'] if '$borrowed$' in graph['item'])
                graph['source_suspensions']['yields'][0]['liveness'][field] = []
                path = work / f'omitted-{field}.lane.json'
                path.write_text(json.dumps(wrong, indent=2) + '\n')
                try:
                    mir_check.check(work / 'cold-mir', path)
                except AssertionError:
                    rejected_liveness_controls += 1
                else:
                    raise AssertionError(f'MIR checker accepted missing {field} locals')
        if phase == 'cold' and name == 'liveness':
            for field in ['saved_local_sources', 'states']:
                wrong = copy.deepcopy(report)
                graph = next(graph for graph in wrong['async']['graphs'] if '$borrowed$' in graph['item'])
                frame = graph['source_suspensions']['runtime_frame']
                if field == 'saved_local_sources':
                    frame[field][0], frame[field][1] = frame[field][1], frame[field][0]
                else:
                    frame[field][3] = []
                path = work / f'wrong-frame-{field}.lane.json'
                path.write_text(json.dumps(wrong, indent=2) + '\n')
                try:
                    mir_check.check(work / 'cold-mir', path)
                except AssertionError:
                    rejected_frame_controls += 1
                else:
                    raise AssertionError(f'MIR checker accepted wrong frame {field}')
        if phase == 'cold' and name == 'initialization':
            probes = mir_rows[0]['initialization_probes']
            for label, field in [('conditional-14', 'maybe_uninitialized'), ('first-field-moved', 'maybe_initialized'), ('call-after-argument', 'maybe_initialized'), ('call-after-destination', 'maybe_initialized')]:
                wrong = copy.deepcopy(report)
                probe = next(probe for probe in probes if probe['label'] == label)
                state = wrong['async']['graphs'][0]['source_suspensions']['initialization']['points'][probe['block']][probe['when']]
                values = set(state[field])
                values.symmetric_difference_update([probe['path']])
                state[field] = sorted(values)
                path = work / f'wrong-init-{label}.lane.json'
                path.write_text(json.dumps(wrong, indent=2) + '\n')
                try:
                    mir_check.check(work / 'cold-mir', path)
                except AssertionError:
                    rejected_initialization_controls += 1
                else:
                    raise AssertionError(f'MIR checker accepted wrong initialization {label}')
        bundle = (root / 'runtime/shim.js').read_bytes() + output.read_bytes() + (root / 'scripts/entry.js').read_bytes()
        result = subprocess.run(['node', '-'], input=bundle, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        assert result.returncode == 0, result.stderr
        expected_stdout = (b'7\ngeneric\n9\n11\n' if name == 'generic'
                           else source_file.with_suffix('.expected').read_bytes())
        assert result.stdout == expected_stdout
        for extension in ['', '.lane.json', '.artifacts.json']:
            shutil.copy2(work / f'{name}.js{extension}', work / f'{phase}-{name}.js{extension}')
        rows.append(dict(phase=phase, fixture=name, observed=observed, fallback=report['fallback_count']))
        assert report['fallback_count'] == 0
for name in ['generic', 'local', 'liveness', 'initialization']:
    assert (work / f'cold-{name}.js').read_bytes() == (work / f'warm-{name}.js').read_bytes()
    cold = json.loads((work / f'cold-{name}.js.lane.json').read_text())['async']
    warm = json.loads((work / f'warm-{name}.js.lane.json').read_text())['async']
    assert cold == warm
cold_live = json.loads((work / 'cold-liveness.js.lane.json').read_text())['async']['graphs']
edited_live = json.loads((work / 'edited-liveness.js.lane.json').read_text())['async']['graphs']
cold_borrow = next(graph for graph in cold_live if '$borrowed$' in graph['item'])['source_suspensions']
edited_borrow = next(graph for graph in edited_live if '$borrowed$' in graph['item'])['source_suspensions']
assert cold_borrow['yield_count'] == edited_borrow['yield_count'] == 1
assert cold_borrow['yields'][0]['liveness'] != edited_borrow['yields'][0]['liveness']
assert cold_borrow['runtime_frame'] != edited_borrow['runtime_frame']
cold_init = json.loads((work / 'cold-initialization.js.lane.json').read_text())['async']['graphs'][0]['source_suspensions']
edited_init = json.loads((work / 'edited-initialization.js.lane.json').read_text())['async']['graphs'][0]['source_suspensions']
assert cold_init['yield_count'] == edited_init['yield_count'] == 1
assert cold_init['initialization'] != edited_init['initialization']
result = dict(status='pass', rows=rows, coldWarmJavaScriptIdentical=True,
              coldWarmSourceReportsIdentical=True, editedProviderObserved=True,
              sameYieldLivenessEditObserved=True,
              rejectedLivenessControls=rejected_liveness_controls,
              rejectedFrameControls=rejected_frame_controls,
              rejectedInitializationControls=rejected_initialization_controls,
              sameYieldInitializationEditObserved=True)
assert rejected_liveness_controls == 2
assert rejected_frame_controls == 2
assert rejected_initialization_controls == 4
(work / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
print('incremental suspension facts: PASS (cold, warm, and edited generic metadata)')

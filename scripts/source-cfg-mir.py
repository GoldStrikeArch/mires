"""Compare captured coroutine control flow with pinned rustc's independent analysis MIR dump."""

import json
from pathlib import Path
import re
import sys


def blocks_of(path):
    text = path.read_text()
    assert 'after analysis\n' in text, path
    blocks = []
    for match in re.finditer(r'^    bb(\d+)(?: \(cleanup\))?: \{\n(.*?)^    \}', text, re.M | re.S):
        assert int(match[1]) == len(blocks), path
        term = match[2].strip().splitlines()[-1].strip()
        if term.startswith('goto '):
            kind = 'Goto'
        elif term.startswith('switchInt('):
            kind = 'Switch'
        elif term.startswith('falseEdge '):
            kind = 'FalseEdge'
        elif term.startswith('falseUnwind '):
            kind = 'FalseUnwind'
        elif term.startswith('drop('):
            kind = 'Drop'
        elif term.startswith('assert('):
            kind = 'Assert'
        elif '= yield(' in term:
            kind = 'Yield'
        elif term == 'return;':
            kind = 'Return'
        elif term == 'resume;':
            kind = 'UnwindResume'
        elif term == 'unreachable;':
            kind = 'Unreachable'
        elif term == 'coroutine_drop;':
            kind = 'CoroutineDrop'
        elif term.startswith('terminate('):
            kind = 'UnwindTerminate'
        else:
            assert ' -> ' in term and '(' in term, (path, term)
            kind = 'Call'
        edges = []
        if ' -> ' in term:
            targets = term.rsplit(' -> ', 1)[1]
            for label, target in re.findall(r'(?:(\w+): )?bb(\d+)', targets):
                edge = {'resume': 'Resume', 'drop': 'Cancel', 'imaginary': 'Imaginary',
                        'unwind': 'Imaginary' if kind == 'FalseUnwind' else 'Unwind'}.get(label, 'Normal')
                edges.append(dict(kind=edge, target=int(target)))
        drop_local = re.match(r'drop\(_(\d+)\)', term) if kind == 'Drop' else None
        blocks.append(dict(kind=kind, successors=edges,
                           drop_local=int(drop_local[1]) if drop_local else None))
    assert blocks, path
    return blocks


def check_frame(path, runtime, frame):
    """Use rustc's textual saved-field and debug-place records, independent of the producer API."""
    text = runtime.read_text()
    assert 'after runtime\n' in text
    field_ids = [int(index) for index in re.findall(r'^        field _s(\d+):', text, re.M)]
    assert field_ids == list(range(len(field_ids)))
    states = []
    for index, fields in re.findall(r'^            \w+\s*\((\d+)\): \[(.*?)\],', text, re.M):
        assert int(index) == len(states)
        states.append([int(field) for field in re.findall(r'_s(\d+)', fields)])
    before = re.findall(r'^\s+debug (.+?) => (.*?);$', path.read_text(), re.M)
    after = re.findall(r'^\s+debug (.+?) => (.*?);$', text, re.M)
    assert len(before) == len(after)
    sources = [[] for _ in field_ids]
    for (name, original), (other, place) in zip(before, after):
        assert name == other
        local = re.fullmatch(r'_(\d+)', original)
        saved = re.fullmatch(r'\(\(\(\*_(\d+)\) as variant#(\d+)\)\.(\d+): .+\)', place)
        if local and saved:
            # The dumped body independently witnesses the Pin projection's argument origin.
            assert re.search(r'_' + saved[1] + r' = (?:copy|move) \(_1\.0: &mut ', text)
            field = states[int(saved[2])][int(saved[3])]
            sources[field].append(int(local[1]))
    expected = dict(saved_local_sources=[sorted(set(locals)) for locals in sources], states=states)
    assert frame == expected, (path.name, frame, expected)
    return sum(bool(locals) for locals in sources)


def initialization_probes(path, source):
    """Authored initialization cases, resolved through independent MIR local and call operands."""
    text = path.read_text()
    locals = {name: int(local) for name, local in re.findall(r'^\s+debug (\w+) => _(\d+);', text, re.M)}
    blocks = {int(match[1]): match[2] for match in re.finditer(r'^    bb(\d+)(?: \(cleanup\))?: \{\n(.*?)^    \}', text, re.M | re.S)}
    init = source['initialization']
    def move_path(name, projection=[]):
        matches = [index for index, item in enumerate(init['paths'])
                   if item['local'] == locals[name] and item['projection'] == projection]
        assert len(matches) == 1, (name, projection, matches)
        return matches[0]
    def checkpoint(number):
        matches = [block for block, body in blocks.items() if f'checkpoint::<{number}>()' in body]
        assert len(matches) == 1, (number, matches)
        return matches[0]
    probes = []
    def expect(label, block, when, path, initialized, uninitialized):
        state = init['points'][block][when]
        assert state is not None, label
        assert (path in state['maybe_initialized'], path in state['maybe_uninitialized']) == (initialized, uninitialized), (label, path, state)
        probes.append(dict(label=label, block=block, when=when, path=path,
                           initialized=initialized, uninitialized=uninitialized))
    expect('early-uninitialized', checkpoint(10), 'before', move_path('early'), False, True)
    expect('early-initialized', checkpoint(11), 'before', move_path('early'), True, False)
    expect('first-field-initialized', checkpoint(12), 'before', move_path('pair', [{'Field': 0}]), True, False)
    expect('second-field-initialized', checkpoint(12), 'before', move_path('pair', [{'Field': 1}]), True, False)
    expect('first-field-moved', checkpoint(13), 'before', move_path('pair', [{'Field': 0}]), False, True)
    expect('second-field-retained', checkpoint(13), 'before', move_path('pair', [{'Field': 1}]), True, False)
    # The edited source initializes the else branch as well, without adding another yield.
    conditional = 'Guard(const 93_i32)' not in text
    for number in [14, 15, 16]:
        expect(f'conditional-{number}', checkpoint(number), 'before', move_path('conditional'), True, conditional)
    expect('early-moved', checkpoint(16), 'before', move_path('early'), False, True)
    expect('return-initialized', checkpoint(16), 'before', move_path('returned'), True, False)
    calls = [(block, re.search(r'_(\d+) = take\(move _(\d+)\)', body)) for block, body in blocks.items()]
    calls = [(block, match) for block, match in calls if match]
    assert len(calls) == 1, calls
    block, call = calls[0]
    for role, local in [('argument', int(call[2])), ('destination', int(call[1]))]:
        matches = [index for index, item in enumerate(init['paths']) if item['local'] == local and item['projection'] == []]
        assert len(matches) == 1
        path = matches[0]
        expect('call-before-' + role, block, 'before', path, role == 'argument', role != 'argument')
        expect('call-after-' + role, block, 'after', path, False, True)
    return probes


def check(dump_dir, report_file):
    report = json.loads(report_file.read_text())
    rows = []
    for graph in report['async']['graphs']:
        source = graph['source_suspensions']
        assert source['status'] == 'captured', graph['item']
        matches = [path for path in dump_dir.glob('*-{closure#0}.*.analysis.after.mir')
                   if graph['item'].startswith(path.name.split('.')[0] + '$')
                   and '$' + path.name.split('.')[1].split('-')[0] + '$' in graph['item']]
        assert len(matches) == 1, (graph['item'], matches)
        path = matches[0]
        runtime = list(dump_dir.glob(path.name.split('.')[0] + '.' + path.name.split('.')[1] + '.*.runtime.after.mir'))
        assert len(runtime) == 1, (path, runtime)
        mapped_fields = check_frame(path, runtime[0], source['runtime_frame'])
        assert graph['runtime_frame_matches_type']
        init_probes = initialization_probes(path, source) if graph['item'].startswith('async_initialization$') else []
        expected = blocks_of(path)
        assert len(source['blocks']) == len(expected), (path, 'block count')
        for index, (actual, block) in enumerate(zip(source['blocks'], expected)):
            assert {key: actual[key] for key in block} == block, (path.name, index, actual, block)
        if graph['item'].startswith('async_liveness$'):
            locals_by_name = {name: int(local) for name, local in
                              re.findall(r'^\s+debug (\w+) => _(\d+);', path.read_text(), re.M)}
            points = source['yields']
            live = [set(point['liveness']['live']) for point in points]
            borrowed = [set(point['liveness']['borrowed']) for point in points]
            if '$borrowed$' in graph['item']:
                assert locals_by_name['view'] in live[0]
                view_type = re.search(r'let _' + str(locals_by_name['view']) + r': ([^;]+);', path.read_text())[1]
                if view_type.startswith('&'):
                    assert locals_by_name['retained'] in live[0]
                    assert locals_by_name['retained'] in borrowed[0]
                else:
                    assert locals_by_name['retained'] not in live[0]
                    assert locals_by_name['retained'] not in borrowed[0]
                assert locals_by_name['dead'] not in live[0]
            elif '$moved_guard$' in graph['item']:
                assert locals_by_name['moved'] not in live[0]
            elif '$held_guard$' in graph['item']:
                assert locals_by_name['held'] in live[0]
            elif '$two_states$' in graph['item']:
                assert locals_by_name['first'] in live[0] and locals_by_name['first'] not in live[1]
                assert locals_by_name['second'] not in live[0] and locals_by_name['second'] in live[1]
            else:
                raise AssertionError(graph['item'])
        rows.append(dict(item=graph['item'], mir=path.name, blocks=len(expected),
                         edges=sum(len(block['successors']) for block in expected),
                         liveness=graph['item'].startswith('async_liveness$'), mapped_fields=mapped_fields, initialization_probes=init_probes))
    assert rows, report_file
    return rows


if __name__ == '__main__':
    print(json.dumps(dict(status='pass', rows=check(Path(sys.argv[1]), Path(sys.argv[2]))), indent=2))

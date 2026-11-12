"""Cuts fixture 123 into smaller combinations, to find which keep their call-argument conversions.

python3 combos.py: writes probe/c_<name>.rs for each combination below.
"""
import re

W = '/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992'
G = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min'
src = open(W + '/examples/core-tests/123_moved_block_addresses.rs').read()
head, main = src.split('#[unsafe(no_mangle)]\npub fn rust_entry() {\n')
main = main.rsplit('}', 1)[0]

# rust_entry's sections, in order, split at blank lines.
sections = [s for s in main.split('\n\n')]
names = ['warm', 'repro', 'chain', 'ret', 'ptr', 'rounds', 'string']
assert len(sections) == len(names), len(sections)
section = dict(zip(names, sections))
# the warm section holds both the `total` warm-up and fixture 122's warm-up loop
total_warm, f122_warm = section['warm'].split('    let mut ok = true;\n')
f122_warm = '    let mut ok = true;\n' + f122_warm


def function_names(text):
    return re.findall(r'\nfn (\w+)\(', text)


def drop_functions(text, drop):
    for name in drop:
        text = re.sub(r'(///[^\n]*\n)*#\[inline\(never\)\]\nfn ' + name + r'\(.*?\n\}\n\n', '', text, flags=re.S)
    return text


combos = {
    'vec4': (['repro', 'chain', 'ret', 'ptr'], False),
    'vec4s': (['repro', 'chain', 'ret', 'ptr', 'string'], False),
    'vec4r': (['repro', 'chain', 'ret', 'ptr', 'rounds'], False),
    'all_nowarm': (['repro', 'chain', 'ret', 'ptr', 'rounds', 'string'], False),
    'rounds_warm': (['rounds'], True),
}
unused_by = {
    'rounds': ['make_n', 'fresh_addr', 'keep_n', 'shape_a'],
    'string': ['make_s', 'other', 'keep_s', 'build'],
    'ptr': ['make_p', 'keep_p'],
    'ret': ['pass'],
    'chain': ['chain'],
    'repro': ['keep'],
}
for name, (parts, warm) in combos.items():
    drop = []
    for part, fns in unused_by.items():
        if part not in parts:
            drop += fns
    if not warm:
        drop += ['vec_dangles', 'boxed', 'grown']
    body = drop_functions(head, drop)
    entry = total_warm + (f122_warm if warm else '') + '\n'.join(section[p] + '\n' for p in parts)
    if not warm:
        body = body.replace('use alloc::boxed::Box;\n', '')
    if 'string' not in parts:
        body = body.replace('use alloc::string::String;\n', '')
    text = body + '#[unsafe(no_mangle)]\npub fn rust_entry() {\n' + entry + '}\n'
    text = text.replace('#![no_main]\n', '#![no_main]\n#![allow(unused)]\n')
    open(f'{G}/probe/c_{name}.rs', 'w').write(text)
    print(name, function_names(text))

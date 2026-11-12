"""Applies or reverts one source mutant in the worktree.

python3 rust_mutant.py apply <name> | revert <name>
The original file is kept in mutsrc/<name>.orig and restored byte for byte by `revert`.
"""
import os
import shutil
import sys

G = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min'
W = '/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992'

MUTANTS = {
    # Round 1's bug: the call-argument conversion is marked as ending the value's life.
    'r1-call-arg-at-drop': ('jsc-link/src/plan/representation.rs',
        '''                                    to: fl,
                                    at_drop: false,''',
        '''                                    to: fl,
                                    at_drop: true,'''),
    # The drop cut spelled as a call boundary: the string and vector drops release nothing.
    'r2-drop-cut-not-released': ('jsc-link/src/plan/representation.rs',
        '''                                to: gl,
                                at_drop: true,''',
        '''                                to: gl,
                                at_drop: false,'''),
    # M2c: `str::from_utf8_unchecked` (a `&[u8]` to `&str` cast) decodes with the releasing helper.
    'm2c-from-utf8-unchecked-releases': ('jsc-link/src/lower/js/ptr.rs',
        '''        let (setup, bytes) = self.reread(bytes, &[BUF, OFF, LEN]);
        let decoded = jsast::rt_call(
            "bytes_str",''',
        '''        let (setup, bytes) = self.reread(bytes, &[BUF, OFF, LEN]);
        let decoded = jsast::rt_call(
            "drop_block_str",'''),
    # M2d: a `&str` built from a pointer and a length decodes with the releasing helper.
    'm2d-str-from-raw-parts-releases': ('jsc-link/src/lower/js/ptr.rs',
        '''                let (setup, data) = self.reread(data, &[BUF, OFF]);
                let decoded = jsast::rt_call(
                    "bytes_str",''',
        '''                let (setup, data) = self.reread(data, &[BUF, OFF]);
                let decoded = jsast::rt_call(
                    "drop_block_str",'''),
    # The adapter spells a call boundary with the releasing helper (round 1 at the adapter level).
    'r3-adapter-call-releases': ('jsc-link/src/lower/js/adapters.rs',
        '''pub fn convert(
    table: &TypeTable,
    interns: &Interns,
    value: Expr,
    ty: TypeId,
    from: Carrier,
    to: Carrier,
) -> Res<Expr> {
    spell(table, interns, value, ty, from, to, false)''',
        '''pub fn convert(
    table: &TypeTable,
    interns: &Interns,
    value: Expr,
    ty: TypeId,
    from: Carrier,
    to: Carrier,
) -> Res<Expr> {
    spell(table, interns, value, ty, from, to, true)'''),
}

action, name = sys.argv[1], sys.argv[2]
path, old, new = MUTANTS[name]
target = os.path.join(W, path)
os.makedirs(G + '/mutsrc', exist_ok=True)
backup = f'{G}/mutsrc/{name}.orig'
if action == 'apply':
    text = open(target).read()
    assert text.count(old) == 1, f'{name}: {text.count(old)} matches'
    shutil.copyfile(target, backup)
    open(target, 'w').write(text.replace(old, new))
    print(f'applied {name} to {path}')
else:
    shutil.copyfile(backup, target)
    print(f'reverted {name} in {path}')

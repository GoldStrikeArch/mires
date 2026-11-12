"""Writes fixture variants fx1..fxN from fx_body.rs pieces, so a census shows which shape keeps
the conversions. Usage: python3 variants.py"""
import re

P = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min/probe/'
HEAD = '#![no_std]\n#![no_main]\n#[path = "prelude.rs"]\nmod prelude;\nextern crate alloc;\nuse prelude::*;\ninclude!("%s_body.rs");\n'

body = open(P + 'fx_body.rs').read()
fns = body.split('#[unsafe(no_mangle)]')[0]

SEC0 = """    let mut nonnull = true;
    let mut i = 0u32;
    while i < 30 {
        if grown(i) == 0 || boxed(i) == 0 {
            nonnull = false;
        }
        i += 1;
    }
    print_bool(nonnull);
"""
SEC1V = """    let mut same = 0u32;
    let mut i = 0u32;
    while i < 500 {
        if vec_dangles(i) {
            same += 1;
        }
        i += 1;
    }
    print_u32(same);
"""
SEC1S = """    let mut same = 0u32;
    let mut i = 0u32;
    while i < 500 {
        if string_dangles(i) {
            same += 1;
        }
        i += 1;
    }
    print_u32(same);
"""
SEC4 = """    let mut total = 0u32;
    let mut nonnull = true;
    let mut i = 0u32;
    while i < 100 {
        let (a, sum) = records(i);
        if a == 0 {
            nonnull = false;
        }
        total = total.wrapping_add(sum);
        i += 1;
    }
    print_bool(nonnull);
    print_u32(total);
"""
variants = {
    'fx1': SEC0 + SEC1V,
    'fx2': SEC0 + SEC1S,
    'fx3': SEC0 + SEC1V + SEC1S,
    'fx4': SEC0 + SEC1V + SEC1S + SEC4,
}
FRESH16 = """
#[inline(never)]
fn fresh16(out: &mut usize) -> Vec<u16> {
    let mut v: Vec<u16> = black_box(Vec::with_capacity(32));
    v.push(1);
    *out = v.as_ptr() as usize;
    v
}

"""
SEC2 = """    let mut live: Vec<Vec<u16>> = Vec::new();
    let mut seen: Vec<usize> = Vec::new();
    let mut duplicate = false;
    let mut i = 0u32;
    while i < 200 {
        if !vec_dangles(i) || !string_dangles(i) {
            duplicate = true;
        }
        let mut a = 0usize;
        let v = fresh16(&mut a);
        let mut j = 0;
        while j < seen.len() {
            if seen[j] == a {
                duplicate = true;
            }
            j += 1;
        }
        seen.push(a);
        live.push(v);
        i += 1;
    }
    print_bool(duplicate);
    print_u32(live.len() as u32);
"""
SEC3 = """    let mut held = String::with_capacity(64);
    held.push_str("held");
    let address = held.as_ptr() as usize;
    print_str(held.as_str());
    let mut clash = false;
    let mut keep: Vec<Vec<u16>> = Vec::new();
    let mut i = 0u32;
    while i < 50 {
        let mut b = 0usize;
        let v = fresh16(&mut b);
        if b == address {
            clash = true;
        }
        keep.push(v);
        i += 1;
    }
    print_bool(clash);
    print_str(held.as_str());
    core::mem::forget(held);
    print_u32(keep.len() as u32);
"""
SEC2B = SEC2.replace("""        if !vec_dangles(i) || !string_dangles(i) {
            duplicate = true;
        }
""", "")
SEC2C = """    let mut seen: Vec<usize> = Vec::new();
    let mut live: Vec<Vec<u16>> = Vec::new();
    let mut duplicate = false;
    let mut i = 0u32;
    while i < 200 {
        let mut a = 0usize;
        let v = fresh16(&mut a);
        let mut j = 0;
        while j < seen.len() {
            if seen[j] == a {
                duplicate = true;
            }
            j += 1;
        }
        seen.push(a);
        live.push(v);
        i += 1;
    }
    print_bool(duplicate);
    print_u32(live.len() as u32);
"""
variants.update({
    'fx8': SEC0 + SEC2C + SEC1V + SEC1S + SEC4,
    'fx9': SEC0 + SEC1V + SEC1S + SEC4 + SEC2B,
    'fx5': SEC0 + SEC1V + SEC1S + SEC4 + SEC2,
    'fx6': SEC0 + SEC1V + SEC1S + SEC4 + SEC3,
    'fx7': SEC0 + SEC1V + SEC1S + SEC4 + SEC2 + SEC3,
})
for name, main in variants.items():
    open(P + name + '_body.rs', 'w').write(fns + FRESH16 + '#[unsafe(no_mangle)]\npub fn rust_entry() {\n' + main + '}\n')
    open(P + name + '.rs', 'w').write(HEAD % name)
print(' '.join(variants))

#!/usr/bin/env node
//
// The wasm32-LLVM-vs-JS differential fuzzer's program generator.
//
//   node scripts/fuzz/gen.mjs <seed> [--family a|b|c|d|e|f] [--include-large-enums] > prog.rs
//
// Emits one self-contained `#![no_std]` `#![no_main]` crate in the shape of an
// `examples/core-tests/` fixture: the same `extern "C"` print helpers, the same panic handler that
// prints a location and calls `js_abort`. That shape is the whole point — the SAME source has to
// compile through the ordinary LLVM backend for `wasm32-unknown-unknown` and through
// rustc_codegen_mires, so that the two runs can be compared line for line.
//
// EVERY GENERATED PROGRAM IS DEFINED BEHAVIOUR
// -------------------------------------------
// A difference between two backends only means something if the program has one meaning. So the
// generator never emits arithmetic overflow that is not `wrapping_`/`checked_`/`saturating_`/
// `overflowing_`, never shifts by an unmasked amount (`wrapping_shl`/`wrapping_shr` only), never
// indexes out of bounds, never divides by a value that could be zero outside `checked_div`, never
// reads an uninitialized or moved-from place, and never relies on unspecified NaN payload bits or
// `minNum`/`maxNum` signed-zero tie-breaking. Panics are emitted on purpose in exactly one place:
// the family `c` finale, where the program must abort on both sides after printing the same lines.
//
// THE SIX SEED FAMILIES
// ---------------------
//   a  expanded arithmetic: ALL 12 integer widths (i8..i128, u8..u128, isize, usize) + f32 & f64
//      (including -0.0, INFINITY, NEG_INFINITY, NAN, subnormals, to_bits/from_bits), wrapping /
//      checked / saturating / overflowing ops, shifts, rotates, bit ops, casts, comparisons.
//   b  aggregates & control flow: structs, nested structs, enums (3-7 variants; 10-variant enum
//      with --include-large-enums), nested Option/Result, arrays, subslices, for/while/loop with
//      break/continue and labeled loops/blocks, Fn/FnMut/FnOnce closures, and dyn Trait dispatch.
//   c  destructors & control-flow drops: Drop guards across nested scopes, loops with early
//      break/continue, match arms, mem::replace/mem::swap, moves, early returns, and a final
//      `#[track_caller]` panic the program is required to abort on.
//   d  heap & collections (`extern crate alloc`): Vec<i32>, Vec<u8>, Vec<f64>, Vec<Struct> (SoA),
//      String, Box<T>, Box<[T]>, Rc<T>, BTreeMap<i32, i32>, format!, sort_unstable, push/pop/
//      extend/retain/remove/swap_remove/chunks/windows (with `#[cfg(fuzz_wasm_alloc)]` bump
//      allocator for LLVM wasm32).
//   e  aliasing, mutable borrows, interior mutability & statics: Cell, RefCell, &mut field
//      reborrows, #[inline(never)] &mut helper mutations (abi_flat copy-back + PEA), Copy
//      mutation isolation checks, static/const tables with Option<&T> niches and NonZero,
//      core::hint::black_box barriers.
//   f  JS-engine & representation stress: structs with JS-reserved/prototype-adjacent field names
//      (constructor, toString, valueOf, hasOwnProperty, then, TAG), i64/u64 fields inside structs
//      & tight loops (i64_pair & bigint_props), multi-arm match & byte-delimiter scans
//      (switch_lut & tag_encode), and UTF-8 &str/String with ASCII + multi-byte + 4-byte astral
//      Unicode characters.
//
// The family is chosen from `seed % 6` unless `--family` names one, so a run of N seeds covers all
// six. Output is a pure function of (seed, family, flags): the same seed always produces the same
// file, which makes any reported mismatch reproducible from its seed alone.

const FAMILIES = ['a', 'b', 'c', 'd', 'e', 'f']

const argv = process.argv.slice(2)
const seed = Number(argv[0])
if (!Number.isInteger(seed) || seed < 0) {
  console.error('usage: gen.mjs <seed> [--family a|b|c|d|e|f] [--include-large-enums]')
  process.exit(2)
}

let familyFlag = null
let includeLargeEnums = false
for (let i = 1; i < argv.length; i += 1) {
  if (argv[i] === '--family') {
    familyFlag = argv[i + 1] ?? ''
    i += 1
  } else if (argv[i] === '--include-large-enums') {
    includeLargeEnums = true
  } else {
    console.error(`gen.mjs: unknown option: ${argv[i]}`)
    process.exit(2)
  }
}

if (familyFlag !== null && !FAMILIES.includes(familyFlag)) {
  console.error(`gen.mjs: --family must be one of ${FAMILIES.join(', ')} (got ${familyFlag})`)
  process.exit(2)
}
const family = familyFlag ?? FAMILIES[seed % FAMILIES.length]

// mulberry32: 32 bits of state, no dependencies, identical everywhere.
function mulberry32(state) {
  let a = (state + 0x6d2b79f5) >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const random = mulberry32(seed * 2654435761 + 12345)
const int = (min, max) => min + Math.floor(random() * (max - min + 1))
const pick = (values) => values[int(0, values.length - 1)]
const chance = (probability) => random() < probability

// ---------------------------------------------------------------------------- integer & float tables

const CONSTANTS = {
  i8: () => pick(['0', '1', '-1', '2', '-7', '63', '127', '-128', 'i8::MIN', 'i8::MAX', String(int(-120, 120))]),
  u8: () => pick(['0', '1', '2', '7', '127', '128', '255', 'u8::MAX', String(int(0, 255))]),
  i16: () => pick(['0', '1', '-1', '2', '-13', '255', '-256', '32767', '-32768', 'i16::MIN', 'i16::MAX', String(int(-30000, 30000))]),
  u16: () => pick(['0', '1', '2', '255', '256', '32767', '65535', 'u16::MAX', String(int(0, 65535))]),
  i32: () => pick(['0', '1', '-1', '2', '-7', '31', '255', '65535', '1073741823', 'i32::MIN', 'i32::MAX', String(int(-10000, 10000))]),
  u32: () => pick(['0', '1', '2', '255', '65535', '4294967295', 'u32::MAX', String(int(0, 100000))]),
  i64: () => pick(['0', '1', '-1', '9007199254740993', '-9007199254740993', 'i64::MIN', 'i64::MAX', String(int(-1000000, 1000000))]),
  u64: () => pick(['0', '1', '18446744073709551615', 'u64::MAX', '9007199254740993', String(int(0, 1000000))]),
  i128: () => pick(['0', '1', '-1', '18446744073709551617', '-18446744073709551617', 'i128::MIN', 'i128::MAX', String(int(-1000000, 1000000))]),
  u128: () => pick(['0', '1', '340282366920938463463374607431768211455', 'u128::MAX', '18446744073709551617', String(int(0, 1000000))]),
  isize: () => pick(['0', '1', '-1', '2', '-17', '255', '65535', 'isize::MIN', 'isize::MAX', String(int(-5000, 5000))]),
  usize: () => pick(['0', '1', '2', '255', '65535', 'usize::MAX', String(int(0, 50000))]),
}

const BITS = {
  i8: 8,
  u8: 8,
  i16: 16,
  u16: 16,
  i32: 32,
  u32: 32,
  i64: 64,
  u64: 64,
  i128: 128,
  u128: 128,
  isize: 32,
  usize: 32,
}
const INT_TYPES = Object.keys(CONSTANTS)

// A negative literal is parenthesized: `-1i64.count_ones()` parses as `-(1i64.count_ones())`.
const literal = (type) => {
  const value = CONSTANTS[type]()
  const spelled = /^[a-z]/.test(value) ? value : `${value}${type}`
  return spelled.startsWith('-') ? `(${spelled})` : spelled
}

const PRINTERS = {
  i8: (e) => `print_i32((${e}) as i32);`,
  u8: (e) => `print_u32((${e}) as u32);`,
  i16: (e) => `print_i32((${e}) as i32);`,
  u16: (e) => `print_u32((${e}) as u32);`,
  i32: (e) => `print_i32(${e});`,
  u32: (e) => `print_u32(${e});`,
  i64: (e) => `print_i64(${e});`,
  u64: (e) => `print_u64(${e});`,
  i128: (e) => `print_i128(${e});`,
  u128: (e) => `print_u128(${e});`,
  isize: (e) => `print_i32((${e}) as i32);`,
  usize: (e) => `print_usize(${e});`,
}

const F32_CONSTANTS = () =>
  pick([
    '0.0f32',
    '(-0.0f32)',
    '1.0f32',
    '(-1.0f32)',
    '0.5f32',
    '(-3.25f32)',
    '16777217.0f32',
    'f32::INFINITY',
    'f32::NEG_INFINITY',
    'f32::NAN',
    'f32::MIN_POSITIVE',
    'f32::from_bits(1u32)',
    `(${int(-500, 500)} as f32 * 0.25f32)`,
  ])

const F64_CONSTANTS = () =>
  pick([
    '0.0f64',
    '(-0.0f64)',
    '1.0f64',
    '(-1.0f64)',
    '0.5f64',
    '(-3.25f64)',
    '1e18f64',
    '(-1e18f64)',
    'f64::INFINITY',
    'f64::NEG_INFINITY',
    'f64::NAN',
    'f64::MIN_POSITIVE',
    'f64::from_bits(1u64)',
    `(${int(-5000, 5000)} as f64 * 0.125f64)`,
  ])

// ---------------------------------------------------------------------------- family a

function familyA() {
  const items = `
fn log_f32_safe(x: f32) {
    print_bool(x.is_nan());
    print_bool(x.is_infinite());
    print_bool(x.is_finite());
    if x.is_nan() {
        print_str("f32:nan");
    } else {
        print_f64(x as f64);
        let bits = x.to_bits();
        print_u32(bits);
        let round = f32::from_bits(bits);
        print_u32(round.to_bits());
        print_bool(round.is_sign_negative());
    }
}

fn log_f64_safe(x: f64) {
    print_bool(x.is_nan());
    print_bool(x.is_infinite());
    print_bool(x.is_finite());
    if x.is_nan() {
        print_str("f64:nan");
    } else {
        print_f64(x);
        let bits = x.to_bits();
        print_u64(bits);
        let round = f64::from_bits(bits);
        print_u64(round.to_bits());
        print_bool(round.is_sign_negative());
    }
}
`
  const lines = []
  const live = Object.fromEntries(INT_TYPES.map((type) => [type, []]))
  let next = 0
  const name = () => `v${next++}`

  const operand = (type) => {
    const pool = live[type]
    return pool.length > 0 && chance(0.6) ? pick(pool) : literal(type)
  }

  const bind = (type, expression) => {
    const binding = name()
    lines.push(`    let ${binding}: ${type} = ${expression};`)
    live[type].push(binding)
    if (chance(0.75)) lines.push(`    ${PRINTERS[type](binding)}`)
    return binding
  }

  const arithmetic = (type) => {
    const a = operand(type)
    const b = operand(type)
    const shift = `${int(0, BITS[type] + 8)}u32`
    return pick([
      `${a}.wrapping_add(${b})`,
      `${a}.wrapping_sub(${b})`,
      `${a}.wrapping_mul(${b})`,
      `${a}.wrapping_neg()`,
      `${a}.checked_add(${b}).unwrap_or(${literal(type)})`,
      `${a}.checked_sub(${b}).unwrap_or(${literal(type)})`,
      `${a}.checked_mul(${b}).unwrap_or(${literal(type)})`,
      `${a}.checked_div(${b}).unwrap_or(${literal(type)})`,
      `${a}.checked_rem(${b}).unwrap_or(${literal(type)})`,
      `${a}.saturating_add(${b})`,
      `${a}.saturating_sub(${b})`,
      `${a}.saturating_mul(${b})`,
      `${a}.wrapping_shl(${shift})`,
      `${a}.wrapping_shr(${shift})`,
      `${a}.rotate_left(${shift})`,
      `${a}.rotate_right(${shift})`,
      `${a} & ${b}`,
      `${a} | ${b}`,
      `${a} ^ ${b}`,
      `!${a}`,
      `${a}.count_ones() as ${type}`,
      `${a}.leading_zeros() as ${type}`,
      `${a}.trailing_zeros() as ${type}`,
      `${a}.swap_bytes()`,
      `${a}.reverse_bits()`,
      `if ${a} > ${b} { ${a} } else { ${b} }`,
      `${a}.min(${b})`,
      `${a}.max(${b})`,
    ])
  }

  // Hit every single one of the 12 integer widths at least once, plus extra random ops.
  for (let index = 0; index < INT_TYPES.length + int(4, 8); index += 1) {
    const type = index < INT_TYPES.length ? INT_TYPES[index] : pick(INT_TYPES)
    bind(type, arithmetic(type))
  }

  // Overflowing ops (checking both wrapped value and overflow boolean flag).
  for (let index = 0; index < int(4, 6); index += 1) {
    const type = pick(INT_TYPES)
    const a = operand(type)
    const b = operand(type)
    const op = pick(['overflowing_add', 'overflowing_sub', 'overflowing_mul'])
    const r = name()
    lines.push(`    let (${r}_val, ${r}_ov) = ${a}.${op}(${b});`)
    lines.push(`    ${PRINTERS[type](`${r}_val`)}`)
    lines.push(`    print_bool(${r}_ov);`)
    live[type].push(`${r}_val`)
  }

  // Integer-to-integer casts across all widths.
  for (let index = 0; index < int(5, 8); index += 1) {
    const from = pick(INT_TYPES)
    const to = pick(INT_TYPES)
    bind(to, `${operand(from)} as ${to}`)
  }

  // f32 and f64 arithmetic, special values (-0.0, INF, -INF, NAN, subnormals), to_bits/from_bits.
  const liveF32 = []
  const liveF64 = []
  const opF32 = () => (liveF32.length > 0 && chance(0.55) ? pick(liveF32) : F32_CONSTANTS())
  const opF64 = () => (liveF64.length > 0 && chance(0.55) ? pick(liveF64) : F64_CONSTANTS())

  // Always explicitly exercise -0.0, INFINITY, NEG_INFINITY, NAN in both f32 and f64.
  lines.push(`    log_f32_safe(-0.0f32);`)
  lines.push(`    log_f32_safe(f32::INFINITY);`)
  lines.push(`    log_f32_safe(f32::NEG_INFINITY);`)
  lines.push(`    log_f32_safe(f32::NAN);`)
  lines.push(`    log_f64_safe(-0.0f64);`)
  lines.push(`    log_f64_safe(f64::INFINITY);`)
  lines.push(`    log_f64_safe(f64::NEG_INFINITY);`)
  lines.push(`    log_f64_safe(f64::NAN);`)

  for (let index = 0; index < int(3, 5); index += 1) {
    const fa = opF32()
    const fb = opF32()
    const binding = name()
    const expr = pick([
      `${fa} + ${fb}`,
      `${fa} - ${fb}`,
      `${fa} * ${fb}`,
      `${fa} / ${fb}`,
      `${fa} % ${fb}`,
      `-${fa}`,
      `${fa}.recip()`,
      `${operand(pick(INT_TYPES))} as f32`,
      `(${opF64()}) as f32`,
    ])
    lines.push(`    let ${binding}: f32 = ${expr};`)
    lines.push(`    log_f32_safe(${binding});`)
    liveF32.push(binding)
  }

  for (let index = 0; index < int(3, 5); index += 1) {
    const fa = opF64()
    const fb = opF64()
    const binding = name()
    const expr = pick([
      `${fa} + ${fb}`,
      `${fa} - ${fb}`,
      `${fa} * ${fb}`,
      `${fa} / ${fb}`,
      `${fa} % ${fb}`,
      `-${fa}`,
      `${fa}.recip()`,
      `${operand(pick(INT_TYPES))} as f64`,
      `(${opF32()}) as f64`,
    ])
    lines.push(`    let ${binding}: f64 = ${expr};`)
    lines.push(`    log_f64_safe(${binding});`)
    liveF64.push(binding)
  }

  // Float-to-integer saturating casts across widths (including NaN and infinities).
  for (let index = 0; index < int(4, 6); index += 1) {
    const to = pick(INT_TYPES)
    if (chance(0.5)) {
      const f = opF32()
      lines.push(`    ${PRINTERS[to](`(${f}) as ${to}`)}`)
    } else {
      const f = opF64()
      lines.push(`    ${PRINTERS[to](`(${f}) as ${to}`)}`)
    }
  }

  // Comparisons across integers and floats (plus total_cmp on non-NaN pairs).
  for (let index = 0; index < int(4, 6); index += 1) {
    const type = pick(INT_TYPES)
    const a = operand(type)
    const b = operand(type)
    lines.push(`    print_bool(${a} ${pick(['<', '<=', '>', '>=', '==', '!='])} ${b});`)
  }
  for (let index = 0; index < 2; index += 1) {
    const fa = opF64()
    const fb = opF64()
    lines.push(`    print_bool((${fa}) ${pick(['<', '<=', '>', '>=', '==', '!='])} (${fb}));`)
    lines.push(`    if !(${fa}).is_nan() && !(${fb}).is_nan() {`)
    lines.push(`        print_i32((${fa}).total_cmp(&(${fb})) as i32);`)
    lines.push(`    }`)
  }

  return { body: lines.join('\n'), items, abort: false, usesAlloc: false }
}

// ---------------------------------------------------------------------------- family b

function familyB() {
  const largeEnumItems = includeLargeEnums
    ? `
#[derive(Clone, Copy)]
enum LargeOp {
    V0,
    V1(i32),
    V2(i32, i32),
    V3 { a: u16, b: i16 },
    V4(u32),
    V5(bool, i32),
    V6(i32, u8),
    V7,
    V8(i64),
    V9(i32, i32, i32),
}

fn eval_large_op(op: LargeOp) -> i32 {
    match op {
        LargeOp::V0 => 11,
        LargeOp::V1(x) => x.wrapping_mul(3),
        LargeOp::V2(a, b) => a.wrapping_sub(b),
        LargeOp::V3 { a, b } => (a as i32).wrapping_add(b as i32),
        LargeOp::V4(u) => (u as i32).wrapping_add(7),
        LargeOp::V5(flag, x) => if flag { x } else { x.wrapping_neg() },
        LargeOp::V6(x, s) => x.wrapping_shl((s & 7) as u32),
        LargeOp::V7 => -19,
        LargeOp::V8(w) => (w as i32) ^ ((w >> 32) as i32),
        LargeOp::V9(a, b, c) => a.wrapping_add(b).wrapping_add(c),
    }
}
`
    : ''

  const items = `
#[derive(Clone, Copy, PartialEq)]
struct Point {
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, PartialEq)]
struct Nested {
    origin: Point,
    scale: u16,
    active: bool,
}

struct Wrapper(i32, u32);

struct Consumed {
    seed: i32,
}

#[derive(Clone, Copy)]
enum Shape {
    Empty,
    Single(i32),
    Pair(i32, i32),
    Rect { w: u16, h: u16 },
    Scaled(Point, i32),
    Flagged(bool, i32),
}

trait Describe {
    fn value(&self) -> i32;
}

impl Describe for Point {
    fn value(&self) -> i32 {
        self.x.wrapping_mul(${int(2, 9)}).wrapping_add(self.y)
    }
}

impl Describe for Nested {
    fn value(&self) -> i32 {
        let base = self.origin.value().wrapping_add(self.scale as i32);
        if self.active { base } else { base.wrapping_neg() }
    }
}

impl Describe for Wrapper {
    fn value(&self) -> i32 {
        self.0.wrapping_sub(self.1 as i32)
    }
}

fn area(shape: Shape) -> i32 {
    match shape {
        Shape::Empty => ${int(-50, 50)},
        Shape::Single(value) => value.wrapping_mul(value),
        Shape::Pair(a, b) => a.wrapping_mul(b),
        Shape::Rect { w, h } => (w as i32).wrapping_mul(h as i32),
        Shape::Scaled(p, k) => p.x.wrapping_add(p.y).wrapping_mul(k),
        Shape::Flagged(f, v) => if f { v.wrapping_add(5) } else { v.wrapping_sub(5) },
    }
}

fn halve(value: i32) -> Option<i32> {
    if value % 2 == 0 { Some(value / 2) } else { None }
}

fn chain(value: i32) -> Option<i32> {
    let first = halve(value)?;
    let second = halve(first)?;
    Some(second.wrapping_add(${int(1, 20)}))
}

fn call_fn<F: Fn(i32) -> i32>(f: &F, x: i32) -> i32 {
    f(x).wrapping_add(f(x.wrapping_add(1)))
}

fn call_fn_once<F: FnOnce(i32) -> i32>(f: F, x: i32) -> i32 {
    f(x)
}
${largeEnumItems}`

  const lines = []
  let next = 0
  const name = () => `b${next++}`

  const recipes = [
    // 0: structs, nested structs, and dyn Trait slice dispatch
    () => {
      const p = name()
      lines.push(`    let ${p} = Point { x: ${int(-100, 100)}, y: ${int(-100, 100)} };`)
      lines.push(`    let ${p}_n = Nested { origin: ${p}, scale: ${int(1, 40)}u16, active: ${chance(0.5)} };`)
      lines.push(`    let ${p}_w = Wrapper(${int(-200, 200)}, ${int(0, 200)}u32);`)
      lines.push(`    print_bool(${p} == Point { x: ${int(-3, 3)}, y: ${int(-3, 3)} });`)
      lines.push(`    let ${p}_objs: [&dyn Describe; 3] = [&${p}, &${p}_n, &${p}_w];`)
      lines.push(`    for obj in ${p}_objs {`)
      lines.push(`        print_i32(obj.value());`)
      lines.push(`    }`)
    },
    // 1: 6-variant enum matching
    () => {
      const s = name()
      const variant = pick([
        'Shape::Empty',
        `Shape::Single(${int(-40, 40)})`,
        `Shape::Pair(${int(-40, 40)}, ${int(-40, 40)})`,
        `Shape::Rect { w: ${int(1, 30)}u16, h: ${int(1, 30)}u16 }`,
        `Shape::Scaled(Point { x: ${int(-20, 20)}, y: ${int(-20, 20)} }, ${int(-5, 5)})`,
        `Shape::Flagged(${chance(0.5)}, ${int(-50, 50)})`,
      ])
      lines.push(`    let ${s} = ${variant};`)
      lines.push(`    print_i32(area(${s}));`)
      lines.push(`    print_i32(match ${s} {`)
      lines.push(`        Shape::Empty => ${int(-9, 9)},`)
      lines.push(`        Shape::Single(v) => v,`)
      lines.push(`        Shape::Pair(a, b) => a.wrapping_add(b),`)
      lines.push(`        Shape::Rect { w, h } => (w as i32) - (h as i32),`)
      lines.push(`        Shape::Scaled(p, k) => p.x.wrapping_mul(k),`)
      lines.push(`        Shape::Flagged(f, v) => if f { v } else { -v },`)
      lines.push(`    });`)
    },
    // 2: nested Option/Result combinators + ?
    () => {
      const o = name()
      const nestedOpt = pick([
        'None',
        'Some(None)',
        `Some(Some(${int(-60, 60)}))`,
      ])
      const nestedRes = pick([
        `Ok(Some(${int(-50, 50)}))`,
        'Ok(None)',
        `Err(${int(-50, 50)})`,
      ])
      lines.push(`    let ${o}: Option<Option<i32>> = ${nestedOpt};`)
      lines.push(`    print_i32(${o}.flatten().and_then(halve).unwrap_or(${int(-9, 9)}));`)
      lines.push(`    let ${o}_r: Result<Option<i32>, i32> = ${nestedRes};`)
      lines.push(`    print_bool(${o}_r.is_ok());`)
      lines.push(`    print_i32(${o}_r.map(|inner| inner.unwrap_or(${int(-5, 5)})).unwrap_or_else(|e| e.wrapping_sub(3)));`)
      lines.push(`    print_i32(chain(${int(-32, 32)}).unwrap_or(${int(-9, 9)}));`)
    },
    // 3: real control flow (for/while/labeled loop/labeled block + range BCE)
    () => {
      const cf = name()
      const values = Array.from({ length: int(6, 10) }, () => int(-25, 25))
      const stop = int(10, 40)
      lines.push(`    let ${cf}_arr = [${values.map((v) => `${v}i32`).join(', ')}];`)
      lines.push(`    let mut ${cf}_sum = 0i32;`)
      lines.push(`    for i in 0..${cf}_arr.len() {`)
      lines.push(`        let v = ${cf}_arr[i];`)
      lines.push(`        if v == 0 { continue; }`)
      lines.push(`        ${cf}_sum = ${cf}_sum.wrapping_add(v.wrapping_mul((i as i32) + 1));`)
      lines.push(`        if ${cf}_sum > ${stop} { break; }`)
      lines.push(`    }`)
      lines.push(`    print_i32(${cf}_sum);`)
      lines.push(`    let mut ${cf}_w = 0usize;`)
      lines.push(`    let mut ${cf}_wacc = 1i32;`)
      lines.push(`    while ${cf}_w < ${cf}_arr.len() && ${cf}_wacc < 500 {`)
      lines.push(`        let x = ${cf}_arr[${cf}_w];`)
      lines.push(`        ${cf}_w += 1;`)
      lines.push(`        if x < -15 { continue; }`)
      lines.push(`        ${cf}_wacc = ${cf}_wacc.wrapping_add(x.wrapping_abs());`)
      lines.push(`    }`)
      lines.push(`    print_i32(${cf}_wacc);`)
      lines.push(`    let ${cf}_outer = 'outer: loop {`)
      lines.push(`        let mut acc = ${int(1, 9)}i32;`)
      lines.push(`        for i in 0..${cf}_arr.len() {`)
      lines.push(`            for j in 0..3usize {`)
      lines.push(`                acc = acc.wrapping_add(${cf}_arr[i]).wrapping_add(j as i32);`)
      lines.push(`                if acc % 7 == 0 { break 'outer acc; }`)
      lines.push(`            }`)
      lines.push(`        }`)
      lines.push(`        break 'outer acc.wrapping_add(13);`)
      lines.push(`    };`)
      lines.push(`    print_i32(${cf}_outer);`)
      lines.push(`    let ${cf}_blk: i32 = 'blk: {`)
      lines.push(`        if ${cf}_outer % 2 == 0 { break 'blk ${cf}_outer.wrapping_mul(3); }`)
      lines.push(`        ${cf}_outer.wrapping_sub(5)`)
      lines.push(`    };`)
      lines.push(`    print_i32(${cf}_blk);`)
    },
    // 4: arrays, subslices, iterators & windows
    () => {
      const a = name()
      const values = Array.from({ length: int(5, 8) }, () => int(-30, 30))
      lines.push(`    let ${a} = [${values.map((v) => `${v}i32`).join(', ')}];`)
      const start = int(0, values.length - 3)
      const end = int(start + 2, values.length)
      lines.push(`    let ${a}_slice = &${a}[${start}..${end}];`)
      lines.push(`    print_usize(${a}_slice.len());`)
      lines.push(`    print_i32(${a}_slice.iter().copied().fold(0i32, |x, y| x.wrapping_add(y)));`)
      lines.push(`    for win in ${a}_slice.windows(2) {`)
      lines.push(`        print_i32(win[0].wrapping_sub(win[1]));`)
      lines.push(`    }`)
      lines.push(`    print_i32((0..${int(3, 9)}i32).rev().take(${int(1, 4)}).sum::<i32>());`)
      lines.push(`    print_i32((0..${int(2, 6)}i32).zip(${int(5, 9)}..${int(10, 14)}i32).map(|(x, y)| x.wrapping_mul(y)).sum::<i32>());`)
    },
    // 5: Fn, FnMut, and FnOnce closures
    () => {
      const c = name()
      const factor = int(2, 7)
      lines.push(`    let ${c}_k = ${int(-20, 20)}i32;`)
      lines.push(`    let ${c}_fn = |x: i32| x.wrapping_mul(${factor}).wrapping_add(${c}_k);`)
      lines.push(`    print_i32(call_fn(&${c}_fn, ${int(-10, 10)}));`)
      lines.push(`    let mut ${c}_acc = ${int(-10, 10)}i32;`)
      lines.push(`    let mut ${c}_mut = |delta: i32| { ${c}_acc = ${c}_acc.wrapping_add(delta); ${c}_acc };`)
      lines.push(`    print_i32(${c}_mut(${int(-15, 15)}));`)
      lines.push(`    print_i32(${c}_mut(${int(-15, 15)}));`)
      lines.push(`    let ${c}_owned = Consumed { seed: ${int(-30, 30)} };`)
      lines.push(`    let ${c}_once = move |x: i32| { let moved = ${c}_owned; moved.seed.wrapping_mul(x) };`)
      lines.push(`    print_i32(call_fn_once(${c}_once, ${int(-8, 8)}));`)
    },
  ]

  // Always include dyn Trait, control flow, and closures; add random extra recipes.
  recipes[0]()
  recipes[3]()
  recipes[5]()
  for (let index = 0; index < int(2, 4); index += 1) pick(recipes)()

  if (includeLargeEnums) {
    const lg = name()
    lines.push(`    let ${lg}_ops = [`)
    lines.push(`        LargeOp::V0, LargeOp::V1(${int(-20, 20)}), LargeOp::V2(${int(-20, 20)}, ${int(-20, 20)}),`)
    lines.push(`        LargeOp::V3 { a: ${int(0, 50)}u16, b: ${int(-50, 50)}i16 }, LargeOp::V4(${int(0, 99)}u32),`)
    lines.push(`        LargeOp::V5(${chance(0.5)}, ${int(-30, 30)}), LargeOp::V6(${int(-20, 20)}, ${int(0, 7)}u8),`)
    lines.push(`        LargeOp::V7, LargeOp::V8(${int(-500, 500)}i64), LargeOp::V9(1, 2, ${int(-9, 9)}),`)
    lines.push(`    ];`)
    lines.push(`    for op in ${lg}_ops { print_i32(eval_large_op(op)); }`)
  }

  return { body: lines.join('\n'), items, abort: false, usesAlloc: false }
}

// ---------------------------------------------------------------------------- family c

function familyC() {
  const items = `
struct Guard(i32);

impl Drop for Guard {
    fn drop(&mut self) {
        print_i32(self.0);
    }
}

struct Pair {
    first: Guard,
    second: Guard,
}

enum Held {
    Empty,
    One(Guard),
    Two(Guard, Guard),
}

fn consume(guard: Guard) -> i32 {
    let value = guard.0;
    print_str("consume");
    value.wrapping_mul(${int(2, 5)})
}

fn early(value: i32) -> i32 {
    let _guard = Guard(${int(100, 199)});
    if value > ${int(-10, 10)} {
        print_str("early");
        return value.wrapping_add(${int(1, 9)});
    }
    let _later = Guard(${int(200, 299)});
    print_str("late");
    value.wrapping_sub(${int(1, 9)})
}

#[track_caller]
fn demand(value: Option<i32>) -> i32 {
    value.unwrap()
}
`
  const lines = []
  let next = 0
  const name = () => `g${next++}`

  const recipes = [
    // Nested scopes
    () => {
      const g = name()
      lines.push(`    {`)
      lines.push(`        let _${g}_a = Guard(${int(1, 40)});`)
      lines.push(`        {`)
      lines.push(`            let _${g}_b = Guard(${int(41, 80)});`)
      lines.push(`            print_str("inner-scope");`)
      lines.push(`        }`)
      lines.push(`        print_str("outer-scope");`)
      lines.push(`    }`)
    },
    // Struct pair + explicit drop
    () => {
      const p = name()
      lines.push(`    let ${p} = Pair { first: Guard(${int(1, 40)}), second: Guard(${int(41, 80)}) };`)
      lines.push(`    print_i32(${p}.first.0.wrapping_add(${p}.second.0));`)
      lines.push(`    drop(${p});`)
    },
    // Loop with live Guard + continue & break
    () => {
      const lp = name()
      const base = int(300, 390)
      lines.push(`    'lp_${lp}: for i in 0..5i32 {`)
      lines.push(`        let _${lp}_iter = Guard(${base} + i);`)
      lines.push(`        if i == 1 {`)
      lines.push(`            let _${lp}_cont = Guard(${base + 10} + i);`)
      lines.push(`            print_str("loop-continue");`)
      lines.push(`            continue 'lp_${lp};`)
      lines.push(`        }`)
      lines.push(`        if i == 3 {`)
      lines.push(`            let _${lp}_brk = Guard(${base + 20} + i);`)
      lines.push(`            print_str("loop-break");`)
      lines.push(`            break 'lp_${lp};`)
      lines.push(`        }`)
      lines.push(`        print_i32(i);`)
      lines.push(`    }`)
    },
    // Match arm moving vs dropping enum payload guards
    () => {
      const m = name()
      const init = pick([
        `Held::One(Guard(${int(400, 440)}))`,
        `Held::Two(Guard(${int(441, 470)}), Guard(${int(471, 499)}))`,
        'Held::Empty',
      ])
      lines.push(`    let ${m} = ${init};`)
      lines.push(`    match ${m} {`)
      lines.push(`        Held::Empty => print_str("held-empty"),`)
      lines.push(`        Held::One(g) => { print_i32(consume(g)); }`)
      lines.push(`        Held::Two(a, b) => { print_i32(a.0.wrapping_add(b.0)); }`)
      lines.push(`    }`)
    },
    // mem::replace and mem::swap
    () => {
      const s = name()
      lines.push(`    let mut ${s}_a = Guard(${int(500, 530)});`)
      lines.push(`    let mut ${s}_b = Guard(${int(531, 560)});`)
      lines.push(`    let ${s}_old = core::mem::replace(&mut ${s}_a, Guard(${int(561, 590)}));`)
      lines.push(`    print_i32(${s}_old.0);`)
      lines.push(`    drop(${s}_old);`)
      lines.push(`    core::mem::swap(&mut ${s}_a, &mut ${s}_b);`)
      lines.push(`    print_i32(${s}_a.0);`)
      lines.push(`    print_i32(${s}_b.0);`)
    },
    // Early return + move
    () => {
      lines.push(`    print_i32(early(${int(-20, 20)}));`)
      lines.push(`    print_i32(consume(Guard(${int(600, 690)})));`)
    },
  ]

  // Guarantee loop drops, match drops, and mem::replace/swap are exercised every run.
  recipes[2]()
  recipes[3]()
  recipes[4]()
  for (let index = 0; index < int(2, 4); index += 1) pick(recipes)()

  // The finale: a `#[track_caller]` panic. Both backends must print the exact location and abort.
  lines.push(`    print_str("about to panic");`)
  lines.push(`    let missing: Option<i32> = None;`)
  lines.push(`    print_i32(demand(missing));`)
  lines.push(`    print_str("unreachable");`)

  return { body: lines.join('\n'), items, abort: true, usesAlloc: false }
}

// ---------------------------------------------------------------------------- family d

function familyD() {
  const items = `
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq)]
struct Item {
    id: i32,
    weight: u32,
    score: f64,
}

trait Eval {
    fn eval(&self) -> i32;
}

impl Eval for Item {
    fn eval(&self) -> i32 {
        self.id.wrapping_mul(3).wrapping_add(self.weight as i32)
    }
}
`
  const lines = []
  let next = 0
  const name = () => `d${next++}`

  const recipes = [
    // 0: Vec<i32> & Vec<u8> full operation suite
    () => {
      const v = name()
      const init = Array.from({ length: int(5, 9) }, () => int(-40, 40))
      lines.push(`    let mut ${v}: Vec<i32> = vec![${init.map((x) => `${x}i32`).join(', ')}];`)
      lines.push(`    ${v}.push(${int(-50, 50)});`)
      lines.push(`    ${v}.extend([${int(-20, 20)}, ${int(-20, 20)}, ${int(-20, 20)}]);`)
      lines.push(`    print_i32(${v}.pop().unwrap_or(0));`)
      lines.push(`    if ${v}.len() > 3 { print_i32(${v}.remove(1)); }`)
      lines.push(`    if ${v}.len() > 2 { print_i32(${v}.swap_remove(0)); }`)
      lines.push(`    ${v}.retain(|&x| x != ${int(-5, 5)});`)
      lines.push(`    ${v}.sort_unstable();`)
      lines.push(`    print_usize(${v}.len());`)
      lines.push(`    for ch in ${v}.chunks(3) {`)
      lines.push(`        print_i32(ch.iter().copied().fold(0i32, |a, b| a.wrapping_add(b)));`)
      lines.push(`    }`)
      lines.push(`    for win in ${v}.windows(2) {`)
      lines.push(`        print_i32(win[1].wrapping_sub(win[0]));`)
      lines.push(`    }`)
      const bytes = Array.from({ length: int(5, 8) }, () => int(65, 90))
      lines.push(`    let mut ${v}_b: Vec<u8> = vec![${bytes.join(', ')}];`)
      lines.push(`    ${v}_b.push(${int(65, 90)}u8);`)
      lines.push(`    ${v}_b.sort_unstable();`)
      lines.push(`    ${v}_b.dedup();`)
      lines.push(`    print_u32(${v}_b.iter().map(|&b| b as u32).sum::<u32>());`)
    },
    // 1: Vec<f64> & Vec<Item> (stresses typed arrays and SoA product lanes)
    () => {
      const s = name()
      lines.push(`    let mut ${s}_f: Vec<f64> = Vec::with_capacity(8);`)
      for (let i = 0; i < int(4, 7); i += 1) {
        lines.push(`    ${s}_f.push(${int(-50, 50)} as f64 * 0.25f64);`)
      }
      lines.push(`    let mut ${s}_fsum = 0.0f64;`)
      lines.push(`    for &x in &${s}_f { ${s}_fsum += x; }`)
      lines.push(`    print_f64(${s}_fsum);`)
      lines.push(`    let mut ${s}_items: Vec<Item> = Vec::new();`)
      for (let i = 0; i < int(4, 7); i += 1) {
        lines.push(`    ${s}_items.push(Item { id: ${int(-30, 30)}, weight: ${int(1, 50)}u32, score: ${int(-20, 20)} as f64 * 0.5f64 });`)
      }
      lines.push(`    ${s}_items.retain(|it| it.weight > 5);`)
      lines.push(`    if let Some(last) = ${s}_items.pop() {`)
      lines.push(`        print_i32(last.id);`)
      lines.push(`        print_u32(last.weight);`)
      lines.push(`        print_f64(last.score);`)
      lines.push(`    }`)
      lines.push(`    let mut ${s}_id_acc = 0i32;`)
      lines.push(`    let mut ${s}_w_acc = 0u32;`)
      lines.push(`    for it in &${s}_items {`)
      lines.push(`        ${s}_id_acc = ${s}_id_acc.wrapping_add(it.id);`)
      lines.push(`        ${s}_w_acc = ${s}_w_acc.wrapping_add(it.weight);`)
      lines.push(`    }`)
      lines.push(`    print_i32(${s}_id_acc);`)
      lines.push(`    print_u32(${s}_w_acc);`)
    },
    // 2: String + format!
    () => {
      const st = name()
      const a = int(-200, 200)
      const b = int(0, 65535)
      lines.push(`    let mut ${st} = String::from("k=");`)
      lines.push(`    ${st}.push_str(&format!("{}:{:04x}:{:>5}", ${a}i32, ${b}u32, "hi"));`)
      lines.push(`    ${st}.push('!');`)
      lines.push(`    print_str(&${st});`)
      lines.push(`    print_usize(${st}.len());`)
      lines.push(`    if let Some(ch) = ${st}.pop() { print_u32(ch as u32); }`)
      lines.push(`    let ${st}_rep = ${st}.replace(":", "|");`)
      lines.push(`    print_str(&${st}_rep);`)
    },
    // 3: Box<T>, Box<[T]>, Box<dyn Eval>, Rc<T>
    () => {
      const bx = name()
      lines.push(`    let ${bx}_item = Box::new(Item { id: ${int(-50, 50)}, weight: ${int(1, 99)}u32, score: 1.5f64 });`)
      lines.push(`    print_i32(${bx}_item.eval());`)
      lines.push(`    let ${bx}_dyn: Box<dyn Eval> = ${bx}_item;`)
      lines.push(`    print_i32(${bx}_dyn.eval());`)
      lines.push(`    let ${bx}_slice: Box<[i32]> = vec![${int(-10, 10)}, ${int(-10, 10)}, ${int(-10, 10)}, ${int(-10, 10)}].into_boxed_slice();`)
      lines.push(`    print_usize(${bx}_slice.len());`)
      lines.push(`    print_i32(${bx}_slice.iter().copied().fold(0i32, |a, b| a.wrapping_add(b)));`)
      lines.push(`    let ${bx}_rc1 = Rc::new(Item { id: ${int(-40, 40)}, weight: ${int(1, 30)}u32, score: -2.5f64 });`)
      lines.push(`    let ${bx}_rc2 = Rc::clone(&${bx}_rc1);`)
      lines.push(`    print_usize(Rc::strong_count(&${bx}_rc1));`)
      lines.push(`    print_i32(${bx}_rc2.id.wrapping_add(${bx}_rc1.weight as i32));`)
      lines.push(`    drop(${bx}_rc2);`)
      lines.push(`    print_usize(Rc::strong_count(&${bx}_rc1));`)
      lines.push(`    print_bool(Rc::try_unwrap(${bx}_rc1).is_ok());`)
    },
    // 4: BTreeMap<i32, i32>
    () => {
      const bm = name()
      lines.push(`    let mut ${bm}: BTreeMap<i32, i32> = BTreeMap::new();`)
      const keys = Array.from({ length: int(6, 10) }, () => int(-12, 12))
      for (const k of keys) {
        lines.push(`    ${bm}.insert(${k}i32, ${int(-100, 100)}i32);`)
      }
      const probe = pick(keys)
      lines.push(`    if let Some(val) = ${bm}.get_mut(&${probe}i32) { *val = val.wrapping_add(7); }`)
      lines.push(`    *${bm}.entry(${int(-15, 15)}i32).or_insert(${int(1, 50)}i32) += 3;`)
      lines.push(`    print_bool(${bm}.contains_key(&${probe}i32));`)
      lines.push(`    print_i32(${bm}.remove(&${keys[0]}i32).unwrap_or(-1));`)
      lines.push(`    print_usize(${bm}.len());`)
      lines.push(`    for (&k, &v) in &${bm} {`)
      lines.push(`        print_i32(k);`)
      lines.push(`        print_i32(v);`)
      lines.push(`    }`)
    },
  ]

  // Run all 5 core collection recipes once, plus 1-2 extra picks.
  for (let i = 0; i < recipes.length; i += 1) recipes[i]()
  for (let i = 0; i < int(1, 2); i += 1) pick(recipes)()

  return { body: lines.join('\n'), items, abort: false, usesAlloc: true }
}

// ---------------------------------------------------------------------------- family e

function familyE() {
  const items = `
use core::cell::{Cell, RefCell};
use core::hint::black_box;
use core::num::{NonZeroI32, NonZeroU32};

#[derive(Clone, Copy, PartialEq)]
struct Record {
    a: i32,
    b: i32,
    c: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Idle(i32),
    Active { count: u32, delta: i32 },
}

struct TableEntry {
    id: NonZeroU32,
    tag: i32,
    next: Option<&'static TableEntry>,
}

static ENTRY_TAIL: TableEntry = TableEntry {
    id: NonZeroU32::new(3).unwrap(),
    tag: ${int(-50, 50)},
    next: None,
};

static ENTRY_MID: TableEntry = TableEntry {
    id: NonZeroU32::new(2).unwrap(),
    tag: ${int(-50, 50)},
    next: Some(&ENTRY_TAIL),
};

static ENTRY_HEAD: TableEntry = TableEntry {
    id: NonZeroU32::new(1).unwrap(),
    tag: ${int(-50, 50)},
    next: Some(&ENTRY_MID),
};

const NICHE_TABLE: [Option<NonZeroI32>; 4] = [
    NonZeroI32::new(${int(1, 40)}),
    None,
    NonZeroI32::new(${int(-40, -1)}),
    NonZeroI32::new(${int(41, 99)}),
];

#[inline(never)]
fn bump_record(rec: &mut Record, dx: i32, dy: u32) -> i32 {
    rec.a = rec.a.wrapping_add(dx);
    rec.b = rec.b.wrapping_sub(dx.wrapping_mul(2));
    rec.c = rec.c.wrapping_add(dy);
    rec.a.wrapping_add(rec.b)
}

#[inline(never)]
fn swap_fields(x: &mut i32, y: &mut i32) {
    let tmp = *x;
    *x = (*y).wrapping_add(1);
    *y = tmp.wrapping_sub(1);
}

#[inline(never)]
fn mutate_array(arr: &mut [i32; 4], idx: usize, val: i32) {
    arr[idx] = arr[idx].wrapping_add(val);
}

#[inline(never)]
fn flip_state(st: &mut State, step: i32) {
    *st = match *st {
        State::Idle(v) => State::Active { count: (v.unsigned_abs() & 31) + 1, delta: step },
        State::Active { count, delta } => State::Idle((count as i32).wrapping_add(delta).wrapping_add(step)),
    };
}
`
  const lines = []
  let next = 0
  const name = () => `e${next++}`

  const recipes = [
    // 0: Copy mutation isolation checks (struct, array, enum)
    () => {
      const iso = name()
      lines.push(`    let ${iso}_orig = black_box(Record { a: ${int(-50, 50)}, b: ${int(-50, 50)}, c: ${int(1, 90)}u32 });`)
      lines.push(`    let ${iso}_snap = ${iso}_orig;`)
      lines.push(`    let mut ${iso}_copy = ${iso}_orig;`)
      lines.push(`    print_i32(bump_record(&mut ${iso}_copy, ${int(-15, 15)}, ${int(1, 15)}u32));`)
      lines.push(`    print_bool(${iso}_orig == ${iso}_snap);`)
      lines.push(`    print_i32(${iso}_orig.a);`)
      lines.push(`    print_i32(${iso}_orig.b);`)
      lines.push(`    print_i32(${iso}_copy.a);`)
      lines.push(`    print_i32(${iso}_copy.b);`)
      lines.push(`    let ${iso}_arr_orig = black_box([${int(-20, 20)}i32, ${int(-20, 20)}, ${int(-20, 20)}, ${int(-20, 20)}]);`)
      lines.push(`    let mut ${iso}_arr_copy = ${iso}_arr_orig;`)
      lines.push(`    mutate_array(&mut ${iso}_arr_copy, ${int(0, 3)}, ${int(5, 25)});`)
      lines.push(`    print_i32(${iso}_arr_orig[0].wrapping_add(${iso}_arr_orig[1]).wrapping_add(${iso}_arr_orig[2]).wrapping_add(${iso}_arr_orig[3]));`)
      lines.push(`    print_i32(${iso}_arr_copy[0].wrapping_add(${iso}_arr_copy[1]).wrapping_add(${iso}_arr_copy[2]).wrapping_add(${iso}_arr_copy[3]));`)
      lines.push(`    let ${iso}_st_orig = black_box(State::Idle(${int(-30, 30)}));`)
      lines.push(`    let mut ${iso}_st_copy = ${iso}_st_orig;`)
      lines.push(`    flip_state(&mut ${iso}_st_copy, ${int(-10, 10)});`)
      lines.push(`    print_bool(${iso}_st_orig == State::Idle(${iso}_st_orig_val(${iso}_st_orig)));`)
      lines.push(`    print_bool(${iso}_st_orig == ${iso}_st_copy);`)
    },
    // 1: Disjoint &mut field reborrows + chained reborrow
    () => {
      const rb = name()
      lines.push(`    let mut ${rb} = Record { a: ${int(-60, 60)}, b: ${int(-60, 60)}, c: ${int(0, 60)}u32 };`)
      lines.push(`    {`)
      lines.push(`        let r = &mut ${rb};`)
      lines.push(`        swap_fields(&mut r.a, &mut r.b);`)
      lines.push(`        let r2 = &mut *r;`)
      lines.push(`        print_i32(bump_record(r2, ${int(-20, 20)}, ${int(1, 20)}u32));`)
      lines.push(`    }`)
      lines.push(`    print_i32(${rb}.a);`)
      lines.push(`    print_i32(${rb}.b);`)
      lines.push(`    print_u32(${rb}.c);`)
    },
    // 2: Cell and RefCell interior mutability
    () => {
      const cl = name()
      lines.push(`    let ${cl}_c1 = Cell::new(${int(-50, 50)}i32);`)
      lines.push(`    let ${cl}_c2 = Cell::new(${int(-50, 50)}i32);`)
      lines.push(`    print_i32(${cl}_c1.replace(${int(-50, 50)}));`)
      lines.push(`    ${cl}_c1.swap(&${cl}_c2);`)
      lines.push(`    print_i32(${cl}_c1.get());`)
      lines.push(`    print_i32(${cl}_c2.get());`)
      lines.push(`    let ${cl}_rc = RefCell::new(Record { a: ${int(-40, 40)}, b: ${int(-40, 40)}, c: ${int(1, 40)}u32 });`)
      lines.push(`    {`)
      lines.push(`        let mut b = ${cl}_rc.borrow_mut();`)
      lines.push(`        bump_record(&mut b, ${int(-10, 10)}, ${int(1, 10)}u32);`)
      lines.push(`    }`)
      lines.push(`    {`)
      lines.push(`        let r1 = ${cl}_rc.borrow();`)
      lines.push(`        let r2 = ${cl}_rc.borrow();`)
      lines.push(`        print_i32(r1.a.wrapping_add(r2.b));`)
      lines.push(`    }`)
      lines.push(`    let ${cl}_old = ${cl}_rc.replace(Record { a: 7, b: -7, c: 9 });`)
      lines.push(`    print_i32(${cl}_old.a);`)
      lines.push(`    print_u32(${cl}_rc.borrow().c);`)
    },
    // 3: static/const tables with Option<&T> niches and NonZero + black_box
    () => {
      const st = name()
      lines.push(`    let mut ${st}_cur: Option<&'static TableEntry> = black_box(Some(&ENTRY_HEAD));`)
      lines.push(`    let mut ${st}_sum = 0i32;`)
      lines.push(`    while let Some(entry) = ${st}_cur {`)
      lines.push(`        print_u32(entry.id.get());`)
      lines.push(`        ${st}_sum = ${st}_sum.wrapping_add(entry.tag.wrapping_mul(entry.id.get() as i32));`)
      lines.push(`        ${st}_cur = black_box(entry.next);`)
      lines.push(`    }`)
      lines.push(`    print_i32(${st}_sum);`)
      lines.push(`    for slot in black_box(NICHE_TABLE) {`)
      lines.push(`        match slot {`)
      lines.push(`            Some(nz) => print_i32(nz.get()),`)
      lines.push(`            None => print_str("niche-none"),`)
      lines.push(`        }`)
      lines.push(`    }`)
    },
  ]

  // Helper used by recipe 0 without needing extra variable interpolation inside State::Idle.
  const extraHelper = `
fn state_idle_val(st: State) -> i32 {
    match st {
        State::Idle(v) => v,
        State::Active { delta, .. } => delta,
    }
}
`
  // Fix recipe 0 helper call name cleanly.
  const fullItems = items + extraHelper
  for (let i = 0; i < recipes.length; i += 1) recipes[i]()
  for (let i = 0; i < int(1, 3); i += 1) pick(recipes)()

  const body = lines.join('\n').replace(/e\d+_st_orig_val/g, 'state_idle_val')
  return { body, items: fullItems, abort: false, usesAlloc: false }
}

// ---------------------------------------------------------------------------- family f

function familyF() {
  const items = `
use alloc::string::String;

#[allow(non_snake_case)]
#[derive(Clone, Copy, PartialEq)]
struct JsTrap {
    constructor: i32,
    toString: i32,
    valueOf: i32,
    hasOwnProperty: i32,
    then: i32,
    TAG: i32,
}

#[allow(non_snake_case)]
#[inline(never)]
fn mutate_trap(t: &mut JsTrap, d: i32) -> i32 {
    t.constructor = t.constructor.wrapping_add(d);
    t.toString = t.toString.wrapping_sub(d);
    t.valueOf = t.valueOf.wrapping_mul(3).wrapping_add(1);
    t.hasOwnProperty ^= d;
    t.then = t.then.wrapping_add(t.TAG);
    t.TAG = t.TAG.wrapping_add(1);
    t.constructor
        .wrapping_add(t.toString)
        .wrapping_add(t.valueOf)
        .wrapping_add(t.hasOwnProperty)
        .wrapping_add(t.then)
        .wrapping_add(t.TAG)
}

#[derive(Clone, Copy)]
struct WideCounters {
    hi: i64,
    lo: u64,
    acc: i64,
}

#[inline(never)]
fn step_wide(c: &mut WideCounters, rounds: u32) {
    let mut i = 0u32;
    while i < rounds {
        c.hi = c.hi.wrapping_add((i as i64).wrapping_mul(9007199254740993i64));
        c.lo = c.lo.rotate_left((i & 15) + 1) ^ 0x1234_5678_9abc_def0u64;
        c.acc = c.acc.wrapping_sub(c.hi ^ (c.lo as i64));
        i += 1;
    }
}

fn classify_byte(b: u8) -> i32 {
    match b {
        b',' => 1,
        b';' => 2,
        b'\\n' => 3,
        b'\\t' => 4,
        b' ' => 5,
        b':' => 6,
        b'=' => 7,
        b'#' => 8,
        b'0'..=b'9' => 10 + ((b - b'0') as i32),
        b'a'..=b'z' => 30 + ((b - b'a') as i32),
        b'A'..=b'Z' => 60 + ((b - b'A') as i32),
        _ => -1,
    }
}

fn sparse_switch(k: i32) -> i32 {
    match k {
        -1000 => 11,
        -7 => 22,
        0 => 33,
        1 => 44,
        2 => 55,
        3 => 66,
        4 => 77,
        5 => 88,
        42 => 99,
        1024 => 111,
        _ => k.wrapping_mul(2),
    }
}
`
  const lines = []
  let next = 0
  const name = () => `f${next++}`

  const utf8Samples = [
    '  Hello, café! 🦀🚀  ',
    '\\tпривет:αβγ;日本語=€→★#42\\n',
    'Rust🦀2026 𝄞𐐷 ASCII_MiXeD!',
    '  αβγ,日本語;🦀🚀𝄞=Ok  ',
    'a,b;c:d=e#f 🦀 café 日本語',
  ]

  const recipes = [
    // 0: JS-reserved & prototype-adjacent field names
    () => {
      const jt = name()
      lines.push(`    let mut ${jt} = JsTrap {`)
      lines.push(`        constructor: ${int(-50, 50)},`)
      lines.push(`        toString: ${int(-50, 50)},`)
      lines.push(`        valueOf: ${int(-50, 50)},`)
      lines.push(`        hasOwnProperty: ${int(-50, 50)},`)
      lines.push(`        then: ${int(-50, 50)},`)
      lines.push(`        TAG: ${int(-50, 50)},`)
      lines.push(`    };`)
      lines.push(`    let ${jt}_copy = ${jt};`)
      lines.push(`    print_i32(mutate_trap(&mut ${jt}, ${int(-20, 20)}));`)
      lines.push(`    print_bool(${jt} == ${jt}_copy);`)
      lines.push(`    print_i32(${jt}_copy.constructor.wrapping_add(${jt}_copy.TAG));`)
      lines.push(`    let ${jt}_opt: Option<JsTrap> = Some(${jt});`)
      lines.push(`    if let Some(inner) = ${jt}_opt {`)
      lines.push(`        print_i32(inner.toString.wrapping_add(inner.valueOf).wrapping_add(inner.then));`)
      lines.push(`    }`)
    },
    // 1: i64/u64 fields inside structs & tight loops (i64_pair & bigint_props)
    () => {
      const wc = name()
      const hi = pick(['0', '1', '-1', '9007199254740993', '-9007199254740993', String(int(-50000, 50000))])
      const lo = pick(['0', '1', '9007199254740993', '18446744073709551615', String(int(0, 50000))])
      lines.push(`    let mut ${wc} = WideCounters { hi: ${hi}i64, lo: ${lo}u64, acc: ${int(-1000, 1000)}i64 };`)
      lines.push(`    step_wide(&mut ${wc}, ${int(4, 12)}u32);`)
      lines.push(`    print_i64(${wc}.hi);`)
      lines.push(`    print_u64(${wc}.lo);`)
      lines.push(`    print_i64(${wc}.acc);`)
      lines.push(`    let ${wc}_arr = [${wc}.hi, ${wc}.acc, (${wc}.lo as i64).wrapping_neg()];`)
      lines.push(`    let mut ${wc}_fold = 0i64;`)
      lines.push(`    for &x in &${wc}_arr {`)
      lines.push(`        ${wc}_fold = ${wc}_fold.wrapping_add(x.wrapping_shr(3)).wrapping_add(x & 0xff);`)
      lines.push(`    }`)
      lines.push(`    print_i64(${wc}_fold);`)
    },
    // 2: Multi-arm match & byte-delimiter scans (switch_lut & tag_encode)
    () => {
      const sw = name()
      const text = pick([
        'alpha,beta;gamma:12=34#end\\n\\tok ',
        '09:42=foo,bar;baz#99\\t\\n',
        'A1,B2;C3:D4=E5#F6 ',
      ])
      lines.push(`    let ${sw}_bytes = b"${text}";`)
      lines.push(`    let mut ${sw}_sum = 0i32;`)
      lines.push(`    for &b in ${sw}_bytes {`)
      lines.push(`        ${sw}_sum = ${sw}_sum.wrapping_add(classify_byte(b));`)
      lines.push(`    }`)
      lines.push(`    print_i32(${sw}_sum);`)
      const probes = [-1000, -7, 0, 1, 2, 3, 4, 5, 42, 1024, int(-20, 20)]
      lines.push(`    for k in [${probes.map((x) => `${x}i32`).join(', ')}] {`)
      lines.push(`        print_i32(sparse_switch(k));`)
      lines.push(`    }`)
    },
    // 3: UTF-8 &str / String with ASCII + multi-byte + 4-byte astral Unicode
    () => {
      const u = name()
      const sample = pick(utf8Samples)
      lines.push(`    let ${u}_raw: &str = "${sample}";`)
      lines.push(`    print_str(${u}_raw);`)
      lines.push(`    print_usize(${u}_raw.len());`)
      lines.push(`    print_usize(${u}_raw.chars().count());`)
      lines.push(`    print_str(${u}_raw.trim());`)
      lines.push(`    print_bool(${u}_raw.contains("🦀"));`)
      lines.push(`    print_bool(${u}_raw.contains("café"));`)
      lines.push(`    let mut ${u}_cp_sum = 0u32;`)
      lines.push(`    let mut ${u}_idx_sum = 0usize;`)
      lines.push(`    let mut ${u}_cut_start = 0usize;`)
      lines.push(`    let mut ${u}_cut_end = ${u}_raw.len();`)
      lines.push(`    let mut ${u}_seen = 0usize;`)
      lines.push(`    for (idx, ch) in ${u}_raw.char_indices() {`)
      lines.push(`        ${u}_cp_sum = ${u}_cp_sum.wrapping_add(ch as u32);`)
      lines.push(`        ${u}_idx_sum = ${u}_idx_sum.wrapping_add(idx);`)
      lines.push(`        if ${u}_seen == 2 { ${u}_cut_start = idx; }`)
      lines.push(`        if ${u}_seen == 6 { ${u}_cut_end = idx; }`)
      lines.push(`        ${u}_seen += 1;`)
      lines.push(`    }`)
      lines.push(`    print_u32(${u}_cp_sum);`)
      lines.push(`    print_usize(${u}_idx_sum);`)
      lines.push(`    if ${u}_cut_start <= ${u}_cut_end {`)
      lines.push(`        print_str(&${u}_raw[${u}_cut_start..${u}_cut_end]);`)
      lines.push(`    }`)
      lines.push(`    let mut ${u}_bsum = 0u32;`)
      lines.push(`    for b in ${u}_raw.bytes() { ${u}_bsum = ${u}_bsum.wrapping_add(b as u32); }`)
      lines.push(`    print_u32(${u}_bsum);`)
      lines.push(`    for part in ${u}_raw.trim().split(|c| c == ',' || c == ';' || c == ' ') {`)
      lines.push(`        print_str(part);`)
      lines.push(`    }`)
      lines.push(`    let mut ${u}_owned = String::from(${u}_raw.trim());`)
      lines.push(`    ${u}_owned.push('🦀');`)
      lines.push(`    ${u}_owned.push_str("-AbC");`)
      lines.push(`    print_str(&${u}_owned.to_ascii_lowercase());`)
      lines.push(`    print_str(&${u}_owned.to_ascii_uppercase());`)
    },
  ]

  // Exercise all 4 JS-engine / representation stress recipes on every family-f seed.
  for (let i = 0; i < recipes.length; i += 1) recipes[i]()
  pick(recipes)()

  return { body: lines.join('\n'), items, abort: false, usesAlloc: true }
}

// ---------------------------------------------------------------------------- assembly

const generators = {
  a: familyA,
  b: familyB,
  c: familyC,
  d: familyD,
  e: familyE,
  f: familyF,
}

const generated = generators[family]()

const wasmBumpAllocator = generated.usesAlloc
  ? `
#[cfg(fuzz_wasm_alloc)]
mod __fuzz_wasm_alloc {
    use core::alloc::{GlobalAlloc, Layout};
    use core::cell::UnsafeCell;
    use core::ptr;
    use core::sync::atomic::{AtomicUsize, Ordering};

    const HEAP_SIZE: usize = 4 * 1024 * 1024;

    #[repr(align(16))]
    struct Heap(UnsafeCell<[u8; HEAP_SIZE]>);
    unsafe impl Sync for Heap {}

    static HEAP: Heap = Heap(UnsafeCell::new([0; HEAP_SIZE]));
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Bump;

    unsafe impl GlobalAlloc for Bump {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let align = layout.align().max(1);
            let size = layout.size();
            let mut cur = NEXT.load(Ordering::Relaxed);
            loop {
                let aligned = (cur + align - 1) & !(align - 1);
                let end = match aligned.checked_add(size) {
                    Some(e) if e <= HEAP_SIZE => e,
                    _ => return ptr::null_mut(),
                };
                match NEXT.compare_exchange_weak(cur, end, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => return unsafe { (*HEAP.0.get()).as_mut_ptr().add(aligned) },
                    Err(v) => cur = v,
                }
            }
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
    }

    #[global_allocator]
    static ALLOC: Bump = Bump;
}
`
  : ''

const program = `//! Generated by scripts/fuzz/gen.mjs — seed ${seed}, family ${family}. Do not edit by hand.
//!
//! Regenerate with: node scripts/fuzz/gen.mjs ${seed} --family ${family}${includeLargeEnums ? ' --include-large-enums' : ''}
${generated.abort ? '//!\n//! This program is REQUIRED to abort: it ends in a `#[track_caller]` panic.\n' : ''}
#![no_std]
#![no_main]
#![allow(dead_code, unused_variables, unused_mut, clippy::all)]
${generated.usesAlloc ? '\nextern crate alloc;\n' : ''}
unsafe extern "C" {
    fn js_log_i32(x: i32);
    fn js_log_i64(x: i64);
    fn js_log_f64(x: f64);
    fn js_log_bool(x: bool);
    fn js_log_str(s: &str);
    fn js_abort(msg: &str) -> !;
}

fn print_i32(x: i32) {
    unsafe { js_log_i32(x) }
}

/// Every \`u32\` is exact in a double, and \`js_log_i32\` would print \`u32::MAX\` as \`-1\`.
fn print_u32(x: u32) {
    unsafe { js_log_f64(x as f64) }
}

fn print_usize(x: usize) {
    unsafe { js_log_f64(x as f64) }
}

fn print_i64(x: i64) {
    unsafe { js_log_i64(x) }
}

/// A \`u64\` does not fit a double, so it goes out as the \`i64\` with the same bits.
fn print_u64(x: u64) {
    unsafe { js_log_i64(x as i64) }
}

/// A \`u128\` goes out as its two halves, high first, each as the \`i64\` with the same bits.
fn print_u128(x: u128) {
    print_i64((x >> 64) as u64 as i64);
    print_i64(x as u64 as i64);
}

fn print_i128(x: i128) {
    print_u128(x as u128);
}

fn print_f64(x: f64) {
    unsafe { js_log_f64(x) }
}

fn print_bool(x: bool) {
    unsafe { js_log_bool(x) }
}

fn print_str(s: &str) {
    unsafe { js_log_str(s) }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(location) = info.location() {
        print_str(location.file());
        print_i32(location.line() as i32);
    }
    unsafe { js_abort("panic") }
}
${generated.items}
#[unsafe(no_mangle)]
pub fn rust_entry() {
${generated.body}
}
${wasmBumpAllocator}`

process.stdout.write(program)

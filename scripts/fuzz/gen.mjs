#!/usr/bin/env node
//
// The wasm32-LLVM-vs-JS fuzzer's program generator.
//
//   node scripts/fuzz/gen.mjs <seed> [--family a|b|c] > prog.rs
//
// Emits one self-contained `#![no_std]` `#![no_main]` crate in the shape of an
// `examples/core-tests/` fixture: the same `extern "C"` print helpers, the same panic handler that
// prints a location and calls `js_abort`. That shape is the whole point — the SAME source has to
// compile through the ordinary LLVM backend for `wasm32-unknown-unknown` and through
// rustc_codegen_js, so that the two runs can be compared line for line.
//
// EVERY GENERATED PROGRAM IS DEFINED BEHAVIOUR
// -------------------------------------------
// A difference between two backends only means something if the program has one meaning. So the
// generator never emits arithmetic overflow that is not `wrapping_`/`checked_`/`saturating_`,
// never shifts by an unmasked amount (`wrapping_shl`/`wrapping_shr` only), never indexes out of
// bounds, never divides by a value that could be zero outside `checked_div`, and never reads an
// uninitialized or moved-from place. Panics are emitted on purpose in exactly one place: the
// family `c` finale, where the program must abort on both sides after printing the same lines.
//
// THE THREE SEED FAMILIES
// -----------------------
//   a  integer arithmetic: wrapping/checked/saturating, i64 and u128, shifts, rotates, bit ops,
//      integer and float casts, comparisons.
//   b  aggregates: structs, tuples, enums, Option/Result, arrays, slices, iterator chains,
//      closures, and one `dyn Trait` call.
//   c  destructors: `Drop` guards that print, nested scopes, moves, an early return, and a
//      `#[track_caller]` panic the program is required to abort on.
//
// The family is chosen from the seed unless `--family` names one, so a run of N seeds covers all
// three. Output is a pure function of (seed, family): the same seed always produces the same file,
// which is what makes a reported mismatch reproducible from its seed alone.

const argv = process.argv.slice(2)
const seed = Number(argv[0])
if (!Number.isInteger(seed) || seed < 0) {
  console.error('usage: gen.mjs <seed> [--family a|b|c]')
  process.exit(2)
}
const familyFlag = argv.includes('--family') ? argv[argv.indexOf('--family') + 1] : null
if (familyFlag && !['a', 'b', 'c'].includes(familyFlag)) {
  console.error(`gen.mjs: --family must be a, b or c (got ${familyFlag})`)
  process.exit(2)
}
const family = familyFlag ?? ['a', 'b', 'c'][seed % 3]

// mulberry32: 32 bits of state, no dependencies, identical everywhere. The generator only needs
// reproducibility, not statistical quality.
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

// ---------------------------------------------------------------------------- values

// Interesting constants per type, spelled so that the literal itself never overflows.
const CONSTANTS = {
  i32: () => pick(['0', '1', '-1', '2', '-7', '31', '255', '65535', '1073741823', 'i32::MIN', 'i32::MAX', String(int(-1000, 1000))]),
  u32: () => pick(['0', '1', '2', '255', '65535', '4294967295', 'u32::MAX', String(int(0, 100000))]),
  i64: () => pick(['0', '1', '-1', '9007199254740993', '-9007199254740993', 'i64::MIN', 'i64::MAX', `${int(-1000000, 1000000)}`]),
  u64: () => pick(['0', '1', '18446744073709551615', 'u64::MAX', '9007199254740993', String(int(0, 1000000))]),
  u128: () => pick(['0', '1', '340282366920938463463374607431768211455', 'u128::MAX', '18446744073709551617', String(int(0, 1000000))]),
}
const BITS = { i32: 32, u32: 32, i64: 64, u64: 64, u128: 128 }
const TYPES = Object.keys(CONSTANTS)

// A negative literal is parenthesized: `-1i64.count_ones()` parses as `-(1i64.count_ones())`, and
// the generator must never depend on the reader knowing that.
const literal = (type) => {
  const value = CONSTANTS[type]()
  const spelled = /^[a-z]/.test(value) ? value : `${value}${type}`
  return spelled.startsWith('-') ? `(${spelled})` : spelled
}

const PRINTERS = {
  i32: (expression) => `print_i32(${expression});`,
  u32: (expression) => `print_u32(${expression});`,
  i64: (expression) => `print_i64(${expression});`,
  u64: (expression) => `print_u64(${expression});`,
  u128: (expression) => `print_u128(${expression});`,
}

// ---------------------------------------------------------------------------- family a

function familyA() {
  const lines = []
  const live = Object.fromEntries(TYPES.map((type) => [type, []]))
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
    if (chance(0.7)) lines.push(`    ${PRINTERS[type](binding)}`)
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

  for (let index = 0; index < int(6, 14); index += 1) {
    // One type per statement: the operand pool, the operation and the binding all agree, which is
    // what keeps every generated program type-correct without a type checker in the generator.
    const type = index < TYPES.length ? TYPES[index] : pick(TYPES)
    bind(type, arithmetic(type))
  }

  // Casts, both directions, including the float casts Rust defines as saturating.
  for (let index = 0; index < int(3, 6); index += 1) {
    const from = pick(TYPES)
    const to = pick(TYPES)
    bind(to, `${operand(from)} as ${to}`)
  }
  for (let index = 0; index < int(2, 4); index += 1) {
    const from = pick(TYPES)
    lines.push(`    print_f64(${operand(from)} as f64);`)
    const scale = pick(['0.5', '1.5', '-3.25', '1e18', '-1e18', 'f64::INFINITY', 'f64::NEG_INFINITY'])
    lines.push(`    print_i32((${operand(from)} as f64 * ${scale}) as i32);`)
    lines.push(`    print_i64((${operand(from)} as f64 * ${scale}) as i64);`)
  }

  // Comparisons and the boolean algebra around them.
  for (let index = 0; index < int(3, 6); index += 1) {
    const type = pick(TYPES)
    const a = operand(type)
    const b = operand(type)
    lines.push(`    print_bool(${a} ${pick(['<', '<=', '>', '>=', '==', '!='])} ${b});`)
  }

  return { body: lines.join('\n'), items: '', abort: false }
}

// ---------------------------------------------------------------------------- family b

function familyB() {
  const items = `
#[derive(Clone, Copy, PartialEq)]
struct Point {
    x: i32,
    y: i32,
}

struct Wrapper(i32, u32);

#[derive(Clone, Copy)]
enum Shape {
    Empty,
    Single(i32),
    Pair(i32, i32),
}

trait Describe {
    fn value(&self) -> i32;
}

impl Describe for Point {
    fn value(&self) -> i32 {
        self.x.wrapping_mul(${int(2, 9)}).wrapping_add(self.y)
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
`
  const lines = []
  let next = 0
  const name = () => `b${next++}`

  const recipes = [
    () => {
      const point = name()
      lines.push(`    let ${point} = Point { x: ${int(-100, 100)}, y: ${int(-100, 100)} };`)
      lines.push(`    print_i32(${point}.x.wrapping_add(${point}.y));`)
      lines.push(`    print_bool(${point} == Point { x: ${int(-3, 3)}, y: ${int(-3, 3)} });`)
      lines.push(`    let ${point}_dyn: &dyn Describe = &${point};`)
      lines.push(`    print_i32(${point}_dyn.value());`)
    },
    () => {
      const wrapper = name()
      lines.push(`    let ${wrapper} = Wrapper(${int(-500, 500)}, ${int(0, 500)}u32);`)
      lines.push(`    let ${wrapper}_dyn: &dyn Describe = &${wrapper};`)
      lines.push(`    print_i32(${wrapper}_dyn.value());`)
      lines.push(`    print_u32(${wrapper}.1.wrapping_add(${int(0, 40)}));`)
    },
    () => {
      const shape = name()
      lines.push(`    let ${shape} = ${pick([`Shape::Empty`, `Shape::Single(${int(-40, 40)})`, `Shape::Pair(${int(-40, 40)}, ${int(-40, 40)})`])};`)
      lines.push(`    print_i32(area(${shape}));`)
      lines.push(`    print_i32(match ${shape} {`)
      lines.push(`        Shape::Empty => ${int(-9, 9)},`)
      lines.push(`        Shape::Single(value) => value,`)
      lines.push(`        Shape::Pair(a, b) => a.wrapping_add(b),`)
      lines.push(`    });`)
    },
    () => {
      const option = name()
      lines.push(`    let ${option}: Option<i32> = ${chance(0.5) ? `Some(${int(-60, 60)})` : 'None'};`)
      lines.push(`    print_bool(${option}.is_some());`)
      lines.push(`    print_i32(${option}.map(|value| value.wrapping_mul(${int(2, 7)})).unwrap_or(${int(-9, 9)}));`)
      lines.push(`    print_i32(${option}.and_then(halve).unwrap_or(${int(-9, 9)}));`)
      lines.push(`    print_i32(chain(${int(-32, 32)}).unwrap_or(${int(-9, 9)}));`)
    },
    () => {
      const result = name()
      lines.push(`    let ${result}: Result<i32, i32> = ${chance(0.5) ? `Ok(${int(-60, 60)})` : `Err(${int(-60, 60)})`};`)
      lines.push(`    print_bool(${result}.is_ok());`)
      lines.push(`    print_i32(${result}.unwrap_or_else(|error| error.wrapping_sub(${int(1, 9)})));`)
      lines.push(`    print_i32(${result}.map(|value| value.wrapping_add(1)).unwrap_or(${int(-9, 9)}));`)
    },
    () => {
      const array = name()
      const values = Array.from({ length: int(4, 8) }, () => int(-30, 30))
      lines.push(`    let ${array} = [${values.map((value) => `${value}i32`).join(', ')}];`)
      const start = int(0, values.length - 2)
      const end = int(start + 1, values.length)
      lines.push(`    let ${array}_slice = &${array}[${start}..${end}];`)
      lines.push(`    print_usize(${array}_slice.len());`)
      lines.push(`    print_i32(${array}_slice.iter().copied().fold(0i32, |a, b| a.wrapping_add(b)));`)
      lines.push(`    print_i32(*${array}_slice.iter().max().unwrap_or(&0));`)
      lines.push(`    print_i32(${array}.iter().copied().filter(|value| value % ${int(2, 5)} == 0).count() as i32);`)
      lines.push(`    for (index, value) in ${array}_slice.iter().enumerate() {`)
      lines.push(`        print_i32(index as i32);`)
      lines.push(`        print_i32(*value);`)
      lines.push(`    }`)
    },
    () => {
      const range = name()
      lines.push(`    print_i32((0..${int(3, 9)}i32).map(|index| index.wrapping_mul(${int(2, 6)})).filter(|value| value % 2 == 0).fold(0i32, |a, b| a.wrapping_add(b)));`)
      lines.push(`    print_i32((0..${int(3, 9)}i32).rev().take(${int(1, 4)}).sum::<i32>());`)
      lines.push(`    print_i32((0..${int(2, 6)}i32).zip(${int(5, 9)}..${int(10, 14)}i32).map(|(a, b)| a.wrapping_mul(b)).sum::<i32>());`)
      lines.push(`    let mut ${range} = 0i32;`)
      lines.push(`    let mut ${range}_add = |value: i32| { ${range} = ${range}.wrapping_add(value); ${range} };`)
      lines.push(`    print_i32(${range}_add(${int(-20, 20)}));`)
      lines.push(`    print_i32(${range}_add(${int(-20, 20)}));`)
    },
    () => {
      const tuple = name()
      lines.push(`    let ${tuple} = (${int(-40, 40)}i32, ${int(0, 40)}u32, ${chance(0.5)});`)
      lines.push(`    let (${tuple}_a, ${tuple}_b, ${tuple}_c) = ${tuple};`)
      lines.push(`    print_i32(${tuple}_a);`)
      lines.push(`    print_u32(${tuple}_b);`)
      lines.push(`    print_bool(${tuple}_c);`)
      lines.push(`    let ${tuple}_apply = |value: i32| -> i32 { if ${tuple}_c { value.wrapping_mul(${int(2, 5)}) } else { value.wrapping_sub(${int(1, 9)}) } };`)
      lines.push(`    print_i32(${tuple}_apply(${tuple}_a));`)
    },
  ]

  // Every program gets the `dyn` recipe, so the family's one virtual call is never absent.
  recipes[0]()
  for (let index = 0; index < int(3, 6); index += 1) pick(recipes)()

  return { body: lines.join('\n'), items, abort: false }
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

fn consume(guard: Guard) -> i32 {
    let value = guard.0;
    print_str("consume");
    value.wrapping_mul(${int(2, 5)})
    // \`guard\` is dropped here, at the end of the function that took it by value.
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
    () => {
      const guard = name()
      lines.push(`    {`)
      lines.push(`        let _${guard} = Guard(${int(1, 40)});`)
      lines.push(`        let _${guard}_inner = Guard(${int(41, 80)});`)
      lines.push(`        print_str("scope");`)
      lines.push(`    }`)
    },
    () => {
      const pair = name()
      lines.push(`    let ${pair} = Pair { first: Guard(${int(1, 40)}), second: Guard(${int(41, 80)}) };`)
      lines.push(`    print_i32(${pair}.first.0.wrapping_add(${pair}.second.0));`)
      lines.push(`    drop(${pair});`)
    },
    () => {
      lines.push(`    print_i32(consume(Guard(${int(1, 90)})));`)
    },
    () => {
      lines.push(`    print_i32(early(${int(-20, 20)}));`)
    },
    () => {
      const moved = name()
      lines.push(`    let ${moved} = Guard(${int(1, 90)});`)
      lines.push(`    let ${moved}_moved = ${moved};`)
      lines.push(`    print_i32(${moved}_moved.0);`)
    },
    () => {
      const conditional = name()
      lines.push(`    let ${conditional} = if ${chance(0.5)} { Guard(${int(1, 40)}) } else { Guard(${int(41, 80)}) };`)
      lines.push(`    print_i32(${conditional}.0);`)
    },
  ]

  for (let index = 0; index < int(3, 6); index += 1) pick(recipes)()

  // The finale: a `#[track_caller]` panic. Both backends must print the location the handler
  // reads and then end the program with a non-zero status.
  lines.push(`    print_str("about to panic");`)
  lines.push(`    let missing: Option<i32> = None;`)
  lines.push(`    print_i32(demand(missing));`)
  lines.push(`    print_str("unreachable");`)

  return { body: lines.join('\n'), items, abort: true }
}

// ---------------------------------------------------------------------------- assembly

const generated = family === 'a' ? familyA() : family === 'b' ? familyB() : familyC()

const program = `//! Generated by scripts/fuzz/gen.mjs — seed ${seed}, family ${family}. Do not edit by hand.
//!
//! Regenerate with: node scripts/fuzz/gen.mjs ${seed} --family ${family}
${generated.abort ? '//!\n//! This program is REQUIRED to abort: it ends in a \`#[track_caller]\` panic.\n' : ''}
#![no_std]
#![no_main]
#![allow(dead_code, unused_variables, unused_mut, clippy::all)]

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
`

process.stdout.write(program)

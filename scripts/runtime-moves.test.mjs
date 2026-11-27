// Element moves (`__rt.copy`, `__rt.copy_nonoverlapping`, and the `_put` they share): an element
// that is an aggregate held by value is written into the object already in its place, and every
// other element -- a reference to an aggregate (the aggregate's own object), a raw pointer or a
// `Box` (a slot record) -- rebinds the place. Writing a pointer value into the object in place would
// overwrite the pointee: `[&P; 2].swap(0, 1)` copied one `P` over the other. CONTRACT.md, "Element
// offsets".
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'

const source = await readFile(new URL('../runtime/shim.js', import.meta.url), 'utf8')
function runtime() {
  const context = vm.createContext({})
  vm.runInContext(source, context)
  return context.__rt
}
const P = (x) => ({ x, y: x + 1 })
// The clone the compiler hands over for a `P` element (`value::clone_expr`).
const cloneP = (e) => ({ x: e.x, y: e.y })

// `ptr::swap(x, y)` exactly as `core` spells it: through a `MaybeUninit<T>` temporary, which is a
// boxed cell holding `MaybeUninit::uninit()` (`c = []; c[0] = undefined`) because a transparent
// union is direct.
function ptrSwap(rt, x, y, clone) {
  const tmp = [undefined]
  const t = { $b: tmp, $o: 0 }
  rt.copy_nonoverlapping(t, x, 1, clone)
  rt.copy(x, y, 1, clone)
  rt.copy_nonoverlapping(y, t, 1, clone)
}

test('swapping two references to aggregates exchanges the references and leaves both pointees alone', () => {
  const rt = runtime()
  const h = P(1)
  const h2 = P(2)
  const refs = [h, h2]
  ptrSwap(rt, { $b: refs, $o: 0 }, { $b: refs, $o: 1 })
  assert.equal(refs[0], h2)
  assert.equal(refs[1], h)
  assert.deepEqual([h.x, h.y, h2.x, h2.y], [1, 2, 2, 3])
})

test('copying a reference into another buffer rebinds the element instead of overwriting its pointee', () => {
  const rt = runtime()
  const h = P(1)
  const h2 = P(2)
  const dst = [h]
  rt.copy_nonoverlapping({ $b: dst, $o: 0 }, { $b: [h2], $o: 0 }, 1)
  assert.equal(dst[0], h2)
  assert.deepEqual(h, { x: 1, y: 2 })
  const dst2 = [h, h2]
  rt.copy({ $b: dst2, $o: 0 }, { $b: [h2, h], $o: 0 }, 2)
  assert.equal(dst2[0], h2)
  assert.equal(dst2[1], h)
  assert.deepEqual([h.x, h2.x], [1, 2])
})

test('raw pointers and boxes are slot records: a move stores the record and never mutates the old one', () => {
  const rt = runtime()
  const a = { $b: [P(1)], $o: 0 }
  const b = { $b: [P(2)], $o: 0 }
  const raws = [a, b]
  ptrSwap(rt, { $b: raws, $o: 0 }, { $b: raws, $o: 1 })
  assert.equal(raws[0], b)
  assert.equal(raws[1], a)
  assert.equal(a.$b[0].x, 1)
  assert.equal(b.$b[0].x, 2)
  const held = [a]
  rt.copy({ $b: held, $o: 0 }, { $b: [b], $o: 0 }, 1)
  assert.equal(held[0], b)
  assert.equal(a.$b[0].x, 1, 'the record the element held is not rewritten into the new one')
})

test('an aggregate held by value is written into the object in its place, and the source is not aliased', () => {
  const rt = runtime()
  const first = P(1)
  const second = P(2)
  const arr = [first, second]
  // An alias of the place's object -- the reference `&mut arr[0]` is that object.
  const alias = arr[0]
  ptrSwap(rt, { $b: arr, $o: 0 }, { $b: arr, $o: 1 }, cloneP)
  assert.equal(arr[0], first, 'each place keeps its object')
  assert.equal(arr[1], second)
  assert.deepEqual([alias.x, arr[1].x], [2, 1])
  const src = [P(7)]
  rt.copy_nonoverlapping({ $b: arr, $o: 0 }, { $b: src, $o: 0 }, 1, cloneP)
  assert.equal(arr[0].x, 7)
  assert.notEqual(arr[0], src[0])
  src[0].x = 8
  assert.equal(arr[0].x, 7)
})

test('a `__rt.box` buffer is the place of a local: a move into it is written through even with no clone', () => {
  const rt = runtime()
  // A value held by value that the compiler has no clone for, like a coroutine.
  const local = { TAG: 0, n: 1 }
  const p = rt.box(local)
  rt.copy_nonoverlapping(p, { $b: [{ TAG: 3, n: 2 }], $o: 0 }, 1)
  assert.equal(p.$b[0], local)
  assert.deepEqual({ ...local }, { TAG: 3, n: 2 })
  rt.copy(p, { $b: [{ TAG: 1, n: 3 }], $o: 0 }, 1)
  assert.equal(p.$b[0], local)
  assert.equal(local.n, 3)
})

test('a byte view does not describe its elements, so a byte copy over one keeps writing through', () => {
  const rt = runtime()
  const target = P(1)
  const dst = { $b: [target], $o: 0, $s: 8 }
  rt.copy_nonoverlapping(dst, { $b: [P(5)], $o: 0, $s: 8 }, 8)
  assert.equal(dst.$b[0], target)
  assert.equal(target.x, 5)
})

test('a move into an empty place stores the value, and an overlapping move inside one buffer runs away from the overlap', () => {
  const rt = runtime()
  const h = P(1)
  const fresh = []
  fresh.length = 2
  rt.copy_nonoverlapping({ $b: fresh, $o: 0 }, { $b: [h, h], $o: 0 }, 2, cloneP)
  assert.deepEqual(fresh.map((e) => e.x), [1, 1])
  assert.notEqual(fresh[0], h)
  const refs = [P(1), P(2), P(3), P(4)]
  const [r0, r1, r2] = refs
  // A non-array buffer cannot take `copyWithin`, so the loop has to pick its direction.
  const view = { 0: r0, 1: r1, 2: r2, 3: refs[3], length: 4 }
  rt.copy({ $b: view, $o: 1 }, { $b: view, $o: 0 }, 3)
  assert.deepEqual([view[0], view[1], view[2], view[3]], [r0, r0, r1, r2])
  assert.deepEqual([r0.x, r1.x, r2.x], [1, 2, 3])
})

test('`_put` itself is unchanged: it overwrites an object in place and stores where there is none', () => {
  const rt = runtime()
  const old = { TAG: 'Some', 0: 1 }
  const buf = [old, undefined]
  rt._put(buf, 0, { TAG: 'None' })
  assert.equal(buf[0], old)
  assert.deepEqual({ ...old }, { TAG: 'None' })
  const value = P(3)
  rt._put(buf, 1, value)
  assert.equal(buf[1], value)
})

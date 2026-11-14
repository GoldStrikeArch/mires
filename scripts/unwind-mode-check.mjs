// Runs one examples/unwind-tests program normally and with the host throwing.
//
//   node scripts/unwind-mode-check.mjs <program.js> <runtime/shim.js> <fixture name>
//
// `host_note` records its argument and throws one marker object on `-1`. A row passes when the
// export returns its value with the expected notes, or rethrows the marker itself with the guard's
// note after the throw: the destructor ran and the exception left with its identity.
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

const [program, shim, name] = process.argv.slice(2)
const fixtures = {
  '290_extern_c_import_rejected': { export: 'guarded_import', guard: 1, value: 3 },
  '291_extern_c_unwind_import_cleanup': { export: 'guarded_import', guard: 1, value: 3 },
}
const spec = fixtures[name]
if (!spec) throw new Error(`unknown fixture ${name}`)
const context = vm.createContext({ TextDecoder, TextEncoder })
vm.runInContext(readFileSync(shim, 'utf8'), context)
const marker = new Error('unwind-mode host marker')
let notes
context.__rt.host_note = (value) => {
  notes.push(Number(value))
  if (Number(value) === -1) throw marker
}
vm.runInContext(readFileSync(program, 'utf8'), context)
const rows = []
for (const throwing of [false, true]) {
  notes = []
  let returned
  let caught
  try {
    returned = context[spec.export](throwing)
  } catch (error) {
    caught = error
  }
  const expected = throwing ? [-1, spec.guard] : [2, spec.guard]
  const passed = JSON.stringify(notes) === JSON.stringify(expected)
    && (throwing ? caught === marker && returned === undefined : caught === undefined && returned === spec.value)
  rows.push({
    row: throwing ? 'host-throw' : 'normal',
    passed,
    notes,
    expected,
    returned: returned ?? null,
    thrownIsMarker: caught === marker,
    thrown: caught === undefined || caught === marker ? null : String(caught),
  })
}
const report = { program, fixture: name, passed: rows.every((row) => row.passed), rows }
console.log(JSON.stringify(report))
if (!report.passed) process.exitCode = 1

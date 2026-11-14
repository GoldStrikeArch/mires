// Runs one examples/unwind-tests program normally and with the host throwing.
//
//   node scripts/unwind-mode-check.mjs <program.js> <runtime/shim.js> <fixture name>
//
// `host_note` records its argument and throws one marker object on `-1`. A row passes when the
// export returns its value with the expected notes, or rethrows the marker itself with the guard's
// note after the throw: the destructor ran and the exception left with its identity. A fixture with
// several exports runs both rows for each. `globals` names the host function under the bare names a
// program prints for an import used as a function value (a pointer to it), besides `__rt`'s.
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

const [program, shim, name] = process.argv.slice(2)
const guarded = (name) => ({ export: name, guard: 1, value: 3 })
const fixtures = {
  '290_extern_c_import_rejected': { exports: [guarded('guarded_import')] },
  '291_extern_c_unwind_import_cleanup': { exports: [guarded('guarded_import')] },
  '292_exported_extern_c_guard': { exports: [{ export: 'exported_guard', guard: 7, value: 5 }] },
  '293_extern_c_fn_called_from_rust': { exports: [guarded('guarded_middle')] },
  '294_extern_c_fn_inlined': { exports: [guarded('guarded_inlined')] },
  '295_extern_c_fn_pointer_and_dyn': { exports: [guarded('guarded_pointer'), guarded('guarded_dyn')] },
  '296_cdecl_import_rejected': { exports: [guarded('guarded_import')] },
  '297_extern_c_import_through_pointer': { exports: [guarded('guarded_pointer')], globals: ['host_note'] },
  '298_import_from_sysroot_named_crate': { exports: [guarded('guarded_import')] },
  '299_extern_c_fn_of_dependency': { exports: [guarded('guarded_dependency')] },
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
for (const global of spec.globals ?? []) context[global] = (value) => context.__rt.host_note(value)
vm.runInContext(readFileSync(program, 'utf8'), context)
const rows = []
for (const entry of spec.exports) {
  for (const throwing of [false, true]) {
    notes = []
    let returned
    let caught
    try {
      returned = context[entry.export](throwing)
    } catch (error) {
      caught = error
    }
    const expected = throwing ? [-1, entry.guard] : [2, entry.guard]
    const passed = JSON.stringify(notes) === JSON.stringify(expected)
      && (throwing ? caught === marker && returned === undefined : caught === undefined && returned === entry.value)
    rows.push({
      export: entry.export,
      row: throwing ? 'host-throw' : 'normal',
      passed,
      notes,
      expected,
      returned: returned ?? null,
      thrownIsMarker: caught === marker,
      thrown: caught === undefined || caught === marker ? null : String(caught),
    })
  }
}
const report = { program, fixture: name, passed: rows.every((row) => row.passed), rows }
console.log(JSON.stringify(report))
if (!report.passed) process.exitCode = 1

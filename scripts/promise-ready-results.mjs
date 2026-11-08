import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
const root = process.argv[2];
const read = (directory,file) => JSON.parse(fs.readFileSync(path.join(root,directory,file)));
const reference = read('jir-control','result.json');
let cases = 0;
const rows = [];
for (const lane of ['baseline','jir']) for (const mode of ['off','report','on']) {
  const directory = `${lane}-${mode}`;
  const result = read(directory,'result.json');
  assert.deepEqual(result,reference,directory);
  cases += result.cases;
  if (lane === 'jir') {
    const report = read(directory,'program.mjs.lane.json');
    assert.equal(report.fallback_count,0);
    const decisions = report.async.ready_boundaries;
    assert.equal(decisions.filter(row => row.selected).length,mode === 'on' ? 1 : 0);
    if (mode === 'report') assert.equal(decisions.filter(row => row.would_select).length,1);
    if (mode === 'on') assert.match(fs.readFileSync(path.join(root,directory,'program.mjs'),'utf8'),/\.host_future_ready\(/);
    rows.push({directory,decisions});
  }
}
for (const kind of ['custom','suspending','local_drop','captured_drop','helper_drop','borrowed','mixed','inlined_helper']) {
  const result = read(kind,'result.json');
  assert.equal(result.passed,true);
  cases += result.cases;
  const report = read(kind,'program.mjs.lane.json');
  assert.equal(report.fallback_count,0);
  const decisions = report.async.ready_boundaries;
  assert.ok(decisions.length > 0,kind);
  assert.ok(decisions.every(row => !row.selected),kind);
  assert.doesNotMatch(fs.readFileSync(path.join(root,kind,'program.mjs'),'utf8'),/\.host_future_ready\(/);
  if (kind === 'mixed') assert.ok(decisions.some(row => row.reason === 'economics.generic-driver-retained'));
  if (kind === 'inlined_helper') {
    assert.ok(decisions.some(row => row.reason === 'legality.source-callee-cleanup'));
    assert.ok(report.items.every(row => !row.name.includes('$identity$')),'helper must actually be inlined');
  }
  rows.push({directory:kind,decisions});
}
cases += reference.cases;
const retry = read('jir-retry','program.mjs.lane.json');
assert.equal(retry.fallback_count,0);
assert.equal(retry.plan_retries,1);
assert.equal(retry.abi.registers,0);
assert.ok(retry.async.ready_boundaries.some(row => row.reason === 'proof.program-retry'));
assert.ok(retry.async.ready_boundaries.every(row => !row.selected && !row.would_select));
assert.deepEqual(read('jir-retry','result.json'),reference);
cases += reference.cases;
rows.push({directory:'jir-retry',decisions:retry.async.ready_boundaries});
const receipt = {passed:true,cases,rows};
fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(receipt,null,2)+'\n');
console.log(JSON.stringify(receipt,null,2));

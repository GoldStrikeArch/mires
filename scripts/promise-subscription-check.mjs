import { cases, runCase } from '../../../experiments-pro/compiler-v2/oracle-targets/async-transition-probe.mjs';

// The transition oracle is shared unchanged with the broad, still-incomplete async gate.
export async function checkPromiseSubscription(program) {
  const rows = [];
  for (const native of [false, true]) {
    for (const spec of cases.filter(spec => spec.kind === 'host')) {
      const expected = await runCase(null, spec, native, true);
      const actual = await runCase(program, spec, native);
      const passed = JSON.stringify(actual) === JSON.stringify(expected);
      rows.push({ native, case: spec.name, passed, expected, actual });
      if (!passed) throw new Error(`Promise subscription ${JSON.stringify(rows.at(-1))}`);
    }
    const trace = [], token = {}, other = {};
    let late, outcome;
    globalThis.transitionNote = value => trace.push(value);
    const promise = program.host_wait({ then(resolve, reject) { late = [resolve, reject]; throw token; } }, 1, native);
    const synchronous = trace.slice();
    promise.then(value => { trace.push('resolve'); outcome = value; }, error => { trace.push('reject'); outcome = error; });
    for (let i = 0; i < 30; i++) await Promise.resolve();
    late[0](other);
    late[1](other);
    for (let i = 0; i < 30; i++) await Promise.resolve();
    const passed = JSON.stringify(synchronous) === '[10,90]'
      && JSON.stringify(trace) === '[10,90,"reject"]' && outcome === token;
    rows.push({ native, case: 'late-callbacks-after-subscription-throw', passed, synchronous, trace, identity: outcome === token });
    if (!passed) throw new Error(`Late Promise callbacks ${JSON.stringify(rows.at(-1))}`);
  }
  return { passed: true, rows };
}

if (typeof process !== 'undefined' && process.argv[1]) {
  const { pathToFileURL } = await import('node:url');
  if (import.meta.url === pathToFileURL(process.argv[1]).href) {
    const errors = [];
    const fail = error => { errors.push(String(error)); process.exitCode = 1; };
    process.on('uncaughtException', fail);
    process.on('unhandledRejection', fail);
    try {
      const result = await checkPromiseSubscription(await import(pathToFileURL(process.argv[2]).href));
      if (errors.length) throw new Error(`Unexpected host errors: ${JSON.stringify(errors)}`);
      console.log(JSON.stringify(result));
    } catch (error) { console.error(error); process.exitCode = 1; }
  }
}

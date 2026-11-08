export async function checkReady(module, kind = 'ready') {
  const same = (actual, expected, label) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${label}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
  };
  const trace = [];
  globalThis.readyNote = phase => trace.push(phase);
  module.inert({});
  same(trace, kind === 'captured_drop' ? [0,90,3] : [0,3], 'inert construction');
  const rows = [];
  for (const reject of [false, true]) {
    trace.length = 0;
    const value = {};
    const promise = module.start(value, reject);
    const prefix = kind === 'helper_drop' ? [1,11,90,10] : kind === 'borrowed' && reject ? [1,12,10] : [1,10];
    const dropped = ['local_drop','captured_drop'].includes(kind) ? [90] : [];
    same(trace, [...prefix,...dropped,2], 'synchronous progress');
    const outcome = await promise.then(value => ({value,ok:true}), value => ({value,ok:false}));
    if (outcome.value !== value || outcome.ok === reject) throw new Error('result identity');
    rows.push({case:reject ? 'reject' : 'fulfill',trace:[...trace]});
  }
  if (kind !== 'ready') {
    delete globalThis.readyNote;
    return {passed:true,kind,cases:rows.length + 1,rows};
  }
  for (const scenario of ['plain','promise','thenable','getter-throw','call-throw','resolve-throw','reject-throw','poll-throw']) {
    trace.length = 0;
    const token = {}, thrown = {};
    let value = token;
    if (scenario === 'promise') value = Promise.resolve(token);
    if (['thenable','call-throw','resolve-throw','reject-throw'].includes(scenario)) value = {then(resolve,reject) {
      trace.push('then');
      if (scenario === 'call-throw') throw thrown;
      if (scenario === 'reject-throw') reject(token); else resolve(token);
      if (scenario.endsWith('-throw')) throw thrown;
    }};
    if (scenario === 'getter-throw') value = {get then() { trace.push('getter'); throw thrown; }};
    globalThis.readyNote = phase => { trace.push(phase); if (phase === 10 && scenario === 'poll-throw') throw thrown; };
    const promise = module.start(value, false);
    if (promise === value) throw new Error('outward Promise must be fresh');
    same(trace, scenario === 'getter-throw' ? [1,10,'getter',2] : [1,10,2], 'progress before adoption jobs');
    queueMicrotask(() => trace.push('sentinel'));
    const outcome = await promise.then(value => ({value,ok:true}), value => ({value,ok:false}));
    const rejected = ['getter-throw','call-throw','reject-throw','poll-throw'].includes(scenario);
    const expectedValue = ['getter-throw','call-throw','poll-throw'].includes(scenario) ? thrown : token;
    if (outcome.value !== expectedValue || outcome.ok === rejected) throw new Error(`${scenario}: rejection/fulfillment identity`);
    rows.push({case:scenario,ok:outcome.ok,trace:[...trace]});
  }
  trace.length = 0;
  let nested, entered = false;
  const outerValue = {}, innerValue = {};
  globalThis.readyNote = phase => {
    trace.push(phase);
    if (phase === 10 && !entered) { entered = true; nested = module.start(innerValue, false); }
  };
  const outer = module.start(outerValue, false);
  same(trace, [1,10,1,10,2,2], 'reentrant driving');
  if (await nested !== innerValue || await outer !== outerValue) throw new Error('reentrant result identity');
  rows.push({case:'reentrant',trace:[...trace]});
  delete globalThis.readyNote;
  return {passed:true,kind,cases:rows.length + 1,rows};
}

if (typeof process !== 'undefined' && process.argv[1]?.endsWith('promise-ready-check.mjs')) {
  const {pathToFileURL} = await import('node:url');
  const module = await import(pathToFileURL(process.argv[2]));
  console.log(JSON.stringify(await checkReady(module,process.argv[3]),null,2));
}

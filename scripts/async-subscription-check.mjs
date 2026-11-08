// Shared Node/browser checks over typed Rust futures and the deployed owner runtime.
export async function checkOwnerSubscription(program, dom) {
  const rows = [];
  const drain = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
  for (const kind of [0, 1, 2, 3]) {
    for (const input of ['plain', 'fulfilled', 'rejected', 'sync-resolve', 'sync-reject', 'throw-getter', 'throw-call', 'resolve-throw', 'reject-throw', 'late-throw']) {
      const trace = [], token = [{ token: true }, 'token', 71, undefined][kind];
      const other = [{ other: true }, 'other', 99, undefined][kind];
      let owner, dispose, late, outcome;
      dom.createRoot(cleanup => { dispose = cleanup; owner = dom.getOwner(); });
      const checkOwner = () => { if (dom.getOwner() !== owner) throw new Error('subscription owner lost'); };
      globalThis.subscriptionNote = phase => { checkOwner(); trace.push(phase); };
      globalThis.subscriptionResult = (value, ok) => { checkOwner(); trace.push(ok ? 'ok' : 'err'); outcome = { value, ok }; };
      globalThis.subscriptionUnit = ok => globalThis.subscriptionResult(undefined, ok);
      let value = token;
      if (input === 'fulfilled') value = Promise.resolve(token);
      if (input === 'rejected') value = Promise.reject(token);
      if (input === 'sync-resolve') value = { then(resolve) { resolve(token); } };
      if (input === 'sync-reject') value = { then(resolve, reject) { reject(token); } };
      if (input === 'throw-getter') value = { get then() { throw token; } };
      if (input === 'throw-call') value = { then() { throw token; } };
      if (input === 'resolve-throw') value = { then(resolve) { resolve(other); throw token; } };
      if (input === 'reject-throw') value = { then(resolve, reject) { reject(other); throw token; } };
      if (input === 'late-throw') value = { then(resolve, reject) { late = [resolve, reject]; throw token; } };
      dom.runWithOwner(owner, () => program.start(value, kind));
      if (JSON.stringify(trace) !== '[1,2]') throw new Error('first owner poll was not deferred');
      queueMicrotask(() => { trace.push('s1'); queueMicrotask(() => { trace.push('s2'); queueMicrotask(() => trace.push('s3')); }); });
      await drain();
      const ok = ['plain', 'fulfilled', 'sync-resolve'].includes(input);
      const completion = [90, 80, ok ? 'ok' : 'err', 20];
      const expected = input.includes('throw')
        ? [1, 2, 10, ...completion, 's1', 's2', 's3']
        : input.startsWith('sync-') ? [1, 2, 10, 's1', ...completion, 's2', 's3']
        : [1, 2, 10, 's1', 's2', ...completion, 's3'];
      if (late) { late[0](other); late[1](other); }
      dispose(); dispose(); await drain();
      const passed = JSON.stringify(trace) === JSON.stringify(expected)
        && outcome?.ok === ok && outcome?.value === token && dom.getOwner() === null;
      rows.push({ kind, input, passed, trace, expected, identity: outcome?.value === token });
      if (!passed) throw new Error(`owner subscription: ${JSON.stringify(rows.at(-1))}`);
    }
  }
  return { passed: true, rows };
}

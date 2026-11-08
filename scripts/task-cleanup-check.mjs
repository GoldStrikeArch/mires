// The same compiled future and owner runtime run in Node and browsers.
export async function checkTaskCleanup(program, dom) {
  const rows = [];
  const drain = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
  for (const owned of [false, true]) {
    const kinds = ['complete', 'throw-first', 'throw-resume', 'throw-drop', 'throw-both',
      ...(owned ? ['cancel-before', 'cancel-poll', 'cancel-throw', 'cancel-drop'] : [])];
    for (const kind of kinds) {
      // The DOM owner runtime normalizes non-Error throws; Error instances retain identity.
      const trace = [], errors = [], pollError = owned ? new Error('poll') : {},
        dropError = owned ? new Error('drop') : {};
      let owner = null, dispose = () => {};
      if (owned) dom.createRoot(cleanup => { dispose = cleanup; owner = dom.getOwner(); });
      const originalQueue = globalThis.queueMicrotask;
      // Retain the real queue and timing, but collect deliberately uncaught task exceptions.
      globalThis.queueMicrotask = callback => originalQueue(() => {
        try { callback(); } catch (error) { errors.push(error); trace.push('error'); }
      });
      globalThis.taskCleanupNote = phase => {
        if (dom.getOwner() !== owner) throw new Error('task cleanup owner lost');
        trace.push(phase);
        if (phase === 10 && ['cancel-poll', 'cancel-throw', 'cancel-drop'].includes(kind)) dispose();
        if (phase === 10 && ['throw-first', 'cancel-throw'].includes(kind)
          || phase === 20 && ['throw-resume', 'throw-both'].includes(kind)) throw pollError;
        if (phase === 80 && ['throw-drop', 'throw-both', 'cancel-drop'].includes(kind)) throw dropError;
      };
      try {
        try {
          if (owned) dom.runWithOwner(owner, () => program.start(true));
          else program.start(false);
        }
        catch (error) { errors.push(error); trace.push('error'); }
        const synchronous = trace.slice();
        if (kind === 'cancel-before') dispose();
        await drain();
        dispose(); dispose();
        await drain();
        const first = ['throw-first', 'cancel-poll', 'cancel-throw', 'cancel-drop'].includes(kind);
        const throws = kind.startsWith('throw-') || ['cancel-throw', 'cancel-drop'].includes(kind);
        const expected = kind === 'cancel-before' ? [80]
          : [...(first ? [10, 80] : [10, 20, 80]), ...(throws ? ['error'] : [])];
        const error = ['throw-drop', 'throw-both', 'cancel-drop'].includes(kind) ? dropError : pollError;
        const passed = JSON.stringify(trace) === JSON.stringify(expected)
          && JSON.stringify(synchronous) === JSON.stringify(owned ? [] : kind === 'throw-first' ? [10, 80, 'error'] : [10])
          && errors.length === (throws ? 1 : 0) && (!throws || errors[0] === error)
          && dom.getOwner() === null;
        rows.push({ owned, kind, passed, trace, expected, synchronous, errorCount: errors.length,
          identity: !throws || errors[0] === error });
      } finally {
        globalThis.queueMicrotask = originalQueue;
      }
    }
  }
  return { passed: rows.every(row => row.passed), rows };
}

if (typeof process !== 'undefined' && process.argv[1]) {
  const { pathToFileURL } = await import('node:url');
  if (import.meta.url === pathToFileURL(process.argv[1]).href) {
    const program = await import(pathToFileURL(process.argv[2]).href);
    const dom = await import(pathToFileURL(process.argv[3]).href);
    const result = await checkTaskCleanup(program, dom);
    console.log(JSON.stringify(result, null, 2));
    if (!result.passed) process.exitCode = 1;
  }
}

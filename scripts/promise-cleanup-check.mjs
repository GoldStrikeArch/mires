// Runs in Node or a browser over the same compiled safe Rust Future.
export async function checkPromiseCleanup(program) {
  const rows = [];
  for (const native of [false, true]) {
    for (const kind of ["complete", "reject", "throw-first", "throw-resume", "throw-drop", "throw-both"]) {
      const trace = [], value = {}, pollError = {}, dropError = {};
      globalThis.cleanupNote = phase => {
        trace.push(phase);
        if (phase === 10 && kind === "throw-first" || phase === 20 && ["throw-resume", "throw-both"].includes(kind)) throw pollError;
        if (phase === 80 && ["throw-drop", "throw-both"].includes(kind)) throw dropError;
      };
      let outcome;
      const promise = program.start(value, kind === "reject", native);
      const synchronous = trace.slice();
      promise.then(
        result => { trace.push("resolve"); outcome = { ok: true, value: result }; },
        error => { trace.push("reject"); outcome = { ok: false, value: error }; },
      );
      for (let i = 0; i < 30; i++) await Promise.resolve();
      const expectedValue = ["throw-drop", "throw-both"].includes(kind) ? dropError
        : ["throw-first", "throw-resume"].includes(kind) ? pollError : value;
      const expectedTrace = kind === "throw-first" ? [10, 80, "reject"]
        : [10, 20, 80, kind === "complete" ? "resolve" : "reject"];
      const passed = JSON.stringify(trace) === JSON.stringify(expectedTrace)
        && JSON.stringify(synchronous) === JSON.stringify(kind === "throw-first" ? [10, 80] : [10])
        && outcome?.ok === (kind === "complete") && outcome?.value === expectedValue;
      rows.push({ native, kind, passed, trace, expectedTrace, synchronous,
        settled: outcome !== undefined, identity: outcome?.value === expectedValue });
      if (!passed) throw new Error(`Promise cleanup ${native}/${kind}: ${JSON.stringify(rows.at(-1))}`);
    }
  }
  return { passed: true, rows };
}

if (typeof process !== "undefined" && process.argv[1]) {
  const { pathToFileURL } = await import("node:url");
  if (import.meta.url === pathToFileURL(process.argv[1]).href) {
    const errors = [];
    const fail = error => { errors.push(String(error)); process.exitCode = 1; };
    process.on("uncaughtException", fail);
    process.on("unhandledRejection", fail);
    try {
      const result = await checkPromiseCleanup(await import(pathToFileURL(process.argv[2]).href));
      if (errors.length) throw new Error(`Unexpected host errors: ${JSON.stringify(errors)}`);
      console.log(JSON.stringify(result));
    } catch (error) {
      console.error(error);
      process.exitCode = 1;
    }
  }
}

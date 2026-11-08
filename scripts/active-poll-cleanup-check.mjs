// The active Rust invocation must clean up its initialized locals before a host rejection settles.
// A driver dropping only the suspended future cannot satisfy these cases.
export async function checkActivePollCleanup(program) {
  const cases = [
    { name: "first-poll", throwAt: 10, normal: [10, 90, 80], thrown: [10, 90, 80] },
    { name: "resumed-local", throwAt: 30, normal: [30, 91, 80], thrown: [30, 91, 80] },
    { name: "partial-aggregate", throwAt: 82, normal: [82, 40, 81, 82, 80], thrown: [82, 81, 80] },
    { name: "moved-argument", throwAt: 13, normal: [13, 83, 80], thrown: [13, 83, 80] },
    { name: "replacement-rhs", throwAt: 85, normal: [85, 84, 40, 85, 80], thrown: [85, 84, 80] },
  ];
  const rows = [];
  for (const native of [false, true]) {
    for (const [index, spec] of cases.entries()) {
      for (const throwing of [false, true]) {
        const trace = [], value = {}, error = {};
        let injected = false, outcome;
        globalThis.activePollNote = event => {
          trace.push(event);
          if (throwing && !injected && event === spec.throwAt) {
            injected = true;
            throw error;
          }
        };
        try {
          const promise = program.start(index, value, native);
          promise.then(
            result => { trace.push("resolve"); outcome = { ok: true, value: result }; },
            result => { trace.push("reject"); outcome = { ok: false, value: result }; },
          );
        } catch (result) {
          trace.push("throw");
          outcome = { ok: false, value: result };
        }
        for (let step = 0; step < 40; step++) await Promise.resolve();
        const expected = [...(throwing ? spec.thrown : spec.normal), throwing ? "reject" : "resolve"];
        const identity = outcome?.value === (throwing ? error : value);
        const passed = JSON.stringify(trace) === JSON.stringify(expected)
          && identity && outcome?.ok === !throwing && injected === throwing;
        rows.push({ case: spec.name, native, throwing, passed, trace, expected, identity, injected });
      }
    }
  }
  return { passed: rows.every(row => row.passed), rows };
}

if (typeof process !== "undefined" && process.argv[1]) {
  const { pathToFileURL } = await import("node:url");
  if (import.meta.url === pathToFileURL(process.argv[1]).href) {
    const errors = [];
    const fail = error => { errors.push(String(error)); process.exitCode = 1; };
    process.on("uncaughtException", fail);
    process.on("unhandledRejection", fail);
    try {
      const result = await checkActivePollCleanup(await import(pathToFileURL(process.argv[2]).href));
      console.log(JSON.stringify({ ...result, errors }));
      if (!result.passed || errors.length) process.exitCode = 1;
    } catch (error) {
      console.error(error);
      process.exitCode = 1;
    }
  }
}

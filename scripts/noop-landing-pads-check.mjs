// Removing no-op landing pads must keep every drop and the thrown value when a host call throws.
export function checkNoopLandingPads(program) {
  const cases = [
    { name: "mixed", run: () => program.mixed(), normal: [10, 11, 1] },
    { name: "copy-only", run: () => program.copy_only(41), normal: [20], result: 42 },
    { name: "moved-then-inner", run: () => program.moved_then_inner(), normal: [103, 3, 40, 4] },
  ];
  const rows = [];
  for (const spec of cases) {
    // Throw at each host call in turn. A guard dropped after the throw also calls the host.
    for (const throwAt of [null, ...spec.normal]) {
      const trace = [], error = {};
      let injected = false, outcome;
      globalThis.noopPadNote = event => {
        trace.push(event);
        if (throwAt !== null && !injected && event === throwAt) {
          injected = true;
          throw error;
        }
      };
      try {
        outcome = { ok: true, value: spec.run() };
      } catch (value) {
        outcome = { ok: false, value };
      }
      const expected = throwAt === null ? spec.normal : expectedThrown(spec.name, throwAt);
      const passed = JSON.stringify(trace) === JSON.stringify(expected)
        && (throwAt === null
          ? outcome.ok && (spec.result === undefined || outcome.value === spec.result)
          : !outcome.ok && outcome.value === error && injected);
      rows.push({ case: spec.name, throwAt, passed, trace, expected });
    }
  }
  return { passed: rows.every(row => row.passed), rows };
}

// The trace up to the throw, then the drops of the guards still live at that point.
function expectedThrown(name, throwAt) {
  const table = {
    mixed: { 10: [10], 11: [10, 11, 1], 1: [10, 11, 1] },
    "copy-only": { 20: [20] },
    "moved-then-inner": { 103: [103, 3], 3: [103, 3], 40: [103, 3, 40, 4], 4: [103, 3, 40, 4] },
  };
  return table[name][throwAt];
}

if (typeof process !== "undefined" && process.argv[1]) {
  const { pathToFileURL } = await import("node:url");
  if (import.meta.url === pathToFileURL(process.argv[1]).href) {
    try {
      const result = checkNoopLandingPads(await import(pathToFileURL(process.argv[2]).href));
      console.log(JSON.stringify(result));
      if (!result.passed) process.exitCode = 1;
    } catch (error) {
      console.error(error);
      process.exitCode = 1;
    }
  }
}

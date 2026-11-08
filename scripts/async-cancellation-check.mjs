// Shared Node/browser checks over compiled Rust and the deployed reactive runtime.
export async function checkOwnerCancellation(program, dom) {
  let trace, pending, results, owner, dispose, cancelOnPoll = false;
  let checks = 0;
  const assert = (condition, label) => {
    if (!condition) throw new Error(`owner cancellation: ${label}: ${JSON.stringify(trace)}`);
    checks++;
  };
  const equal = (actual, expected, label) =>
    assert(JSON.stringify(actual) === JSON.stringify(expected), label);
  const drain = async () => {
    for (let i = 0; i < 20; i++) await Promise.resolve();
  };
  globalThis.ownerNote = (task, phase) => {
    trace.push([task, phase]);
    if (phase >= 10) assert(dom.getOwner() === owner, "poll/drop owner");
    if (cancelOnPoll && phase === 10) dispose();
  };
  globalThis.ownerPending = (task, phase) => new Promise((resolve, reject) => {
    pending.push({ task, phase, resolve, reject });
  });
  globalThis.ownerResult = (task, phase, value, ok) => results.push({ task, phase, value, ok });
  globalThis.ownerAction = () => {};
  const reset = () => {
    trace = []; pending = []; results = []; cancelOnPoll = false;
    dom.createRoot((cleanup) => { dispose = cleanup; owner = dom.getOwner(); });
  };
  const start = (task = 1, steps = 2) => dom.runWithOwner(owner, () => program.start(task, steps));
  const phases = () => trace.filter(([task]) => task === 1).map(([, phase]) => phase);
  const late = async () => {
    const before = JSON.stringify(trace);
    for (const item of pending) item.resolve({ late: true });
    dispose();
    await drain();
    assert(JSON.stringify(trace) === before, "late fulfillment/repeated disposal is inert");
    assert(dom.getOwner() === null, "owner restored");
  };

  reset(); start(); dispose(); await drain();
  equal(phases(), [1, 2, 80], "dispose before first poll drops captured state only");
  equal(pending.length, 0, "no host work before first poll");
  await late();

  for (const stage of ["pending", "queued", "second", "polling"]) {
    reset(); cancelOnPoll = stage === "polling"; start(); await drain();
    if (stage === "second") { pending[0].resolve({ first: true }); await drain(); }
    if (stage === "queued") pending[0].resolve({ queued: true });
    dispose(); await drain();
    equal(phases(), stage === "second"
      ? [1, 2, 10, 20, 100, 101, 90, 80]
      : [1, 2, 10, 100, 90, 80], `${stage}: exact initialized-field drop order`);
    equal(results.length, stage === "second" ? 1 : 0, `${stage}: no cancelled result`);
    await late();
  }

  for (const reject of [false, true]) {
    reset(); start(1, 1); await drain();
    const value = { identity: reject ? "rejection" : "fulfillment" };
    pending[0][reject ? "reject" : "resolve"](value); await drain();
    equal(phases(), reject ? [1, 2, 10, 20, 40, 100, 30, 90, 80]
      : [1, 2, 10, 20, 100, 30, 90, 80], "completed drop order");
    assert(results[0].value === value && results[0].ok === !reject, "result identity");
    await late();
  }

  reset(); owner = null; program.start(1, 1); await drain(); dispose();
  pending[0].resolve("ownerless"); await drain();
  equal(phases(), [1, 2, 10, 20, 100, 30, 90, 80], "ownerless task completes");

  reset(); start(1); start(2); await drain(); dispose(); await drain();
  for (const task of [1, 2]) equal(trace.filter(([id]) => id === task).map(([, phase]) => phase),
    [1, 2, 10, 100, 90, 80], "all tasks under owner cancelled");
  await late();

  reset();
  let refresh;
  dom.runWithOwner(owner, () => {
    const [read, write] = dom.createSignal(1); refresh = write;
    dom.effect(() => { const task = read(); owner = dom.getOwner(); program.start(task, 2); });
  });
  await drain(); refresh(2); await drain();
  equal(phases(), [1, 2, 10, 100, 90, 80], "effect refresh cancels previous task");
  dispose(); await drain();
  equal(trace.filter(([id]) => id === 2).map(([, phase]) => phase),
    [1, 2, 10, 100, 90, 80], "root disposal cancels refreshed task");
  await late();
  return { passed: true, checks };
}

import assert from "node:assert/strict";
import test from "node:test";

import "../runtime/shim.js";

test("owner cleanup cancels before polling, while suspended, and during a poll", async () => {
  for (const phase of ["before", "pending", "polling", "ready"]) {
    const owner = {}, future = {}, trace = [];
    let currentOwner = null, cleanup, waker, state, inPoll = false;
    let polls = 0, drops = 0;
    const run = (next, action) => {
      const previous = currentOwner;
      currentOwner = next;
      try { return action(); }
      finally { currentOwner = previous; }
    };
    const dispose = () => {
      trace.push("dispose");
      cleanup();
      assert.equal(currentOwner, owner);
    };
    globalThis.__rt.host_spawn(future, (next, wake) => {
      state = next; waker = wake; polls++;
      assert.equal(currentOwner, owner);
      inPoll = true;
      trace.push("poll-start");
      if (phase === "polling") {
        dispose();
        assert.equal(drops, 0, "an active poll retains its pinned frame");
      }
      trace.push("poll-end");
      inPoll = false;
      return phase === "ready";
    }, next => {
      state = next; drops++;
      assert.equal(next.future, future);
      assert.equal(inPoll, false);
      assert.equal(currentOwner, owner);
      trace.push("drop");
    }, true, owner, run, callback => {
      assert.equal(currentOwner, owner);
      cleanup = callback;
    });
    assert.equal(currentOwner, null);
    assert.equal(typeof cleanup, "function");
    if (phase === "before") run(owner, dispose);
    await Promise.resolve();
    if (phase === "pending" || phase === "ready") run(owner, dispose);
    run(owner, dispose);
    if (waker) {
      waker.vtable.wake_by_ref(waker.data);
      waker.vtable.wake_by_ref(waker.data);
    }
    await Promise.resolve();
    assert.equal(currentOwner, null);
    assert.equal(polls, phase === "before" ? 0 : 1);
    assert.equal(drops, 1);
    assert.equal(state.future, undefined);
    assert.equal(state.done, true);
    if (phase === "polling") assert.deepEqual(trace.slice(0, 4), ["poll-start", "dispose", "poll-end", "drop"]);
  }
});

test("an ownerless task does not register cleanup", async () => {
  let polls = 0, drops = 0;
  globalThis.__rt.host_spawn({}, () => { polls++; return true; }, () => { drops++; },
    true, null, (_owner, action) => action(), () => assert.fail("no owner"));
  assert.equal(polls, 0);
  await Promise.resolve();
  assert.equal(polls, 1);
  assert.equal(drops, 1);
});

test("a throwing Promise poll drops its future before rejecting", async () => {
  const future = { retained: true };
  const reason = { reason: "poll threw" };
  const order = [];
  let state;
  let waker;
  let polls = 0;
  let drops = 0;

  const promise = globalThis.__rt.host_future_promise(
    future,
    (nextState, nextWaker) => {
      state = nextState;
      waker = nextWaker;
      polls += 1;
      order.push("poll");
      assert.strictEqual(state.future, future);
      throw reason;
    },
    (nextState) => {
      drops += 1;
      order.push("drop");
      assert.strictEqual(nextState, state);
      assert.strictEqual(nextState.future, future);
    },
  );

  const outcome = await promise.then(
    (value) => ({ status: "resolved", value }),
    (error) => {
      order.push("reject");
      return { status: "rejected", error };
    },
  );

  assert.equal(outcome.status, "rejected");
  assert.strictEqual(outcome.error, reason);
  assert.deepEqual(order, ["poll", "drop", "reject"]);
  assert.equal(polls, 1);
  assert.equal(drops, 1);
  assert.equal(state.done, true);
  assert.equal(state.future, undefined);

  waker.vtable.wake_by_ref(waker.data);
  await Promise.resolve();
  assert.equal(polls, 1);
  assert.equal(drops, 1);
});

test("a throwing owner-aware poll drops under its captured owner", async () => {
  const future = { retained: true };
  const owner = { name: "captured" };
  const reason = { reason: "poll threw" };
  const trace = [];
  let currentOwner = null;
  let state;
  let waker;
  let polls = 0;
  let drops = 0;

  const run = (nextOwner, callback) => {
    const previous = currentOwner;
    currentOwner = nextOwner;
    trace.push("enter");
    try {
      return callback();
    } finally {
      trace.push("exit");
      currentOwner = previous;
    }
  };

  let thrown;
  try {
    globalThis.__rt.host_spawn(
      future,
      (nextState, nextWaker) => {
        state = nextState;
        waker = nextWaker;
        polls += 1;
        trace.push("poll");
        assert.strictEqual(currentOwner, owner);
        throw reason;
      },
      (nextState) => {
        drops += 1;
        trace.push("drop");
        assert.strictEqual(currentOwner, owner);
        assert.strictEqual(nextState, state);
        assert.strictEqual(nextState.future, future);
      },
      false,
      owner,
      run,
    );
  } catch (error) {
    thrown = error;
  }

  assert.strictEqual(thrown, reason);
  assert.deepEqual(trace, ["enter", "poll", "exit", "enter", "drop", "exit"]);
  assert.equal(currentOwner, null);
  assert.equal(polls, 1);
  assert.equal(drops, 1);
  assert.equal(state.done, true);
  assert.equal(state.future, undefined);

  waker.vtable.wake_by_ref(waker.data);
  await Promise.resolve();
  assert.equal(polls, 1);
  assert.equal(drops, 1);
});

// Checks examples/host_event_access.rs: listener closures installed on an opaque host object,
// reading the event's properties and calling its methods.
//
//   node scripts/host-event-access-check.mjs <program.mjs>
//
// The `input` listener's assertions are scripts/event-accessor-check.mjs's, which checked
// `Event::target_value` and `Event::prevent_default` on the DOM suite's 21_event_accessors and no
// longer runs (that fixture needs the framework's `view_dom_macro`). Here nothing is called
// directly: the program installs its listeners through `addEventListener` on a fake element, and
// the check dispatches fake events to whatever was installed, so the closure, the property reads
// and the method call are exercised together.
//
// The `submit` listener holds a `Drop` guard across `preventDefault`. The check makes that call
// throw values of several kinds and requires, per dispatch, that the guard dropped exactly once,
// that nothing after the throwing call ran, and that the exception reaching the caller is the
// very value the host threw.

import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const program = resolve(process.argv[2]);
const api = await import(pathToFileURL(program));

let failures = 0, checks = 0;
const check = (label, ok, detail) => {
  checks += 1;
  console.log(`${ok ? 'ok  ' : 'FAIL'}  ${label}${detail === undefined ? '' : `: ${detail}`}`);
  if (!ok) failures += 1;
};

// A fake element: records every `addEventListener` call and the receiver it was made on.
const element = (name) => {
  const self = {
    name,
    installed: [],
    addEventListener(kind, listener, ...rest) {
      self.installed.push({ receiver: this, kind, listener, rest });
    },
  };
  return self;
};
const installed = (host, label) => {
  check(`${label}: addEventListener called exactly once`, host.installed.length === 1, String(host.installed.length));
  const [call] = host.installed;
  check(`${label}: on the element it was handed`, call?.receiver === host);
  check(`${label}: with the event type and nothing after the listener`, call?.rest.length === 0, JSON.stringify(call?.kind));
  check(`${label}: the listener is a one-argument function`, typeof call?.listener === 'function' && call.listener.length === 1);
  return call;
};

// ----------------------------------------------------------------------------- input listener

{
  const host = element('field'), log = [];
  api.listen_input(host, log);
  const { kind, listener } = installed(host, 'input');
  check('input: the type is "input"', kind === 'input', JSON.stringify(kind));
  const dispatch = (event) => {
    log.length = 0;
    const result = listener.call(host, event);
    return { result, entry: log.length === 1 ? log[0] : log };
  };
  const plain = (target) => ({ target, preventDefault() {} });

  // The value the user typed, which is the whole reason the accessor exists.
  const typed = dispatch(plain({ value: 'hello' }));
  check('the text the user typed comes back', typed.entry === 'hello', JSON.stringify(typed.entry));
  check('the listener returns undefined', typed.result === undefined);
  check('an empty field is the empty string', dispatch(plain({ value: '' })).entry === '');

  // A string is promised. An element with no `value` -- a `<div>` the event bubbled from -- must
  // not hand the reader `undefined`.
  const missing = dispatch(plain({})).entry;
  check('an element with no `value` is the empty string, not undefined', missing === '', JSON.stringify(missing));
  check('and the answer is a string', typeof missing === 'string', typeof missing);
  check('so the length of what comes back is readable', dispatch(plain({ value: 'abcd' })).entry.length === 4);

  // `target`, not `currentTarget` and not the event itself: the element the event came FROM.
  check(
    'the value is read off `target`, not off the event itself',
    dispatch({ value: 'wrong', currentTarget: { value: 'wrong' }, target: { value: 'right' }, preventDefault() {} }).entry === 'right',
  );

  // preventDefault is called once, on the event the listener was given, after the read.
  const order = [];
  let calls = 0, receiver = null;
  const event = {
    get target() { order.push('target'); return { get value() { order.push('value'); return 'typed'; } }; },
    preventDefault() { order.push('preventDefault'); calls += 1; receiver = this; },
  };
  dispatch(event);
  check('prevent_default calls preventDefault exactly once', calls === 1, String(calls));
  check('on the event it was handed', receiver === event);
  check('after reading target, then value', order.join() === 'target,value,preventDefault', order.join());

  // Each dispatch is independent: the listener is reusable and keeps its captured log.
  for (let turn = 0; turn < 3; turn += 1) dispatch(plain({ value: `turn ${turn}` }));
  check('a listener dispatched repeatedly answers each event', log[0] === 'turn 2', JSON.stringify(log));
}

// -------------------------------------------------------------------------- guarded listener

{
  const host = element('form'), log = [];
  api.listen_guarded(host, log);
  const { kind, listener } = installed(host, 'submit');
  check('submit: the type is "submit"', kind === 'submit', JSON.stringify(kind));

  // The normal path: every entry, and the guard last.
  for (let turn = 0; turn < 2; turn += 1) {
    log.length = 0;
    let calls = 0;
    listener.call(host, { target: { value: `sent ${turn}` }, preventDefault() { calls += 1; } });
    check(
      `normal dispatch ${turn}: before, the value, then the guard, once`,
      JSON.stringify(log) === JSON.stringify(['before preventDefault', `sent ${turn}`, 'guard dropped']) && calls === 1,
      JSON.stringify(log),
    );
  }

  // The throwing path: the guard drops exactly once, nothing after the call runs, and the caller
  // receives the thrown value itself. A plain object, a symbol and `undefined` are thrown as well
  // as an Error, because a cleanup path that rethrew "an error" rather than the value would pass
  // with an Error alone.
  const thrown = {
    error: new Error('preventDefault refused'),
    object: Object.freeze({ sentinel: true }),
    symbol: Symbol('sentinel'),
    undefined: undefined,
    null: null,
  };
  for (const [label, value] of Object.entries(thrown)) {
    for (let turn = 0; turn < 2; turn += 1) {
      log.length = 0;
      let caught = false, identity = false, reads = 0;
      try {
        listener.call(host, {
          get target() { reads += 1; return { value: 'never read' }; },
          preventDefault() { throw value; },
        });
      } catch (error) {
        caught = true;
        identity = Object.is(error, value);
      }
      check(`throw ${label} (${turn}): the exception reaches the caller`, caught);
      check(`throw ${label} (${turn}): it is the value the host threw`, identity);
      check(
        `throw ${label} (${turn}): the guard dropped exactly once, after the entry before the call`,
        JSON.stringify(log) === JSON.stringify(['before preventDefault', 'guard dropped']),
        JSON.stringify(log),
      );
      check(`throw ${label} (${turn}): nothing after the throwing call ran`, reads === 0, String(reads));
    }
  }

  // The listener is still whole after the throws.
  log.length = 0;
  listener.call(host, { target: { value: 'after' }, preventDefault() {} });
  check('a dispatch after the throws runs normally', JSON.stringify(log) === JSON.stringify(['before preventDefault', 'after', 'guard dropped']), JSON.stringify(log));
}

// The checks above must not pass against a program that never installed anything.
check('the program exports both installers', typeof api.listen_input === 'function' && typeof api.listen_guarded === 'function');
check('the program is an ES module that imports its shim', /^import\b/m.test(readFileSync(program, 'utf8')));

writeFileSync(`${program}.check.json`, JSON.stringify({ passed: failures === 0, checks, failures }) + '\n');
console.log(failures === 0 ? `host event access: ${checks} checks passed` : `host event access: ${failures} of ${checks} checks FAILED`);
process.exit(failures === 0 ? 0 : 1);

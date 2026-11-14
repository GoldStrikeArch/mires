// The worker behind `test_node::run`: one long-lived node process that runs the programs the
// workspace's unit tests check, each in a fresh `vm` context, so a check costs a context and not
// a process. It is started as `node -e <this file>` and spoken to over its stdin and stdout.
//
// A request is the mode byte, the timeout in milliseconds (u32, big-endian), the payload's length
// in bytes (u32, big-endian) and the payload: the program's text, or for `f` and `k` the path of
// the file holding it. The modes are the command lines they stand for:
//
//   s  node -                         the program on stdin
//   c  node --input-type=commonjs     the program on stdin
//   e  node -e <program>
//   f  node <path>
//   k  node --check <path>
//
// The reply is "TNOD", a byte that is 1 when the program ran out of time and 0 otherwise, the
// exit status node would have given (i32), and the program's stdout and stderr, each a u32
// length followed by the bytes.
//
// What node does with each command line, and so what this file reproduces (Node 26):
//
//   * `-`, `--input-type=commonjs` and `-e` run the text as a script in the global scope. A
//     top-level `var` or function is a property of `globalThis`, `this` is `globalThis`, there is
//     no `arguments`, and `require`, `module`, `exports`, `__filename` (`[stdin]` or `[eval]`)
//     and `__dirname` (`.`) are globals.
//   * `node <path>` runs the file as a CommonJS module: the text is a function body, so a
//     top-level `var` stays local, `this` is `module.exports`, `arguments` exists, `return` is
//     allowed, and `require` and friends are parameters, with `__filename` the file's real path
//     (symbolic links resolved, as node resolves its main module).
//   * `--check` parses the file as that same function body and runs nothing.
//
// A program is finished when it is quiescent: its synchronous run returned, the microtask queue
// drained, and none of the timers it set is still pending. It fails, as node exits with status
// 1, on the first uncaught exception, unhandled rejection, or syntax error, and its output stops
// there, as node's does. A program that does not finish within its timeout fails as well; the
// caller then retires this worker, since whatever the program left running is still here.
'use strict';

const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');
const util = require('node:util');
const { Writable } = require('node:stream');
const { Console } = require('node:console');
const Module = require('node:module');

const CJS_PARAMETERS = ['exports', 'require', 'module', '__filename', '__dirname'];
const HOST = globalThis;

// The codecs must answer with this context's `Uint8Array`, or the runtime's
// `buf instanceof Uint8Array` would tell a host array from its own. So the context gets small
// classes of its own that forward to the host's codecs.
const CODECS = new vm.Script(
  `(function (HostEncoder, HostDecoder) {
  'use strict';
  const encoder = new HostEncoder();
  class TextEncoder {
    get encoding() { return encoder.encoding; }
    encode(input = '') { return new Uint8Array(encoder.encode(input)); }
    encodeInto(source, destination) {
      const { read, written } = encoder.encodeInto(source, destination);
      return { read, written };
    }
  }
  class TextDecoder {
    #decoder;
    constructor(label = 'utf-8', options = {}) { this.#decoder = new HostDecoder(label, options); }
    get encoding() { return this.#decoder.encoding; }
    get fatal() { return this.#decoder.fatal; }
    get ignoreBOM() { return this.#decoder.ignoreBOM; }
    decode(input, options) { return this.#decoder.decode(input, options); }
  }
  for (const [name, value] of [['TextEncoder', TextEncoder], ['TextDecoder', TextDecoder]]) {
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });
  }
})`,
  { filename: 'test-node:codecs' },
);

// `node <path>` runs the module function under the same timeout as a script, so the function is
// called from a one-line script in its context. The name is gone before the program runs.
const ENTRY = ' test-node entry ';
const CALL_ENTRY = new vm.Script(`globalThis[${JSON.stringify(ENTRY)}]()`, {
  filename: 'test-node:entry',
});

// Without its parent this process has no one to answer, yet a program that keeps the event loop
// busy (an endless chain of microtasks) would keep it running forever: the parent's kill can miss
// it when `node` is a launcher that starts the real node as its child, and a parent killed
// outright never closes stdin. So a thread of its own, which no program can block, ends the
// process once the parent is gone.
new (require('node:worker_threads').Worker)(
  `const parent = process.ppid;
  setInterval(() => { if (process.ppid !== parent) process.kill(process.pid, 'SIGKILL'); }, 250);`,
  { eval: true },
).unref();

// The program running now. Anything thrown or rejected while it runs is its failure.
let active = null;

process.on('uncaughtException', (error) => fail(active, error));
process.on('unhandledRejection', (reason) => fail(active, reason));

function fail(session, error) {
  if (session === null) {
    process.stderr.write(`test-node: an error outside any program: ${util.inspect(error)}\n`);
    return;
  }
  if (session.done) return;
  if (error !== null && typeof error === 'object' && error.code === 'ERR_SCRIPT_EXECUTION_TIMEOUT') {
    session.timedOut = true;
  }
  // Node prints an uncaught error by `util.inspect` (its stack, already headed by the source line
  // for a syntax error) and anything else after "Uncaught".
  const shown = util.types.isNativeError(error) || error instanceof Error
    ? util.inspect(error)
    : `Uncaught ${util.inspect(error)}`;
  end(session, `${shown}\n\nNode.js ${process.version}\n`);
}

// Ends `session` as failed with `message` on its stderr: what it still has pending is cancelled,
// and nothing it does from here on is output.
function end(session, message) {
  if (session.done) return;
  session.done = true;
  session.code = 1;
  session.stderr.push(Buffer.from(message));
  for (const [handle, clear] of session.timers) clear(handle);
  session.timers.clear();
  session.wake();
}

// A stream into `chunks` that stops taking output once the program has failed.
function capture(session, chunks) {
  return new Writable({
    write(chunk, _encoding, callback) {
      if (!session.done) chunks.push(chunk);
      callback();
    },
  });
}

// The host timers, counted against `session` so that a pending one keeps the program running.
function timers(session) {
  const track = (handle, clear) => {
    session.timers.set(handle, clear);
    return handle;
  };
  const settle = (handle) => {
    if (session.timers.delete(handle)) session.wake();
  };
  return {
    setTimeout(callback, delay, ...args) {
      const handle = setTimeout(() => {
        settle(handle);
        callback(...args);
      }, delay);
      return track(handle, clearTimeout);
    },
    setInterval(callback, delay, ...args) {
      return track(setInterval(() => callback(...args), delay), clearInterval);
    },
    setImmediate(callback, ...args) {
      const handle = setImmediate(() => {
        settle(handle);
        callback(...args);
      });
      return track(handle, clearImmediate);
    },
    clearTimeout(handle) {
      clearTimeout(handle);
      settle(handle);
    },
    clearInterval(handle) {
      clearInterval(handle);
      settle(handle);
    },
    clearImmediate(handle) {
      clearImmediate(handle);
      settle(handle);
    },
  };
}

// Defines `values` on the context's global object with the flags the same globals have in node.
function define(context, values) {
  for (const [name, value] of Object.entries(values)) {
    const host = Object.getOwnPropertyDescriptor(HOST, name);
    Object.defineProperty(context, name, {
      value,
      writable: true,
      configurable: true,
      enumerable: host ? host.enumerable : true,
    });
  }
}

// A fresh context holding what the programs use from node: a console writing into the session,
// `queueMicrotask`, the timers, the codecs, and `atob`/`btoa`.
function contextFor(session) {
  const context = vm.createContext({});
  define(context, {
    console: new Console(capture(session, session.stdout), capture(session, session.stderr)),
    queueMicrotask,
    ...timers(session),
    atob,
    btoa,
  });
  CODECS.runInContext(context)(TextEncoder, TextDecoder);
  return context;
}

// The module object, `require` and globals node gives the text of `-e` and stdin.
function scriptGlobals(name) {
  const filename = path.join(process.cwd(), name);
  const module = new Module(name, null);
  module.filename = filename;
  module.paths = Module._nodeModulePaths(process.cwd());
  return {
    require: Module.createRequire(filename),
    module,
    exports: module.exports,
    __filename: name,
    __dirname: '.',
  };
}

function stripBom(text) {
  return text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
}

// Starts the program; what it leaves pending runs after this returns.
function start(session, mode, timeout, payload) {
  if (mode === 'k') {
    const filename = fs.realpathSync(payload);
    vm.compileFunction(stripBom(fs.readFileSync(filename, 'utf8')), CJS_PARAMETERS, { filename });
    return;
  }
  const context = contextFor(session);
  if (mode === 'f') {
    const filename = fs.realpathSync(payload);
    const dirname = path.dirname(filename);
    const source = stripBom(fs.readFileSync(filename, 'utf8'));
    const module = new Module(filename, null);
    module.filename = filename;
    module.paths = Module._nodeModulePaths(dirname);
    const require = Module.createRequire(filename);
    const body = vm.compileFunction(source, CJS_PARAMETERS, { filename, parsingContext: context });
    Object.defineProperty(context, ENTRY, {
      configurable: true,
      value: () => {
        delete context[ENTRY];
        return Reflect.apply(body, module.exports, [module.exports, require, module, filename, dirname]);
      },
    });
    CALL_ENTRY.runInContext(context, { timeout, displayErrors: true });
    return;
  }
  const name = mode === 'e' ? '[eval]' : '[stdin]';
  define(context, scriptGlobals(name));
  new vm.Script(payload, { filename: name }).runInContext(context, { timeout, displayErrors: true });
}

// Resolves once `session` is quiescent: every microtask has run (a host `setImmediate` comes only
// after them, and after node has reported the rejections they left unhandled) and no timer of the
// program's is pending.
async function quiescent(session) {
  for (;;) {
    await new Promise((resolve) => setImmediate(resolve));
    if (session.done || session.timers.size === 0) return;
    await new Promise((resolve) => {
      session.wake = resolve;
    });
    session.wake = () => {};
  }
}

async function run(mode, timeout, payload) {
  const session = {
    stdout: [],
    stderr: [],
    timers: new Map(),
    done: false,
    timedOut: false,
    code: 0,
    wake: () => {},
  };
  active = session;
  const guard = setTimeout(() => {
    session.timedOut = !session.done;
    end(session, `test-node: the program did not finish within ${timeout} ms\n`);
  }, timeout);
  try {
    start(session, mode, timeout, payload);
  } catch (error) {
    fail(session, error);
  }
  await quiescent(session);
  clearTimeout(guard);
  active = null;
  const stdout = Buffer.concat(session.stdout);
  const stderr = Buffer.concat(session.stderr);
  const head = Buffer.alloc(13);
  head.write('TNOD', 0, 'latin1');
  head.writeUInt8(session.timedOut ? 1 : 0, 4);
  head.writeInt32BE(session.code, 5);
  head.writeUInt32BE(stdout.length, 9);
  const tail = Buffer.alloc(4);
  tail.writeUInt32BE(stderr.length, 0);
  return Buffer.concat([head, stdout, tail, stderr]);
}

let input = Buffer.alloc(0);
let busy = false;

function next() {
  if (busy || input.length < 9) return;
  const length = input.readUInt32BE(5);
  if (input.length < 9 + length) return;
  const mode = String.fromCharCode(input[0]);
  const timeout = input.readUInt32BE(1);
  const payload = input.toString('utf8', 9, 9 + length);
  input = input.subarray(9 + length);
  busy = true;
  run(mode, timeout, payload).then(
    (reply) => {
      process.stdout.write(reply);
      busy = false;
      next();
    },
    // A fault of this file, not of the program: the caller sees the worker die, with this.
    (error) => {
      process.stderr.write(`test-node: the worker failed: ${util.inspect(error)}\n`);
      process.exit(70);
    },
  );
}

process.stdin.on('data', (chunk) => {
  input = input.length === 0 ? chunk : Buffer.concat([input, chunk]);
  next();
});
// The test process is gone: so is the reason to be here.
process.stdin.on('end', () => process.exit(0));

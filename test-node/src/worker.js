// The worker behind `test_node::run`: one long-lived node process that runs the programs the
// workspace's tests check, each in a `worker_threads` thread of its own, so a check costs a thread
// and not a process. It is started as `node -e <this file>` and spoken to over its stdin and
// stdout.
//
// A request is the mode byte, the timeout in milliseconds (u32, big-endian), the program's length
// in bytes (u32, big-endian) and the program's text. The modes are the command lines they stand
// for:
//
//   s  node -                         the program on stdin
//   c  node --input-type=commonjs     the program on stdin
//
// The reply is "TNOD", a byte that is 1 when the program ran out of time and 0 otherwise, the exit
// status (i32), and the program's stdout and stderr, each a u32 length followed by the bytes.
//
// A thread is as close to a node process as node gets without starting one: its own isolate,
// globals, module cache, `process.exitCode` and `process.exit`, event loop and uncaught-error
// handling. Node evaluates stdin as a script in the global scope of a `[stdin]` module whose
// `require`, `module`, `exports`, `__filename` (`[stdin]`) and `__dirname` (`.`) are globals
// (`evalScript` in lib/internal/process/execution.js), and BOOTSTRAP does the same in the thread.
// The thread then runs until its event loop is empty, as the process would, and its exit code is
// the process's. An uncaught exception or unhandled rejection reaches this process as the thread's
// `error` event, ends the program's output there and is printed after its stderr, as node prints
// it. A program that does not finish within its timeout is terminated, and this process goes on to
// the next one.
'use strict';

const { Worker } = require('node:worker_threads');
const util = require('node:util');

// Without its parent this process has no one to answer. The parent's kill can miss it when `node`
// is a launcher that starts the real node as its child, so a thread of its own, which nothing else
// here can block, ends the process once the parent is gone.
new Worker(
  `const parent = process.ppid;
  setInterval(() => { if (process.ppid !== parent) process.kill(process.pid, 'SIGKILL'); }, 250);`,
  { eval: true },
).unref();

// What a program's thread runs first. It sits in a function so that none of its names is a global
// the program could collide with. Node reads the program from stdin and runs it in the stream's
// 'end' handler, after which the stream destroys itself (`readStdin`); so the program comes in here
// through a stream that stands for stdin, a duplex whose writable side has ended, as a pipe's
// socket is.
const BOOTSTRAP = `(() => {
  const { workerData } = require('node:worker_threads');
  const { Duplex } = require('node:stream');
  const vm = require('node:vm');
  const path = require('node:path');
  const Module = require('node:module');
  process.argv = workerData.commonjs ? [process.argv[0]] : [process.argv[0], '-'];
  process.execArgv = workerData.commonjs ? ['--input-type=commonjs'] : [];
  const stdin = new Duplex({ read() {}, writable: false });
  const { enumerable } = Object.getOwnPropertyDescriptor(process, 'stdin');
  Object.defineProperty(process, 'stdin', { configurable: true, enumerable, get: () => stdin });
  stdin.setEncoding('utf8');
  let code = '';
  stdin.on('data', (chunk) => { code += chunk; });
  stdin.on('end', () => {
    process._eval = code;
    const name = '[stdin]';
    const cwd = process.cwd();
    const module = new Module(name);
    module.filename = path.join(cwd, name);
    module.paths = Module._nodeModulePaths(cwd);
    globalThis.__filename = name;
    const run = module._compile(
      'globalThis.module = module; globalThis.exports = exports; globalThis.__dirname = __dirname;' +
        ' globalThis.require = require; return (main) => main();',
      name + '-wrapper',
    );
    /^/.exec(''); // As node does: the RegExp statics are reset before the program runs.
    run(() => vm.runInThisContext(code, {
      filename: name,
      importModuleDynamically: vm.constants.USE_MAIN_CONTEXT_DEFAULT_LOADER,
    }));
  });
  stdin.push(workerData.program);
  stdin.push(null);
})();`;

// Node prints an uncaught error by `util.inspect` (its stack, headed by the source line when the
// error escaped the program's synchronous run) and anything else after "Uncaught".
function describe(error) {
  const shown = util.types.isNativeError(error) || error instanceof Error
    ? util.inspect(error)
    : `Uncaught ${util.inspect(error)}`;
  return Buffer.from(`${shown}\n\nNode.js ${process.version}\n`);
}

function run(mode, timeout, program) {
  return new Promise((resolve) => {
    const stdout = [];
    const stderr = [];
    let uncaught = null;
    let timedOut = false;
    let code = 0;
    const thread = new Worker(BOOTSTRAP, {
      eval: true,
      workerData: { program, commonjs: mode === 'c' },
      stdout: true,
      stderr: true,
      execArgv: [],
      // A thread's stack is 4 MB unless told otherwise, four times the 984 KB node gives its main
      // thread, so a recursion that overflows node's stack would pass here. 1.125 MB (192 KB of it
      // node keeps back) runs out a little before node does: 9771 frames to node's 10006 on node
      // 26, 9766 to 10002 on node 24.
      resourceLimits: { stackSizeMb: 1.125 },
    });
    // The reply goes out once the thread has exited and both of its streams have ended.
    let open = 3;
    const settle = () => {
      if (--open > 0) return;
      const out = Buffer.concat(stdout);
      const err = Buffer.concat(uncaught === null ? stderr : [...stderr, uncaught]);
      const head = Buffer.alloc(13);
      head.write('TNOD', 0, 'latin1');
      head.writeUInt8(timedOut ? 1 : 0, 4);
      head.writeInt32BE(code, 5);
      head.writeUInt32BE(out.length, 9);
      const tail = Buffer.alloc(4);
      tail.writeUInt32BE(err.length, 0);
      resolve(Buffer.concat([head, out, tail, err]));
    };
    // A process stops at its first uncaught error, but a thread still runs the microtasks already
    // queued. Its output and its error come over one port, in order, and its `exit` listeners run
    // before the error is sent, so what follows the error is what the process would not have done.
    const keep = (chunks) => (chunk) => {
      if (uncaught === null) chunks.push(chunk);
    };
    thread.stdout.on('data', keep(stdout)).on('end', settle);
    thread.stderr.on('data', keep(stderr)).on('end', settle);
    thread.on('error', (error) => {
      uncaught = describe(error);
    });
    // A thread that does not stop when terminated leaves this process unanswered, and the caller
    // replaces it.
    const guard = setTimeout(() => {
      timedOut = true;
      stderr.push(Buffer.from(`test-node: the program did not finish within ${timeout} ms\n`));
      thread.terminate();
    }, timeout);
    thread.on('exit', (exitCode) => {
      clearTimeout(guard);
      code = exitCode;
      settle();
    });
  });
}

let input = Buffer.alloc(0);
let busy = false;

function next() {
  if (busy || input.length < 9) return;
  const length = input.readUInt32BE(5);
  if (input.length < 9 + length) return;
  const mode = String.fromCharCode(input[0]);
  const timeout = input.readUInt32BE(1);
  const program = input.toString('utf8', 9, 9 + length);
  input = input.subarray(9 + length);
  busy = true;
  run(mode, timeout, program).then(
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

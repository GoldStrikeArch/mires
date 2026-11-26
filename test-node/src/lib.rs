//! Runs the JavaScript the workspace's unit tests check, as `node` would, in a few long-lived
//! node workers instead of one node process per program.
//!
//! A test names the command line it used to spawn ([`Node`]) and gets back what that process
//! would have produced ([`Output`]). Each program runs in a `worker_threads` thread of its own in
//! a worker, which `src/worker.js` describes: its own globals, event loop and exit code, evaluated
//! as node evaluates its stdin. So a check costs a thread, about ten milliseconds, where a node
//! start that loads the whole runtime costs tens of milliseconds alone and far more when a test
//! run starts them by the hundred.
//!
//! `RCGJS_TEST_NODE=spawn` spawns one node per program instead, exactly as the tests did before,
//! for debugging and for comparing the two. `RCGJS_TEST_NODE_TIMEOUT` (seconds, 60 by default)
//! bounds one program in a worker. A spawned node has no bound, as before.
//!
//! The unit tests inside `rcgjs-object/src` still spawn their node (seven programs a run, with
//! `node -e`, `node <path>` and `node --check`): every byte of that crate's source files, its
//! `#[cfg(test)]` code included, reaches the crate hash the backend dylib records, and that
//! dylib's SHA-256 keys every sysroot.

use std::io::{self, Read, Write};
use std::num::NonZeroUsize;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The worker, started as `node -e WORKER`.
const WORKER: &str = include_str!("worker.js");

/// How long past a program's timeout its worker may take to answer before it is killed (at most
/// the timeout again). The worker terminates a program that runs out of time and reports it,
/// unless the program's thread does not stop.
const GRACE: Duration = Duration::from_secs(5);

/// The largest timeout Node's `setTimeout` accepts: delays above `i32::MAX` milliseconds overflow
/// its signed 32-bit timer and are clamped to 1 ms.
const MAX_TIMEOUT_MS: u32 = i32::MAX as u32;

/// Default upper bound on concurrent workers in the shared pool.
const MAX_POOL_SIZE: NonZeroUsize = NonZeroUsize::new(4).expect("non-zero");

/// How long to wait for a worker's stderr reader to reach EOF after the worker exits.
const STDERR_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// Maximum size of a single stdout or stderr block accepted from a worker reply (64 MiB).
const MAX_BLOCK_BYTES: u32 = 64 * 1024 * 1024;

/// One node command line a test runs, with its program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node<'a> {
    /// `node -`, the program written to its stdin.
    Stdin(&'a str),
    /// `node --input-type=commonjs`, the program written to its stdin.
    StdinCommonJs(&'a str),
}

/// How node exited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    code: Option<i32>,
}

impl Status {
    /// Whether node exited with status 0.
    #[must_use]
    pub fn success(self) -> bool {
        self.code == Some(0)
    }

    /// The exit status, or `None` when node was killed or the program ran out of time.
    #[must_use]
    pub fn code(self) -> Option<i32> {
        self.code
    }
}

/// What the node process would have produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub status: Status,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `node` as `invocation` says and waits for it to finish.
///
/// # Errors
///
/// Only where node cannot be started, as when it is not installed. A test that is skipped
/// without node is skipped on this error, and a test that requires node panics on it.
pub fn run(invocation: Node<'_>) -> io::Result<Output> {
    if spawn_mode() {
        return spawn(invocation);
    }
    match Request::new(invocation, timeout()) {
        Some(request) => pool().run(&request),
        // A program of 4 GiB or more cannot be told to the worker, but node can still be given it.
        None => spawn(invocation),
    }
}

/// Whether `RCGJS_TEST_NODE=spawn` was requested, bypassing the worker pool.
fn spawn_mode() -> bool {
    static SPAWN: OnceLock<bool> = OnceLock::new();
    *SPAWN.get_or_init(|| std::env::var_os("RCGJS_TEST_NODE").is_some_and(|mode| mode == "spawn"))
}

/// One node process for `invocation`, spawned as the tests spawned it before the pool.
fn spawn(invocation: Node<'_>) -> io::Result<Output> {
    let output = match invocation {
        Node::Stdin(program) => piped("-", program)?,
        Node::StdinCommonJs(program) => piped("--input-type=commonjs", program)?,
    };
    Ok(Output {
        status: Status {
            code: output.status.code(),
        },
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

/// `node <arg>` with `program` written to its stdin.
fn piped(arg: &str, program: &str) -> io::Result<std::process::Output> {
    let mut child = Command::new("node")
        .arg(arg)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(program.as_bytes())?;
    }
    child.wait_with_output()
}

/// The bound on one program in a worker.
fn timeout() -> Duration {
    static TIMEOUT: OnceLock<Duration> = OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        let seconds = std::env::var("RCGJS_TEST_NODE_TIMEOUT")
            .ok()
            .and_then(|seconds| seconds.parse().ok())
            .filter(|&seconds| seconds > 0)
            .unwrap_or(60);
        Duration::from_secs(seconds)
    })
}

/// A request as the worker reads it (the mode, the timeout, and the program, length first), and
/// how long to wait for its reply.
struct Request {
    bytes: Vec<u8>,
    deadline: Duration,
}

impl Request {
    fn new(invocation: Node<'_>, timeout: Duration) -> Option<Self> {
        let (mode, payload) = match invocation {
            Node::Stdin(program) => (b's', program),
            Node::StdinCommonJs(program) => (b'c', program),
        };
        let millis = u32::try_from(timeout.as_millis())
            .unwrap_or(MAX_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);
        let length = u32::try_from(payload.len()).ok()?;
        let mut bytes = Vec::with_capacity(9 + payload.len());
        bytes.push(mode);
        bytes.extend_from_slice(&millis.to_be_bytes());
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(payload.as_bytes());
        let clamped = timeout.min(Duration::from_millis(u64::from(MAX_TIMEOUT_MS)));
        Some(Self {
            bytes,
            deadline: clamped.saturating_add(clamped.min(GRACE)),
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The workers of this test process: at most `size`, started when first needed.
struct Pool {
    state: Mutex<PoolState>,
    freed: Condvar,
    size: NonZeroUsize,
}

struct PoolState {
    idle: Vec<Worker>,
    live: usize,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let size =
            thread::available_parallelism().map_or(NonZeroUsize::MIN, |n| n.min(MAX_POOL_SIZE));
        Pool::new(size)
    })
}

impl Pool {
    fn new(size: NonZeroUsize) -> Self {
        Self {
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                live: 0,
            }),
            freed: Condvar::new(),
            size,
        }
    }

    fn run(&self, request: &Request) -> io::Result<Output> {
        let mut lease = self.checkout()?;
        if lease.worker().send(request).is_err() {
            // The worker had died while idle, so the program never reached it: try a fresh one.
            drop(lease);
            lease = self.checkout()?;
            if let Err(error) = lease.worker().send(request) {
                return Ok(lease
                    .worker()
                    .died(&format!("could not be sent the program: {error}")));
            }
        }
        // A program that answered, failed or not, has left nothing behind: its thread is gone. A
        // worker that did not answer is replaced.
        match lease.worker().receive(request.deadline) {
            WorkerReply::Reusable(output) => {
                lease.keep();
                Ok(output)
            }
            WorkerReply::Retired(output) => Ok(output),
        }
    }

    fn checkout(&self) -> io::Result<Lease<'_>> {
        let mut state = lock(&self.state);
        loop {
            if let Some(worker) = state.idle.pop() {
                return Ok(Lease {
                    pool: self,
                    worker: Some(worker),
                });
            }
            if state.live < self.size.get() {
                state.live += 1;
                drop(state);
                return match Worker::spawn() {
                    Ok(worker) => Ok(Lease {
                        pool: self,
                        worker: Some(worker),
                    }),
                    Err(error) => {
                        self.release();
                        Err(error)
                    }
                };
            }
            state = self
                .freed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// A worker is gone; another may be started.
    fn release(&self) {
        lock(&self.state).live -= 1;
        self.freed.notify_one();
    }
}

/// A worker checked out of the pool. Dropped without [`Lease::keep`], it retires its worker.
struct Lease<'a> {
    pool: &'a Pool,
    worker: Option<Worker>,
}

impl Lease<'_> {
    fn worker(&mut self) -> &mut Worker {
        self.worker
            .as_mut()
            .expect("a lease holds its worker until it ends")
    }

    /// Returns the worker to the pool for the next program.
    fn keep(mut self) {
        if let Some(worker) = self.worker.take() {
            lock(&self.pool.state).idle.push(worker);
            self.pool.freed.notify_one();
        }
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            drop(worker);
            self.pool.release();
        }
    }
}

/// Outcome of reading a worker's reply: either the worker answered cleanly and may return to the
/// pool, or it failed / timed out and must be retired.
enum WorkerReply {
    Reusable(Output),
    Retired(Output),
}

struct StderrState {
    bytes: Vec<u8>,
    eof: bool,
}

struct WorkerStderr {
    state: Mutex<StderrState>,
    eof: Condvar,
}

struct StderrEofGuard<'a>(&'a WorkerStderr);

impl Drop for StderrEofGuard<'_> {
    fn drop(&mut self) {
        lock(&self.0.state).eof = true;
        self.0.eof.notify_all();
    }
}

/// Drains a worker's stderr on a background thread and wakes waiters once the pipe hits EOF.
struct StderrCollector {
    shared: Arc<WorkerStderr>,
    reader: Option<JoinHandle<()>>,
}

impl StderrCollector {
    fn spawn(mut pipe: impl Read + Send + 'static) -> Self {
        let shared = Arc::new(WorkerStderr {
            state: Mutex::new(StderrState {
                bytes: Vec::new(),
                eof: false,
            }),
            eof: Condvar::new(),
        });
        let log = Arc::clone(&shared);
        let reader = thread::spawn(move || {
            let _notify_eof = StderrEofGuard(&log);
            let mut buffer = [0; 4096];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => lock(&log.state).bytes.extend_from_slice(&buffer[..read]),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });
        Self {
            shared,
            reader: Some(reader),
        }
    }

    /// Waits up to `timeout` for the reader thread to reach EOF and joins it if it did.
    fn finish(&mut self, timeout: Duration) {
        let Some(handle) = self.reader.take() else {
            return;
        };
        let (state, _) = self
            .shared
            .eof
            .wait_timeout_while(lock(&self.shared.state), timeout, |state| !state.eof)
            .unwrap_or_else(PoisonError::into_inner);
        let eof = state.eof;
        drop(state);
        if eof {
            let _ = handle.join();
        }
    }

    fn read_lossy(&self) -> String {
        String::from_utf8_lossy(&lock(&self.shared.state).bytes).into_owned()
    }
}

/// One node process running `worker.js`. Its replies are read on a thread of their own, so a
/// program that never finishes is noticed and the worker killed.
struct Worker {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<io::Result<Output>>,
    stderr: StderrCollector,
}

impl Worker {
    fn spawn() -> io::Result<Self> {
        let mut child = Command::new("node")
            .args(["-e", WORKER])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (Some(stdin), Some(mut stdout), Some(stderr_pipe)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("node started without its pipes"));
        };
        let (sender, replies) = mpsc::channel();
        thread::spawn(move || loop {
            let reply = read_reply(&mut stdout);
            let ended = reply.is_err();
            if sender.send(reply).is_err() || ended {
                break;
            }
        });
        // What the worker itself says goes into the message when it dies.
        let stderr = StderrCollector::spawn(stderr_pipe);
        Ok(Self {
            child,
            stdin,
            replies,
            stderr,
        })
    }

    fn send(&mut self, request: &Request) -> io::Result<()> {
        self.stdin.write_all(&request.bytes)?;
        self.stdin.flush()
    }

    /// The worker's answer, or the failure of a worker that gave none.
    fn receive(&mut self, deadline: Duration) -> WorkerReply {
        match self.replies.recv_timeout(deadline) {
            Ok(Ok(output)) => WorkerReply::Reusable(output),
            Ok(Err(error)) => {
                WorkerReply::Retired(self.died(&format!("stopped answering: {error}")))
            }
            Err(RecvTimeoutError::Disconnected) => {
                WorkerReply::Retired(self.died("stopped answering"))
            }
            Err(RecvTimeoutError::Timeout) => WorkerReply::Retired(failure(format!(
                "test-node: the program did not finish within {deadline:?}, so its worker was \
                 killed\n"
            ))),
        }
    }

    /// The failure of a program whose worker died under it, with what the worker said.
    fn died(&mut self, what: &str) -> Output {
        let _ = self.child.kill();
        let status = self.child.wait();
        // The last of what it said is still in the pipe; wait for the reader to reach EOF.
        self.stderr.finish(STDERR_DRAIN_TIMEOUT);
        let said = self.stderr.read_lossy();
        failure(format!("test-node: the worker {what} ({status:?})\n{said}"))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stderr.finish(STDERR_DRAIN_TIMEOUT);
    }
}

/// A program that produced no output and has no exit status, with `message` as its stderr.
fn failure(message: String) -> Output {
    Output {
        status: Status { code: None },
        stdout: Vec::new(),
        stderr: message.into_bytes(),
    }
}

/// Reads one reply: "TNOD", a byte that is 1 for a program that ran out of time, the exit status
/// (i32), then stdout and stderr, each a u32 length and the bytes, all big-endian.
fn read_reply(from: &mut impl Read) -> io::Result<Output> {
    let mut head = [0; 13];
    from.read_exact(&mut head)?;
    let [t, n, o, d, timed_out, c0, c1, c2, c3, l0, l1, l2, l3] = head;
    if [t, n, o, d] != *b"TNOD" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a reply that does not start with `TNOD`: did the program write to the worker's own \
             stdout?",
        ));
    }
    let stdout = read_block(from, u32::from_be_bytes([l0, l1, l2, l3]))?;
    let mut length = [0; 4];
    from.read_exact(&mut length)?;
    let stderr = read_block(from, u32::from_be_bytes(length))?;
    Ok(Output {
        status: Status {
            code: (timed_out == 0).then_some(i32::from_be_bytes([c0, c1, c2, c3])),
        },
        stdout,
        stderr,
    })
}

fn read_block(from: &mut impl Read, length: u32) -> io::Result<Vec<u8>> {
    if length > MAX_BLOCK_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("reply block length {length} exceeds limit of {MAX_BLOCK_BYTES} bytes"),
        ));
    }
    let expected = usize::try_from(length).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("reply block length {length} does not fit in usize"),
        )
    })?;
    let mut block = Vec::new();
    let mut remaining = expected;
    let mut chunk = [0u8; 8192];
    while remaining > 0 {
        let to_read = remaining.min(chunk.len());
        match from.read(&mut chunk[..to_read]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!(
                        "reply block truncated: expected {expected} bytes, got {}",
                        block.len()
                    ),
                ));
            }
            Ok(read) => {
                block.try_reserve(read).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::OutOfMemory,
                        "could not allocate buffer for reply block",
                    )
                })?;
                block.extend_from_slice(&chunk[..read]);
                remaining -= read;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(block)
}

#[cfg(test)]
mod tests {
    //! The pool against a spawned node, program by program: what each command line gives a
    //! program's top level, its exit status, the work node waits for, how a program fails, and
    //! that nothing of one program reaches the next.

    use super::*;

    fn lossy(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn pooled(pool: &Pool, invocation: Node<'_>, timeout: Duration) -> Output {
        let request = Request::new(invocation, timeout).expect("a short program");
        pool.run(&request).expect("node starts")
    }

    /// Runs `invocation` in the pool and in a node of its own, and asserts the two agree on the
    /// status and the stdout.
    fn same(invocation: Node<'_>) -> (Output, Output) {
        let pooled = pooled(pool(), invocation, super::timeout());
        let spawned = spawn(invocation).expect("node starts");
        assert_eq!(
            (pooled.status, lossy(&pooled.stdout)),
            (spawned.status, lossy(&spawned.stdout)),
            "{invocation:?}\n--- pooled stderr ---\n{}\n--- spawned stderr ---\n{}",
            lossy(&pooled.stderr),
            lossy(&spawned.stderr)
        );
        (pooled, spawned)
    }

    /// Both command lines, for `program`.
    fn both(program: &str) -> [Node<'_>; 2] {
        [Node::Stdin(program), Node::StdinCommonJs(program)]
    }

    const SCOPE: &str = r#"
var declared = 1;
function named() {}
console.log(JSON.stringify({
  declaredIsGlobal: globalThis.declared === 1,
  namedIsGlobal: typeof globalThis.named,
  thisIsGlobal: this === globalThis,
  bindings: [typeof require, typeof module, typeof exports, typeof globalThis.require],
  filename: typeof __filename === 'string' ? __filename : typeof __filename,
  dirname: typeof __dirname === 'string' ? __dirname : typeof __dirname,
  module: [module.id, module.filename, module.paths[0], typeof require.main],
  arguments: typeof arguments,
  sloppy: (function () { return this === globalThis; })(),
  command: [process.argv.slice(1), process.execArgv],
  globals: Object.getOwnPropertyNames(globalThis).sort(),
  bytes: new TextEncoder().encode('é') instanceof Uint8Array,
}));
console.log([1, 'two', { three: [3n] }], new Map([[1, 2]]));
"#;

    #[test]
    fn each_command_line_gives_the_top_level_what_node_gives_it() {
        for invocation in both(SCOPE) {
            same(invocation);
        }
    }

    #[test]
    fn the_exit_status_is_the_one_node_exits_with() {
        for program in [
            "process.exitCode = 3;",
            "console.log('a'); process.exit(7); console.log('b');",
            "if (typeof process !== 'undefined') process.exit(1);",
            "require('node:process').exitCode = 1;",
            "process.on('exit', () => { process.exitCode = 2; });",
            "process.exitCode = 5; throw new Error('thrown');",
        ] {
            for invocation in both(program) {
                let (pooled, _) = same(invocation);
                assert!(!pooled.status.success(), "{program}");
            }
        }
    }

    #[test]
    fn a_program_runs_until_nothing_is_left_that_node_would_wait_for() {
        for (program, fails) in [
            (
                "require('node:timers').setTimeout(() => { throw new Error('late'); }, 30);",
                true,
            ),
            (
                "require('node:fs').readFile(__filename, () => { throw new Error('fs'); });",
                true,
            ),
            (
                "const c = new MessageChannel();
                 c.port1.on('message', () => { c.port1.close(); throw new Error('port'); });
                 c.port2.postMessage(1);",
                true,
            ),
            (
                "WebAssembly.compile(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]))
                   .then(() => { throw new Error('wasm'); });",
                true,
            ),
            (
                "const t = setTimeout(() => console.log('refreshed'), 30);
                 setTimeout(() => t.refresh(), 15);",
                false,
            ),
            (
                "setTimeout(() => console.log('never'), 1e9).unref(); console.log('end');",
                false,
            ),
        ] {
            let (pooled, _) = same(Node::Stdin(program));
            assert_eq!(pooled.status.success(), !fails, "{program}");
        }
    }

    #[test]
    fn a_program_fails_as_node_exits_on_its_first_uncaught_error() {
        for (program, said) in [
            (
                "console.log(1); throw new Error('thrown');",
                "Error: thrown",
            ),
            (
                "console.log(1); queueMicrotask(() => { throw new Error('microtask'); });
                 queueMicrotask(() => console.log('never'));",
                "Error: microtask",
            ),
            (
                "console.log(1); Promise.reject(new Error('rejected'));
                 setTimeout(() => console.log('never'), 0);",
                "Error: rejected",
            ),
            (
                "console.log(1); setTimeout(() => { throw new Error('timer'); }, 5);",
                "Error: timer",
            ),
            ("console.log(1); throw 5;", "5"),
            // Node destroys stdin after the program has run, with what the program left of the
            // builtins.
            (
                "Function.prototype.call = null;",
                "destroy.call is not a function",
            ),
            // A thread that reports an uncaught error can still exit 0; the process exits 1.
            (
                "process.on('exit', () => { throw new Error('in exit'); });",
                "Error: in exit",
            ),
            (
                "queueMicrotask(() => { throw new Error('first'); });
                 queueMicrotask(() => process.exit(0));",
                "Error: first",
            ),
            ("let b = (;", "SyntaxError: Unexpected token ';'"),
            // A strict assignment to a name nothing declares, a ReferenceError that a `vm` context
            // lets through. (Node 24 names an error that crossed from a thread `ReferenceError [Error]`.)
            ("'use strict'; f = function () {};", "f is not defined"),
            (
                "'use strict'; class K { m = () => { g = () => 0; }; } new K().m();",
                "g is not defined",
            ),
            ("'use strict'; K = class {};", "K is not defined"),
            (
                "'use strict'; setTimeout(() => { h = function () {}; }, 1);",
                "h is not defined",
            ),
        ] {
            for invocation in both(program) {
                let (pooled, spawned) = same(invocation);
                assert_eq!(pooled.status.code(), Some(1), "{program}");
                for stderr in [&pooled.stderr, &spawned.stderr] {
                    assert!(lossy(stderr).contains(said), "{program}\n{}", lossy(stderr));
                }
            }
        }
        // A rejection handled before the microtasks drain is no failure, and a timer is waited for.
        for program in [
            "const p = Promise.reject(new Error('late'));
             queueMicrotask(() => p.catch(() => console.log('handled')));",
            "setTimeout(() => console.log('second'), 20); console.log('first');",
            "const t = setTimeout(() => console.log('never'), 10); clearTimeout(t);",
        ] {
            let (pooled, _) = same(Node::Stdin(program));
            assert!(pooled.status.success(), "{program}");
        }
    }

    #[test]
    fn a_recursion_too_deep_for_node_is_too_deep_here() {
        // Node's main thread runs out of stack at about 10000 of these frames.
        for depth in [9000, 10500] {
            let program = format!("let d = 0; function f() {{ if (++d < {depth}) f(); }} f();");
            let (pooled, _) = same(Node::Stdin(&program));
            assert_eq!(pooled.status.success(), depth < 10000, "{program}");
        }
    }

    #[test]
    fn nothing_of_one_program_reaches_the_next_on_its_worker() {
        let pool = Pool::new(NonZeroUsize::MIN);
        let pid = "require('node:process').pid";
        let first = pooled(
            &pool,
            Node::Stdin(&format!(
                "globalThis.leak = 1; var declared = 2; Array.prototype.leak = 3;
                 require('node:assert').strictEqual = () => {{}}; console.log({pid});"
            )),
            timeout(),
        );
        let second = pooled(
            &pool,
            Node::Stdin(&format!(
                "console.log({pid}, typeof leak, typeof declared, typeof [].leak);
                 require('node:assert').strictEqual(1, 2);"
            )),
            timeout(),
        );
        assert!(first.status.success(), "{}", lossy(&first.stderr));
        let worker = lossy(&first.stdout);
        assert_eq!(
            lossy(&second.stdout),
            format!("{} undefined undefined undefined\n", worker.trim())
        );
        // The first program's `strictEqual` is not the second's.
        assert_eq!(second.status.code(), Some(1), "{}", lossy(&second.stderr));
    }

    #[test]
    fn a_runaway_program_times_out_and_its_worker_goes_on() {
        let pool = Pool::new(NonZeroUsize::MIN);
        let pid = "console.log(require('node:process').pid);";
        for program in [
            "for (;;) {}",
            "(function again() { queueMicrotask(again); })();",
            "setInterval(() => {}, 10);",
        ] {
            let output = pooled(
                &pool,
                Node::Stdin(&format!("{pid} {program}")),
                Duration::from_millis(500),
            );
            assert_eq!(
                output.status.code(),
                None,
                "{program}\n{}",
                lossy(&output.stderr)
            );
            let next = pooled(&pool, Node::Stdin(pid), timeout());
            assert!(next.status.success(), "{}", lossy(&next.stderr));
            // The program's thread was stopped and the same worker ran the next one.
            assert_eq!(next.stdout, output.stdout, "{program}");
        }
    }

    #[test]
    fn large_timeouts_clamp_to_signed_32_bit_millis() {
        let encoded_millis = |duration: Duration| {
            let request = Request::new(Node::Stdin(""), duration).expect("empty program fits");
            u32::from_be_bytes(request.bytes[1..5].try_into().expect("4-byte timeout"))
        };

        assert_eq!(encoded_millis(Duration::from_secs(60)), 60_000);
        assert_eq!(
            encoded_millis(Duration::from_millis(u64::from(MAX_TIMEOUT_MS))),
            MAX_TIMEOUT_MS
        );
        assert_eq!(
            encoded_millis(Duration::from_millis(u64::from(MAX_TIMEOUT_MS) + 1)),
            MAX_TIMEOUT_MS
        );
        assert_eq!(
            encoded_millis(Duration::from_millis(u64::from(u32::MAX))),
            MAX_TIMEOUT_MS
        );
        assert_eq!(encoded_millis(Duration::MAX), MAX_TIMEOUT_MS);

        // Node clamps delays above `i32::MAX` ms to 1 ms; verify large timeouts do not trigger a
        // spurious 1 ms timeout in the worker.
        let pool = Pool::new(NonZeroUsize::MIN);
        for duration in [Duration::from_millis(u64::from(u32::MAX)), Duration::MAX] {
            let output = pooled(
                &pool,
                Node::Stdin("setTimeout(() => console.log('ok'), 25);"),
                duration,
            );
            assert!(output.status.success(), "{}", lossy(&output.stderr));
            assert_eq!(lossy(&output.stdout), "ok\n");
        }
    }

    #[test]
    fn read_block_guards_against_oversized_and_truncated_lengths() {
        let mut empty: &[u8] = &[];
        assert_eq!(read_block(&mut empty, 0).expect("empty block"), b"");

        for bad_length in [MAX_BLOCK_BYTES + 1, u32::MAX] {
            let mut empty: &[u8] = &[];
            let error = read_block(&mut empty, bad_length).expect_err("length exceeds limit");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }

        // A corrupt length within the cap must fail on EOF without allocating the claimed size.
        let mut short: &[u8] = b"short";
        let error = read_block(&mut short, MAX_BLOCK_BYTES).expect_err("truncated stream");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);

        // Exact block reads stop at the boundary and leave trailing bytes intact.
        let mut payload = vec![b'x'; 20_000];
        payload.extend_from_slice(b"tail");
        let mut reader = payload.as_slice();
        let block = read_block(&mut reader, 20_000).expect("multi-chunk block");
        assert_eq!(block.len(), 20_000);
        assert_eq!(reader, b"tail");

        // `read_reply` rejects an oversized length header without attempting a 4 GiB allocation.
        let mut corrupt_reply = Vec::new();
        corrupt_reply.extend_from_slice(b"TNOD");
        corrupt_reply.push(0);
        corrupt_reply.extend_from_slice(&0i32.to_be_bytes());
        corrupt_reply.extend_from_slice(&u32::MAX.to_be_bytes());
        let error = read_reply(&mut corrupt_reply.as_slice()).expect_err("corrupt stdout length");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn stderr_collector_wakes_on_eof_and_bounds_open_pipes() {
        struct ChannelReader {
            chunks: Receiver<Vec<u8>>,
            on_read: Option<mpsc::Sender<()>>,
        }

        impl Read for ChannelReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if let Some(notify) = &self.on_read {
                    let _ = notify.send(());
                }
                match self.chunks.recv() {
                    Ok(chunk) => {
                        let len = chunk.len().min(buf.len());
                        buf[..len].copy_from_slice(&chunk[..len]);
                        Ok(len)
                    }
                    Err(_) => Ok(0),
                }
            }
        }

        // Immediate EOF wakes `finish` and joins the reader thread.
        let (tx, rx) = mpsc::channel();
        let mut collector = StderrCollector::spawn(ChannelReader {
            chunks: rx,
            on_read: None,
        });
        tx.send(b"worker ".to_vec()).expect("reader alive");
        tx.send(b"failed\n".to_vec()).expect("reader alive");
        drop(tx);
        collector.finish(Duration::from_secs(30));
        assert!(collector.reader.is_none());
        assert!(lock(&collector.shared.state).eof);
        assert_eq!(collector.read_lossy(), "worker failed\n");

        // A pipe held open past the drain timeout does not hang `finish` and keeps partial output.
        let (tx, rx) = mpsc::channel();
        let (read_tx, read_rx) = mpsc::channel();
        let mut collector = StderrCollector::spawn(ChannelReader {
            chunks: rx,
            on_read: Some(read_tx),
        });
        tx.send(b"partial\n".to_vec()).expect("reader alive");
        // First notification: reader entered first `read()`. Second: first chunk is stored and
        // reader is now blocked waiting for the next chunk.
        read_rx.recv().expect("first read");
        read_rx.recv().expect("second read");
        collector.finish(Duration::from_millis(20));
        assert!(collector.reader.is_none());
        assert!(!lock(&collector.shared.state).eof);
        assert_eq!(collector.read_lossy(), "partial\n");
        // Release the reader thread so it exits cleanly before the test finishes.
        drop(tx);
        let shared = Arc::clone(&collector.shared);
        let (state, _) = shared
            .eof
            .wait_timeout_while(lock(&shared.state), Duration::from_secs(5), |state| {
                !state.eof
            })
            .unwrap_or_else(PoisonError::into_inner);
        assert!(state.eof);
    }

    #[test]
    fn node_and_output_derive_equality() {
        assert_eq!(Node::Stdin("1"), Node::Stdin("1"));
        assert_ne!(Node::Stdin("1"), Node::StdinCommonJs("1"));
        assert_ne!(Node::Stdin("1"), Node::Stdin("2"));

        let ok = Output {
            status: Status { code: Some(0) },
            stdout: b"hi\n".to_vec(),
            stderr: Vec::new(),
        };
        let same_ok = ok.clone();
        let diff_ok = Output {
            stdout: b"bye\n".to_vec(),
            ..ok.clone()
        };
        assert_eq!(ok, same_ok);
        assert_ne!(ok, diff_ok);
    }
}

//! Runs the JavaScript the workspace's unit tests check, as `node` would, in a few long-lived
//! node workers instead of one node process per program.
//!
//! A test names the command line it used to spawn ([`Node`]) and gets back what that process
//! would have produced ([`Output`]). Each program runs in a fresh `vm` context of a worker, which
//! `src/worker.js` describes, with what it reproduces of each command line. So a check costs a
//! context, a few milliseconds, where a node start that loads the whole runtime costs tens of
//! milliseconds alone and far more when a test run starts them by the hundred.
//!
//! `RCGJS_TEST_NODE=spawn` spawns one node per program instead, exactly as the tests did before,
//! for debugging and for comparing the two. `RCGJS_TEST_NODE_TIMEOUT` (seconds, 60 by default)
//! bounds one program in a worker. A spawned node has no bound, as before.
//!
//! The unit tests inside `rcgjs-object/src` still spawn their node (seven programs a run): every
//! byte of that crate's source files, its `#[cfg(test)]` code included, reaches the crate hash
//! the backend dylib records, and that dylib's SHA-256 keys every sysroot. A change that edits
//! those files anyway can move them onto [`run`] at no further cost.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The worker, started as `node -e WORKER`.
const WORKER: &str = include_str!("worker.js");

/// How long past a program's timeout its worker may take to answer before it is killed (at most
/// the timeout again). The worker reports a program that runs out of time itself, unless the
/// program keeps its event loop busy, as an endless chain of microtasks does.
const GRACE: Duration = Duration::from_secs(5);

/// How many programs a worker runs before it is replaced. Each leaves a context behind for the
/// collector, and a worker that runs thousands holds hundreds of megabytes between collections.
const PROGRAMS_PER_WORKER: u32 = 200;

/// One node command line a test runs, with its program.
#[derive(Clone, Copy, Debug)]
pub enum Node<'a> {
    /// `node -`, the program written to its stdin.
    Stdin(&'a str),
    /// `node --input-type=commonjs`, the program written to its stdin.
    StdinCommonJs(&'a str),
    /// `node -e <program>`.
    Eval(&'a str),
    /// `node <path>`.
    File(&'a Path),
    /// `node --check <path>`, a syntax check that runs nothing.
    Check(&'a Path),
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
#[derive(Clone, Debug)]
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
    if std::env::var_os("RCGJS_TEST_NODE").is_some_and(|mode| mode == "spawn") {
        return spawn(invocation);
    }
    match Request::new(invocation, timeout()) {
        Some(request) => pool().run(&request),
        // A path that is not UTF-8 cannot be told to the worker, but node can still be given it.
        None => spawn(invocation),
    }
}

/// One node process for `invocation`, spawned as the tests spawned it before the pool.
fn spawn(invocation: Node<'_>) -> io::Result<Output> {
    let output = match invocation {
        Node::Stdin(program) => piped("-", program)?,
        Node::StdinCommonJs(program) => piped("--input-type=commonjs", program)?,
        Node::Eval(program) => Command::new("node").args(["-e", program]).output()?,
        Node::File(path) => Command::new("node").arg(path).output()?,
        Node::Check(path) => Command::new("node").arg("--check").arg(path).output()?,
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

/// A request as the worker reads it (the mode, the timeout, and the payload, length first), and
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
            Node::Eval(program) => (b'e', program),
            Node::File(path) => (b'f', path.to_str()?),
            Node::Check(path) => (b'k', path.to_str()?),
        };
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let length = u32::try_from(payload.len()).ok()?;
        let mut bytes = Vec::with_capacity(9 + payload.len());
        bytes.push(mode);
        bytes.extend_from_slice(&millis.to_be_bytes());
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(payload.as_bytes());
        Some(Self {
            bytes,
            deadline: timeout + timeout.min(GRACE),
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
    size: usize,
}

struct PoolState {
    idle: Vec<Worker>,
    live: usize,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        state: Mutex::new(PoolState {
            idle: Vec::new(),
            live: 0,
        }),
        freed: Condvar::new(),
        size: thread::available_parallelism().map_or(1, |n| n.get().min(4)),
    })
}

impl Pool {
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
        let worker = lease.worker();
        let output = worker.receive(request.deadline);
        worker.programs += 1;
        // A failed program may have left work running in its worker, so only a clean one's worker
        // is used again.
        if output.status.success() && worker.programs < PROGRAMS_PER_WORKER {
            lease.keep();
        }
        Ok(output)
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
            if state.live < self.size {
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

/// One node process running `worker.js`. Its replies are read on a thread of their own, so a
/// program that never finishes is noticed and the worker killed.
struct Worker {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<io::Result<Output>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    stderr_reader: JoinHandle<()>,
    programs: u32,
}

impl Worker {
    fn spawn() -> io::Result<Self> {
        let mut child = Command::new("node")
            .args(["-e", WORKER])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (Some(stdin), Some(mut stdout), Some(mut stderr_pipe)) =
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
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&stderr);
        let stderr_reader = thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(read @ 1..) = stderr_pipe.read(&mut buffer) {
                lock(&log).extend_from_slice(&buffer[..read]);
            }
        });
        Ok(Self {
            child,
            stdin,
            replies,
            stderr,
            stderr_reader,
            programs: 0,
        })
    }

    fn send(&mut self, request: &Request) -> io::Result<()> {
        self.stdin.write_all(&request.bytes)?;
        self.stdin.flush()
    }

    fn receive(&mut self, deadline: Duration) -> Output {
        match self.replies.recv_timeout(deadline) {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => self.died(&format!("stopped answering: {error}")),
            Err(RecvTimeoutError::Disconnected) => self.died("stopped answering"),
            Err(RecvTimeoutError::Timeout) => failure(format!(
                "test-node: the program did not finish within {deadline:?}, so its worker was \
                 killed\n"
            )),
        }
    }

    /// The failure of a program whose worker died under it, with what the worker said.
    fn died(&mut self, what: &str) -> Output {
        let _ = self.child.kill();
        let status = self.child.wait();
        // The last of what it said is still in the pipe; give the reader a moment for it.
        for _ in 0..100 {
            if self.stderr_reader.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let said = String::from_utf8_lossy(&lock(&self.stderr)).into_owned();
        failure(format!("test-node: the worker {what} ({status:?})\n{said}"))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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
    let mut block = vec![0; length as usize];
    from.read_exact(&mut block)?;
    Ok(block)
}

#[cfg(test)]
mod tests {
    //! The pool against a spawned node, program by program: what each command line gives a
    //! program's top level, how a program fails, and that nothing of one program reaches the next.

    use super::*;
    use std::path::PathBuf;

    fn lossy(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn pooled(invocation: Node<'_>, timeout: Duration) -> Output {
        let request = Request::new(invocation, timeout).expect("a UTF-8 request");
        pool().run(&request).expect("node starts")
    }

    /// Runs `invocation` in the pool and in a node of its own, and asserts the two agree on the
    /// status and the stdout.
    fn same(invocation: Node<'_>) -> (Output, Output) {
        let pooled = pooled(invocation, super::timeout());
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

    fn file(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("test-node-{}-{name}", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    const SCOPE: &str = r#"
var declared = 1;
function named() {}
console.log(JSON.stringify({
  declaredIsGlobal: globalThis.declared === 1,
  namedIsGlobal: typeof globalThis.named,
  thisIsGlobal: this === globalThis,
  thisIsExports: typeof module === 'object' && this === module.exports,
  bindings: [typeof require, typeof module, typeof exports, typeof globalThis.require],
  filename: typeof __filename === 'string' ? __filename : typeof __filename,
  dirname: typeof __dirname === 'string' ? __dirname : typeof __dirname,
  arguments: typeof arguments,
  strict: (function () { return this === undefined; })(),
  used: [typeof console, typeof queueMicrotask, typeof setTimeout, typeof TextEncoder,
         typeof TextDecoder, typeof btoa],
  ownBytes: new TextEncoder().encode('é') instanceof Uint8Array,
  decoded: new TextDecoder('utf-8', { ignoreBOM: true }).decode(new Uint8Array([0xef, 0xbb, 0xbf, 0x61])),
  assert: typeof require('node:assert/strict').deepEqual,
  inspected: require('node:util').format('%o', [1, { a: 'b' }]),
}));
console.log([1, 'two', { three: [3n] }], new Map([[1, 2]]));
"#;

    #[test]
    fn each_command_line_gives_the_top_level_what_node_gives_it() {
        for invocation in [
            Node::Stdin(SCOPE),
            Node::StdinCommonJs(SCOPE),
            Node::Eval(SCOPE),
        ] {
            same(invocation);
        }
        let path = file("scope.js", SCOPE);
        same(Node::File(&path));
        std::fs::remove_file(path).unwrap();
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
            ("let b = (;", "SyntaxError: Unexpected token ';'"),
        ] {
            let (pooled, spawned) = same(Node::Stdin(program));
            assert_eq!(pooled.status.code(), Some(1), "{program}");
            for stderr in [&pooled.stderr, &spawned.stderr] {
                assert!(lossy(stderr).contains(said), "{program}\n{}", lossy(stderr));
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
    fn nothing_of_one_program_reaches_the_next_on_its_worker() {
        let pid = "require('node:process').pid";
        let mut shared = 0;
        for _ in 0..8 {
            let first = pooled(
                Node::Stdin(&format!(
                    "globalThis.leak = 1; var declared = 2; String.prototype.leak = 3;
                     console.log({pid});"
                )),
                timeout(),
            );
            let second = pooled(
                Node::Stdin(&format!(
                    "console.log({pid}, typeof leak, typeof declared, typeof ''.leak);"
                )),
                timeout(),
            );
            assert!(first.status.success() && second.status.success());
            let second = lossy(&second.stdout);
            let (worker, seen) = second.trim().split_once(' ').unwrap();
            assert_eq!(seen, "undefined undefined undefined");
            shared += usize::from(lossy(&first.stdout).trim() == worker);
        }
        assert!(shared > 0, "no two of the programs ran on one worker");
    }

    #[test]
    fn a_syntax_check_parses_as_node_check_does_and_runs_nothing() {
        for (name, text, valid) in [
            ("valid.js", "console.log('never printed'); let a = 1;", true),
            ("return.js", "return 5;", true),
            ("undefined.js", "let undefined = 1;", true),
            ("broken.js", "let a = 1;\nlet b = (;\n", false),
        ] {
            let path = file(name, text);
            let (pooled, spawned) = same(Node::Check(&path));
            assert_eq!(pooled.status.success(), valid, "{name}");
            // The location, the line, the caret and the message, as node prints them.
            let head = |stderr: &[u8]| lossy(stderr).lines().take(5).collect::<Vec<_>>().join("\n");
            assert_eq!(head(&pooled.stderr), head(&spawned.stderr), "{name}");
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn a_runaway_program_times_out_and_its_worker_is_replaced() {
        for program in [
            "for (;;) {}",
            "(function again() { queueMicrotask(again); })();",
            "setInterval(() => {}, 10);",
        ] {
            let output = pooled(Node::Stdin(program), Duration::from_millis(500));
            assert_eq!(
                output.status.code(),
                None,
                "{program}\n{}",
                lossy(&output.stderr)
            );
            let next = pooled(Node::Stdin("console.log('next')"), timeout());
            assert!(next.status.success(), "{}", lossy(&next.stderr));
            assert_eq!(next.stdout, b"next\n");
        }
    }
}

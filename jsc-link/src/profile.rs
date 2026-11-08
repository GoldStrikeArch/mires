//! Opt-in compile-cost instrumentation: `RCGJS_COST_PROFILE=1`.
//!
//! Observation only. Nothing here is read by a decision, so the emitted program is the same with
//! the variable set or unset, and when it is unset every hook is one cached `bool` test. The link
//! step is single-threaded, so the figures live in thread-local `Cell`s rather than atomics.
//!
//! At the end of a link, [`dump`] writes one `rcgjs-cost-profile: key=value` line per figure to
//! stderr. The lane sidecar carries the same keys as a flat `cost_profile` object ([`json`]),
//! snapshotted when the sidecar is written (before `phase.link.total` closes).
//!
//! * `phase.<name>.ms` / `phase.<name>.calls`: wall time summed over every entry of a [`scope`].
//!   Scopes nest, so an outer phase includes its inner ones.
//! * [`Counter`] keys: plain event counts and byte totals.
//! * `rss_peak_kb` (Linux, `VmHWM`) or `rss_end_kb` (elsewhere, `ps` at the time of the call):
//!   resident memory. A true peak elsewhere needs `getrusage`, which is `unsafe`.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::sync::OnceLock;
use std::time::Instant;

/// Whether `RCGJS_COST_PROFILE=1` is set. Read once per process.
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("RCGJS_COST_PROFILE").is_ok_and(|value| value == "1"))
}

macro_rules! counters {
    ($($variant:ident => $key:literal,)*) => {
        /// One event count or byte total, named by its output key.
        #[derive(Clone, Copy)]
        pub(crate) enum Counter {
            $($variant,)*
        }
        const KEYS: &[&str] = &[$($key,)*];
    };
}

counters! {
    // `plan::cost::lower_item`, in total and by the caller that priced with it.
    LowerItem => "lower_item.total",
    LowerReprProbe => "lower_item.repr_probe",
    LowerReprProgram => "lower_item.repr_program",
    LowerPhysical => "lower_item.physical",
    LowerInline => "lower_item.inline",
    // Phase C's own lowerings (`lane::lower_all`), which do not go through `cost::lower_item`.
    LowerFinal => "lower.final",
    // Whole-program trials: representation `program_cost`, physical `physical_program_metrics`.
    TrialReprProgram => "trial.repr_program",
    TrialPhysical => "trial.physical",
    // `physical_cost::program_metrics`: calls, and the bytes fed to EACH of gzip and brotli.
    ProgramMetrics => "program_metrics.calls",
    CompressInputBytes => "program_metrics.input_bytes",
    // `lower::fuse::plan`'s catch-binding reservation of every interned string.
    CatchReserveCalls => "catch_reserve.calls",
    CatchReserveStrings => "catch_reserve.strings",
    // Re-parses of the bundled `runtime/shim.js`.
    ShimMembers => "shim.members_parses",
    ShimClosure => "shim.closure_calls",
    // The physical economics cache.
    CacheKeyCalls => "physical_cache.key_calls",
    CacheKeyBytes => "physical_cache.key_bytes",
    CacheHits => "physical_cache.hits",
    CacheMisses => "physical_cache.misses",
}

thread_local! {
    static COUNTS: [Cell<u64>; KEYS.len()] = const { [const { Cell::new(0) }; KEYS.len()] };
    /// `(name, nanoseconds, entries)`, in the order each phase first finished.
    static PHASES: RefCell<Vec<(&'static str, u64, u64)>> = const { RefCell::new(Vec::new()) };
}

/// Adds one to `counter`.
pub(crate) fn bump(counter: Counter) {
    add(counter, 1);
}

/// Adds `amount` to `counter`.
pub(crate) fn add(counter: Counter, amount: usize) {
    if enabled() {
        COUNTS.with(|counts| {
            let cell = &counts[counter as usize];
            cell.set(cell.get().saturating_add(amount as u64));
        });
    }
}

/// A running phase timer; its wall time is recorded under its name when it is dropped.
pub(crate) struct Scope(Option<(&'static str, Instant)>);

/// Starts timing `name` until the returned guard drops.
pub(crate) fn scope(name: &'static str) -> Scope {
    Scope(enabled().then(|| (name, Instant::now())))
}

/// Runs `f` inside a [`scope`] named `name`.
pub(crate) fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let _scope = scope(name);
    f()
}

impl Drop for Scope {
    fn drop(&mut self) {
        let Some((name, start)) = self.0.take() else {
            return;
        };
        let nanos = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        PHASES.with_borrow_mut(|phases| {
            match phases.iter_mut().find(|(phase, _, _)| *phase == name) {
                Some((_, total, entries)) => {
                    *total = total.saturating_add(nanos);
                    *entries += 1;
                }
                None => phases.push((name, nanos, 1)),
            }
        });
    }
}

/// Every figure as `(key, value)`, the value already a JSON number.
fn figures() -> Vec<(String, String)> {
    let mut figures = Vec::new();
    PHASES.with_borrow(|phases| {
        for (name, nanos, entries) in phases {
            figures.push((
                format!("phase.{name}.ms"),
                format!("{:.3}", *nanos as f64 / 1e6),
            ));
            figures.push((format!("phase.{name}.calls"), entries.to_string()));
        }
    });
    COUNTS.with(|counts| {
        for (key, count) in KEYS.iter().zip(counts) {
            figures.push(((*key).to_owned(), count.get().to_string()));
        }
    });
    if let Some((key, kib)) = resident_kib() {
        figures.push((key.to_owned(), kib.to_string()));
    }
    figures
}

/// Peak resident memory where the OS reports it without `unsafe` (Linux), else the current
/// resident size from `ps`. `None` when neither is available.
fn resident_kib() -> Option<(&'static str, u64)> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        let peak = status
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok());
        if let Some(peak) = peak {
            return Some(("rss_peak_kb", peak));
        }
    }
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let rss = String::from_utf8(output.stdout).ok()?.trim().parse().ok()?;
    Some(("rss_end_kb", rss))
}

/// The figures as one flat JSON object, for the lane sidecar's `cost_profile`.
pub(crate) fn json() -> String {
    let mut out = String::from("{");
    for (index, (key, value)) in figures().into_iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{key}\":{value}");
    }
    out.push('}');
    out
}

/// Writes every figure to stderr, one `rcgjs-cost-profile: key=value` line each. A no-op unless
/// profiling is enabled.
pub(crate) fn dump() {
    if !enabled() {
        return;
    }
    let mut out = String::new();
    for (key, value) in figures() {
        let _ = writeln!(out, "rcgjs-cost-profile: {key}={value}");
    }
    eprint!("{out}");
}

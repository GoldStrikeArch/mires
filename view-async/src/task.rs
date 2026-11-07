//! The task table and the poll loop.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use view_abi::{microtask, owner, with_owner, Owner};

/// One spawned future and everything needed to resume it.
struct Task {
    /// The future itself, boxed so the table holds one type however many shapes are spawned, and
    /// pinned because that is what `poll` takes.
    ///
    /// The `RefCell` is what makes a re-entrant poll a refusal rather than aliasing: a wake that
    /// arrives during a poll re-schedules through [`Task::scheduled`], and a poll that somehow
    /// re-entered anyway finds the borrow taken and returns.
    future: RefCell<Pin<Box<dyn Future<Output = ()>>>>,
    /// The owner captured at [`spawn`], before the first poll.
    owner: Owner,
    /// Whether a poll is already queued for this task.
    ///
    /// Without it a future woken in a loop queues a microtask per wake. With it, any number of
    /// wakes between two polls cost one poll.
    scheduled: Cell<bool>,
}

/// Every live task, indexed by the id its waker carries.
///
/// A `RawWaker`'s data is a `*const ()`, and handing out a pointer into the heap would ask this
/// value model for a stable address, which is the part of it with the miscompile history. A task
/// id is a `usize`, and a `usize` is a number here, so the waker carries an index into this table
/// instead and [`waker_for`] never makes a pointer to anything.
///
/// A slot emptied by a finished task is reused, so a page that spawns per keystroke does not grow
/// a table per keystroke.
static mut TASKS: Vec<Option<Rc<Task>>> = Vec::new();

/// Runs `f` with the table.
///
/// The one place the table is reached, so the rule that makes it sound is stated once: `f` never
/// calls back into this function and never runs a future. Every caller below takes what it needs
/// out of the table (an `Rc`, which is a clone rather than a borrow) and drops the borrow before
/// anything else happens.
fn with_table<R>(f: impl FnOnce(&mut Vec<Option<Rc<Task>>>) -> R) -> R {
    // SAFETY: the host is single threaded, so there is no other thread to race with, and the
    // borrow does not escape `f`, which by the rule above cannot re-enter.
    let table = unsafe { &mut *(&raw mut TASKS) };
    f(table)
}

/// Puts `task` in the table and answers the id it went in at.
///
/// The free slot is found by index rather than with `position`, because iterating a `Vec` that has
/// never allocated reaches the backend's provenance guard: `Vec::new()`'s pointer is the alignment
/// as a bare address, and the zero-length `add` that builds the iterator's end pointer offsets from
/// an address with no provenance. A `Vec` that has allocated and been emptied iterates fine, and so
/// does an empty subslice of a real buffer; it is only the never-allocated one, which is exactly
/// what the table is on the first spawn. `build/logs/wave5-backend-report.md` records the probe.
fn install(task: Rc<Task>) -> usize {
    with_table(|table| {
        let mut id = 0;
        while id < table.len() && table[id].is_some() {
            id += 1;
        }
        match id == table.len() {
            true => table.push(Some(task)),
            false => table[id] = Some(task),
        }
        id
    })
}

/// The task `id` names, or `None` for one that has finished.
fn lookup(id: usize) -> Option<Rc<Task>> {
    with_table(|table| table.get(id).and_then(Option::as_ref).map(Rc::clone))
}

/// Frees `id`'s slot.
fn forget(id: usize) {
    with_table(|table| {
        if let Some(slot) = table.get_mut(id) {
            *slot = None;
        }
    });
}

/// Runs `future` to completion, resuming it in microtasks.
///
/// The owner is captured HERE, before the first poll, because it is the last moment one exists:
/// the runtime's `Owner` is a module-level global restored in a synchronous `finally`, so by the
/// time a continuation runs there is none (`CONTRACT-DOM` 14.3). Every resume re-enters it.
///
/// The first poll is a microtask too, not a synchronous call. Spawning is only ever reached from
/// an event handler or an effect, and deferring the first poll makes the future's body start
/// outside whatever computation spawned it rather than inside it, which is one less way for a
/// handler to end up owning work it did not ask to own.
pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    let task = Rc::new(Task {
        future: RefCell::new(Box::pin(future)),
        owner: owner(),
        scheduled: Cell::new(false),
    });
    schedule(install(task));
}

/// Queues one poll of `id`, unless one is already queued.
fn schedule(id: usize) {
    let Some(task) = lookup(id) else { return };
    if task.scheduled.replace(true) {
        return;
    }
    microtask(move || step(id));
}

/// One scheduled poll: re-enter the owner, then poll.
///
/// The flag is cleared BEFORE the poll, so a future that wakes itself while being polled queues
/// the next poll rather than being dropped on the floor.
fn step(id: usize) {
    let Some(task) = lookup(id) else { return };
    task.scheduled.set(false);
    with_owner(task.owner, move || poll_once(id));
}

/// Polls `id` once, and forgets it when it is done.
fn poll_once(id: usize) {
    let Some(task) = lookup(id) else { return };
    let waker = waker_for(id);
    let mut context = Context::from_waker(&waker);
    // A poll already in progress means something re-entered; the wake that got us here has
    // already set the flag, so the outstanding poll will be followed by another.
    let Ok(mut future) = task.future.try_borrow_mut() else { return };
    if let Poll::Ready(()) = future.as_mut().poll(&mut context) {
        drop(future);
        forget(id);
    }
}

/// The waker vtable. Cloning a waker copies the id, waking it schedules a poll, and dropping one
/// does nothing: the table owns the task, and a waker is only ever a number.
static VTABLE: RawWakerVTable =
    RawWakerVTable::new(clone_waker, wake_task, wake_task, drop_waker);

fn clone_waker(data: *const ()) -> RawWaker {
    RawWaker::new(data, &VTABLE)
}

fn wake_task(data: *const ()) {
    schedule(data as usize);
}

fn drop_waker(_: *const ()) {}

/// A waker that schedules task `id`.
fn waker_for(id: usize) -> Waker {
    // SAFETY: the vtable's four functions match `RawWaker`'s contract. `clone` answers a waker
    // for the same id, `wake` and `wake_by_ref` schedule that id and nothing else, and `drop`
    // frees nothing because the data is a number rather than a pointer to anything.
    unsafe { Waker::from_raw(RawWaker::new(id as *const (), &VTABLE)) }
}

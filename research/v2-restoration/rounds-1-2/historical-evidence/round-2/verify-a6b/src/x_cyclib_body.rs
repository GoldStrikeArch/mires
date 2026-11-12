// The body of the dependency crate: statics that refer to each other, used from another crate.
pub struct Node { pub id: u32, pub next: &'static Node, pub f: fn() -> u32 }
pub static LA: Node = Node { id: 1, next: &LB, f: lib_fn };
pub static LB: Node = Node { id: 2, next: &LA, f: lib_fn };
pub static LS: Node = Node { id: 3, next: &LS, f: lib_fn };
pub static LTAB: [&Node; 3] = [&LS, &LB, &LA];
pub fn lib_fn() -> u32 { LA.next.id * 10 + LS.next.next.id }
#[inline(always)]
pub fn inl() -> &'static Node { &LB }
pub fn generic<T: Copy>(t: T) -> (T, &'static Node) { (t, LB.next) }

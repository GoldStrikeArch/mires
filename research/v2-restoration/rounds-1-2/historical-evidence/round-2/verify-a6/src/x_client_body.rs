// The body of the client crate: its own cycle, whose members name the dependency's cycle.
struct Hold { id: u32, n: &'static cyclib::Node, peer: &'static Hold, tab: &'static [&'static cyclib::Node; 3] }
static CA: Hold = Hold { id: 11, n: &cyclib::LA, peer: &CB, tab: &cyclib::LTAB };
static CB: Hold = Hold { id: 12, n: &cyclib::LS, peer: &CA, tab: &cyclib::LTAB };
static OWN: cyclib::Node = cyclib::Node { id: 20, next: &OWN2, f: cyclib::lib_fn };
static OWN2: cyclib::Node = cyclib::Node { id: 21, next: &OWN, f: client_fn };
fn client_fn() -> u32 { OWN.next.id + cyclib::LB.next.id * 100 }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(CA.n.next.id + CA.peer.n.next.next.id * 10 + CB.peer.peer.id * 100);
    print_bool(core::ptr::eq(CA.n.next.next, &cyclib::LA));
    print_bool(core::ptr::eq(CB.n.next, &cyclib::LS));
    print_bool(core::ptr::eq(CA.tab[1], cyclib::inl()));
    print_bool(core::ptr::eq(cyclib::generic(5u8).1, &cyclib::LA));
    print_u32((cyclib::LA.f)() + (OWN.next.f)() * 1000);
    print_u32((OWN.f)() + OWN2.next.next.id * 1000);
    print_u32(cyclib::LTAB[0].next.id + cyclib::LTAB[2].next.next.id * 10);
    print_bool(core::ptr::eq(CA.peer.peer, &CA));
}

use bitflags::bitflags;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    struct Perms: u32 {
        const READ    = 0b0001;
        const WRITE   = 0b0010;
        const EXEC    = 0b0100;
        const ADMIN   = 0b1000;
        const RW      = Self::READ.bits() | Self::WRITE.bits();
        const ALL     = Self::READ.bits() | Self::WRITE.bits() | Self::EXEC.bits() | Self::ADMIN.bits();
    }
}

#[test]
fn set_algebra_and_predicates() {
    let rw = Perms::READ | Perms::WRITE;
    assert_eq!(rw, Perms::RW);
    assert!(rw.contains(Perms::READ));
    assert!(rw.intersects(Perms::WRITE | Perms::EXEC));
    assert!(!rw.contains(Perms::EXEC));
    assert_eq!(Perms::ALL - rw, Perms::EXEC | Perms::ADMIN);
    assert_eq!(rw ^ (Perms::WRITE | Perms::EXEC), Perms::READ | Perms::EXEC);
    assert_eq!(!rw, Perms::EXEC | Perms::ADMIN);
}

#[test]
fn mutation_and_bits_conversions() {
    let mut p = Perms::empty();
    assert!(p.is_empty());
    p.insert(Perms::READ);
    p.set(Perms::EXEC, true);
    p.toggle(Perms::WRITE);
    assert_eq!(p, Perms::READ | Perms::WRITE | Perms::EXEC);
    p.remove(Perms::WRITE);
    assert_eq!(p.bits(), 0b0101);

    assert_eq!(Perms::from_bits(0b0101), Some(Perms::READ | Perms::EXEC));
    assert_eq!(Perms::from_bits(0b1_0001), None);
    assert_eq!(Perms::from_bits_truncate(0b1_0001), Perms::READ);
    assert_eq!(Perms::from_bits_retain(0b1_0001).bits(), 0b1_0001);
}

#[test]
fn iteration_and_formatting() {
    let p = Perms::READ | Perms::EXEC | Perms::ADMIN;
    let bits: Vec<u32> = p.iter().map(|f| f.bits()).collect();
    assert_eq!(bits, vec![0b0001, 0b0100, 0b1000]);

    let names: Vec<&str> = p.iter_names().map(|(name, _)| name).collect();
    assert_eq!(names, vec!["READ", "EXEC", "ADMIN"]);
    assert_eq!(format!("{p:04b}"), "1101");
    assert_eq!(format!("{p:x}"), "d");
}

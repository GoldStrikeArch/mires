pub struct DependencyValue {
    number: i32,
}

#[inline(never)]
fn read(value: &DependencyValue) -> i32 {
    value.number
}

pub fn rejected_dependency_allocation() -> i32 {
    let value = DependencyValue { number: 7 };
    read(&value)
}

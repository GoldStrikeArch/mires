enum Op {
    Add(i32),
    Scale(i32),
    Replace(i32),
}

struct Summary {
    value: i32,
    accepted: u32,
}

fn evaluate<const N: usize>(ops: [Op; N], mut keep: impl FnMut(i32) -> bool) -> Summary {
    let mut summary = Summary {
        value: 2,
        accepted: 0,
    };

    for op in ops {
        let candidate = match op {
            Op::Add(value) => summary.value.wrapping_add(value),
            Op::Scale(value) => summary.value.wrapping_mul(value),
            Op::Replace(value) => value,
        };

        if keep(candidate) {
            summary.value = candidate;
            summary.accepted += 1;
        }
    }

    summary
}

fn main() {
    let minimum = 5;
    let mut checked = 0;
    let summary = evaluate(
        [
            Op::Add(4),
            Op::Scale(3),
            Op::Replace(7),
            Op::Add(-8),
            Op::Scale(0),
        ],
        |value| {
            checked += 1;
            value >= minimum && value % 2 == 0
        },
    );

    println!("{} {} {}", summary.value, summary.accepted, checked);
}

struct Guard(i32);
impl Drop for Guard { fn drop(&mut self) { println!("dropped"); println!("{}", self.0); } }
#[inline(never)] fn total(words: &[u64]) -> u64 { words[2] + words[3] }
#[inline(never)] fn ones(words: &[u64]) -> u32 { words[2].count_ones() + words[3].count_ones() }
#[inline(never)] fn check(rows: &[[u64; 2]], use_ones: bool) {
    let guard = Guard(7);
    let flat = rows.as_flattened();
    let r = if use_ones { ones(flat) as i32 } else { total(flat) as i32 };
    println!("{r}");
    drop(guard);
}
fn main() { let rows = [[1u64, 2], [3, 4]]; check(&rows, false); check(&rows, true); }

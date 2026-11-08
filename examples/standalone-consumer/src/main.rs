fn main() {
    let script = include_str!(concat!(env!("OUT_DIR"), "/client.js"));
    assert!(script.contains("sum_squares"));
    println!("compiled client: {} bytes", script.len());
    println!("{}", concat!(env!("OUT_DIR"), "/client.js"));
}

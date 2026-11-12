//! Verifier round-2 probe s3: a table handed to a C-ABI symbol (fn js_log_i32(rule: &Rule);).
#![no_std]
#![no_main]
unsafe extern "C" {
    fn js_log_f64(x: f64);
    fn js_log_str(s: &str);
    fn js_abort(msg: &str) -> !;
    fn js_log_i32(rule: &Rule);
}
#[derive(Clone, Copy)]
pub struct Rule { pub name: &'static str, pub selector: &'static str }
const RULES: &[Rule] = &[
    Rule { name: "sx-rule-alpha", selector: "#ad-banner-alpha .sponsored" },
    Rule { name: "sx-rule-beta", selector: "div[class^=\"promo-beta\"]" },
    Rule { name: "sx-rule-gamma", selector: ".popup-gamma, #overlay-gamma" },
    Rule { name: "sx-rule-delta", selector: "iframe[src*=\"ads-delta\"]" },
    Rule { name: "sx-rule-epsilon", selector: "#interstitial-epsilon" },
    Rule { name: "sx-rule-zeta", selector: ".sticky-ad-zeta > a[href]" },
    Rule { name: "sx-rule-eta", selector: "#cookie-wall-eta .newsletter" },
    Rule { name: "sx-rule-theta", selector: "a[href*=\"/click-theta?\"]" },
];
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { unsafe { js_abort("panic") } }
#[inline(never)]
fn name_bytes() -> u32 { RULES.iter().map(|rule| rule.name.len() as u32).sum() }
#[inline(never)]
fn first_name() -> &'static str { RULES.iter().next().map_or("none", |rule| rule.name) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        js_log_f64(name_bytes() as f64);
        js_log_str(first_name());
        js_log_i32(&RULES[2]);
        js_log_f64(name_bytes() as f64);
        js_log_str(first_name());
    }
}

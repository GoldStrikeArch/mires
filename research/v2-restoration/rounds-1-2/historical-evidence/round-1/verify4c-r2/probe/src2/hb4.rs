//! Verifier probe: a table handed to a plain `extern "C"` function (lowered to `__rt.<symbol>`).
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
pub struct Rule { pub name: &'static str, pub selector: &'static str }

const RULES: &[Rule] = &[
    Rule { name: "hb4-rule-alpha", selector: "#ad-banner-alpha .sponsored" },
    Rule { name: "hb4-rule-beta", selector: "div[class^=\"promo-beta\"]" },
    Rule { name: "hb4-rule-gamma", selector: ".popup-gamma, #overlay-gamma" },
    Rule { name: "hb4-rule-delta", selector: "iframe[src*=\"ads-delta\"]" },
    Rule { name: "hb4-rule-epsilon", selector: "#interstitial-epsilon" },
    Rule { name: "hb4-rule-zeta", selector: ".sticky-ad-zeta > a[href]" },
    Rule { name: "hb4-rule-eta", selector: "#cookie-wall-eta .newsletter" },
    Rule { name: "hb4-rule-theta", selector: "a[href*=\"/click-theta?\"]" },
];

unsafe extern "C" {
    fn probe_touch_rules(rules: &[Rule]);
}

#[inline(never)]
fn name_bytes() -> u32 { RULES.iter().map(|rule| rule.name.len() as u32).sum() }
#[inline(never)]
fn first_name() -> &'static str { RULES.iter().next().map_or("none", |rule| rule.name) }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(name_bytes());
    print_str(first_name());
    unsafe { probe_touch_rules(RULES) };
    print_u32(name_bytes());
    print_str(first_name());
}

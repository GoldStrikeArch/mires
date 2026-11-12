//! Verifier round-2 probe cc1: the host boundary is in a dependency crate; the table is in this crate.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use ccdep::{Rule, Other};
const RULES: &[Rule] = &[
    Rule { name: "cc1-rule-alpha", selector: "#ad-banner-alpha .sponsored" },
    Rule { name: "cc1-rule-beta", selector: "div[class^=\"promo-beta\"]" },
    Rule { name: "cc1-rule-gamma", selector: ".popup-gamma, #overlay-gamma" },
    Rule { name: "cc1-rule-delta", selector: "iframe[src*=\"ads-delta\"]" },
    Rule { name: "cc1-rule-epsilon", selector: "#interstitial-epsilon" },
    Rule { name: "cc1-rule-zeta", selector: ".sticky-ad-zeta > a[href]" },
    Rule { name: "cc1-rule-eta", selector: "#cookie-wall-eta .newsletter" },
    Rule { name: "cc1-rule-theta", selector: "a[href*=\"/click-theta?\"]" },
];
#[inline(never)] fn name_bytes() -> u32 { RULES.iter().map(|rule| rule.name.len() as u32).sum() }
#[inline(never)] fn first_name() -> &'static str { RULES.iter().next().map_or("none", |rule| rule.name) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(name_bytes()); print_str(first_name()); print_u32(ccdep::dep_name_bytes()); print_str(ccdep::DEP_RULES[0].name);
    #[cfg(leak_main)] ccdep::send(RULES);
    #[cfg(leak_dep)] ccdep::send(ccdep::DEP_RULES);
    #[cfg(leak_other)] ccdep::send_other(&[Other { name: "x", n: 1 }]);
    print_u32(name_bytes()); print_str(first_name()); print_u32(ccdep::dep_name_bytes()); print_str(ccdep::DEP_RULES[0].name);
}

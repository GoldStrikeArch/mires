//! Verifier round-2 probe: a dependency crate that hands a slice of its `Rule` to a host-defined C-ABI symbol.
#![no_std]
#[derive(Clone, Copy)]
pub struct Rule { pub name: &'static str, pub selector: &'static str }
#[derive(Clone, Copy)]
pub struct Other { pub name: &'static str, pub n: u32 }
unsafe extern "C" {
    fn probe_touch_rules(rules: &[Rule]);
    fn probe_touch_other(other: &[Other]);
}
#[inline(never)]
pub fn send(rules: &[Rule]) { unsafe { probe_touch_rules(rules) } }
#[inline(never)]
pub fn send_other(other: &[Other]) { unsafe { probe_touch_other(other) } }
pub const DEP_RULES: &[Rule] = &[
    Rule { name: "ccdep-rule-alpha", selector: "#ad-banner-alpha .sponsored" },
    Rule { name: "ccdep-rule-beta", selector: "div[class^=\"promo-beta\"]" },
    Rule { name: "ccdep-rule-gamma", selector: ".popup-gamma, #overlay-gamma" },
    Rule { name: "ccdep-rule-delta", selector: "iframe[src*=\"ads-delta\"]" },
    Rule { name: "ccdep-rule-epsilon", selector: "#interstitial-epsilon" },
    Rule { name: "ccdep-rule-zeta", selector: ".sticky-ad-zeta > a[href]" },
    Rule { name: "ccdep-rule-eta", selector: "#cookie-wall-eta .newsletter" },
    Rule { name: "ccdep-rule-theta", selector: "a[href*=\"/click-theta?\"]" },
];
#[inline(never)]
pub fn dep_name_bytes() -> u32 { DEP_RULES.iter().map(|rule| rule.name.len() as u32).sum() }

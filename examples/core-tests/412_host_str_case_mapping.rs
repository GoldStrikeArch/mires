//! Host-delegated `alloc::str::to_lowercase` & `to_uppercase`: ASCII, Latin-1/German ß, Turkish İ,
//! Greek Final Sigma contextual rules, multi-char expansions, astral/CJK/emoji, and both native
//! String and baseline `String::into_bytes` carriers.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use prelude::{print_bool, print_str};

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. ASCII
    print_str(&"Hello, WORLD 123!".to_lowercase());
    print_str(&"Hello, world 123!".to_uppercase());
    print_bool("".to_lowercase().is_empty());
    print_bool("".to_uppercase().is_empty());

    // 2. German ß / ẞ, Turkish İ, ligatures, combining marks
    print_str(&"tschüß".to_uppercase());
    print_str(&"ẞ".to_lowercase());
    print_str(&"İ".to_lowercase());
    print_str(&"ﬁ".to_uppercase());
    print_str(&"ŉ".to_uppercase());
    print_str(&"ΐ".to_uppercase());

    // 3. Greek Final Sigma contextual rules
    print_str(&"Σ".to_lowercase());
    print_str(&"ΟΣ".to_lowercase());
    print_str(&"ΟΣ ".to_lowercase());
    print_str(&"ΟΣΑ".to_lowercase());
    print_str(&"Α\u{0301}Σ".to_lowercase());
    print_str(&"Σ\u{0301}".to_lowercase());
    print_str(&"1Σ".to_lowercase());
    print_str(&"A-Σ".to_lowercase());

    // 4. Astral Deseret + CJK + emoji
    print_str(&"𐐀𐐁".to_lowercase());
    print_str(&"𐐨𐐩".to_uppercase());
    print_str(&"农历新年🚀".to_lowercase());
    print_str(&"农历新年🚀".to_uppercase());

    // 5. Baseline String (`into_bytes`) + StringBuilder (`push_str`)
    let baseline = "Größe ОДИН ΟΣ".to_lowercase().into_bytes();
    print_bool(baseline == "größe один ος".as_bytes());
    let mut builder = "MixedCase".to_uppercase();
    builder.push_str("!");
    print_str(&builder);
}

//! The `js!{}` block: what a block of JavaScript is once its captures are numbered, and how it
//! travels.
//!
//! A block reaches the backend the same way a `#[js_extern]` declaration does: as a `link_section`
//! on the marker function the macro writes, which is how `codegen_fn_attrs` carries it across a
//! crate boundary with no new machinery. This module is the format, and it is the ONE source of
//! truth for it: the macro crate compiles it to encode, and the backend includes this same file
//! with `#[path]` to decode. Two copies of a format is how a format drifts, and a drifted block is
//! a wrong emission rather than an error.
//!
//! # The encoding
//!
//! ```text
//! rcgjs.js.<version>.<slots>.<text>
//! ```
//!
//! for example `rcgjs.js.1.2.fetch(_$js0,{body:_$js1})`, which is a block with two captures.
//!
//! * `<version>` is [`VERSION`], and it is the first thing decoded, so a block written by a
//!   different version is named as such rather than mis-read.
//! * `<slots>` is how many captures the block takes, which is also how many arguments the marker
//!   call carries.
//! * `<text>` is the JavaScript, [escaped](escape), with every free identifier already replaced by
//!   its slot name. It runs to the end of the section, so it needs no length: the three fields
//!   before it hold no `.`.
//!
//! # Why the slots are already substituted
//!
//! The free variable analysis is a JavaScript lexer and it lives in the macro, which is where the
//! spans are and so where an error can be reported. Carrying the capture NAMES instead would mean
//! a second lexer in the backend to find them again, and two lexers that disagree is a miscompile.
//! What the backend receives is text with holes at fixed names, and the only thing it does with
//! them is bind them: see [`slot_name`].
//!
//! # Why the text is escaped rather than length prefixed
//!
//! A `link_section` is a string attribute and the block may hold newlines, so the section is kept
//! to one line: it is greppable in a fixture, readable in an error message, and diffable. The
//! escape is the four characters that would otherwise end or corrupt a line, and it is reversed
//! exactly, so nothing about the JavaScript is normalized on the way through.

// Each of the two crates compiling this file uses only its own half: the macro
// encodes and never decodes, the backend the other way around. Dead code here
// is the design, not an accident.
#![allow(dead_code)]

/// The `link_section` prefix a `js!{}` block carries.
///
/// Distinct from `#[js_extern]`'s `rcgjs.ext.` and from `view-abi`'s `rcgjs.tc.`, so the three
/// markers are told apart by their prefix alone.
pub const PREFIX: &str = "rcgjs.js.";

/// The block format version. A change to the grammar above bumps this.
pub const VERSION: u32 = 1;

/// What a capture slot is named in the emitted JavaScript.
///
/// `$` is what keeps a slot out of the way of anything the block itself can spell: the macro
/// refuses a block that mentions a name of this shape, and a Rust binding cannot be called one.
pub const SLOT_PREFIX: &str = "_$js";

/// What capture `index` is called in the emitted JavaScript.
#[must_use]
pub fn slot_name(index: usize) -> String {
    format!("{SLOT_PREFIX}{index}")
}

/// Whether `name` is a slot name, which is what the macro refuses inside a block.
#[must_use]
pub fn is_slot_name(name: &str) -> bool {
    match name.strip_prefix(SLOT_PREFIX) {
        Some(digits) => !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// One `js!{}` block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    /// How many captures the block takes. Slot `i` is [`slot_name`] of `i`, and the marker call's
    /// argument `i` is what fills it.
    pub slots: usize,
    /// The JavaScript, with every free identifier replaced by its slot name.
    pub text: String,
}

impl Block {
    /// The `link_section` this block travels as, [`PREFIX`] included.
    #[must_use]
    pub fn encode(&self) -> String {
        format!("{PREFIX}{VERSION}.{}.{}", self.slots, escape(&self.text))
    }

    /// The block `body` spells, with [`PREFIX`] already stripped.
    ///
    /// # Errors
    ///
    /// Returns why the text is not a block this version understands. Every failure names the text,
    /// so a mismatched macro and backend report each other rather than emitting something.
    pub fn decode(body: &str) -> Result<Block, String> {
        let bad = |why: &str| format!("`{PREFIX}{body}` is not a `js!{{}}` block: {why}");

        let mut parts = body.splitn(3, '.');
        let (Some(version), Some(slots), Some(text)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(bad("it has fewer than three fields"));
        };

        if version != VERSION.to_string() {
            return Err(bad(&format!(
                "it is version `{version}` and this backend reads version {VERSION}"
            )));
        }

        let slots: usize =
            slots.parse().map_err(|_| bad(&format!("`{slots}` is not a slot count")))?;
        let text = unescape(text).map_err(|why| bad(&why))?;

        Ok(Block { slots, text })
    }
}

/// The one line form of `text`.
///
/// Four characters are spelled with a backslash: the backslash itself, a newline, a carriage
/// return, and NUL, which rustc refuses in a `link_section` outright.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out
}

/// What [`escape`] wrote, back as it was.
///
/// # Errors
///
/// Returns why the text is not an escaped one: a backslash naming no escape, or one at the end.
pub fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(other) => return Err(format!("`\\{other}` is not an escape")),
            None => return Err("it ends in a backslash".to_owned()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(block: &Block) {
        let encoded = block.encode();
        let body = encoded.strip_prefix(PREFIX).expect("the prefix is written");
        assert_eq!(Block::decode(body).as_ref(), Ok(block), "{encoded}");
    }

    #[test]
    fn the_encoding_is_the_documented_one() {
        assert_eq!(
            Block { slots: 2, text: "fetch(_$js0,{body:_$js1})".to_owned() }.encode(),
            "rcgjs.js.1.2.fetch(_$js0,{body:_$js1})",
        );
        assert_eq!(Block { slots: 0, text: "1 + 1".to_owned() }.encode(), "rcgjs.js.1.0.1 + 1");
    }

    #[test]
    fn a_block_full_of_delimiters_survives() {
        // The text runs to the end of the section, so every one of these is carried verbatim.
        for text in [
            "a.b.c",
            "\"1.2.3\"",
            "x => ({ \"content-type\": y })",
            "`a${b}c`",
            "/* . */ 1",
            "",
        ] {
            round_trip(&Block { slots: 1, text: text.to_owned() });
        }
    }

    #[test]
    fn a_newline_travels_as_one_line_and_comes_back_as_a_newline() {
        let block = Block { slots: 0, text: "a &&\n// why\nb".to_owned() };
        let encoded = block.encode();
        assert!(!encoded.contains('\n'), "{encoded}");
        round_trip(&block);
    }

    #[test]
    fn a_backslash_survives_a_newline_beside_it() {
        // The case a naive escape gets wrong: a literal backslash-n in the JavaScript is not a
        // newline, and must not come back as one.
        round_trip(&Block { slots: 0, text: "\"a\\nb\"".to_owned() });
        assert_eq!(escape("\\n"), "\\\\n");
        assert_eq!(unescape("\\\\n").as_deref(), Ok("\\n"));
    }

    #[test]
    fn every_malformed_block_is_an_error_and_not_a_guess() {
        for (body, why) in [
            ("1.2", "fewer than three"),
            ("2.1.x", "version `2`"),
            ("1.x.y", "not a slot count"),
            ("1.0.a\\q", "not an escape"),
            ("1.0.a\\", "ends in a backslash"),
        ] {
            let error = Block::decode(body).unwrap_err();
            assert!(error.contains(why), "`{body}` reported `{error}`");
            assert!(error.contains(body), "`{body}` reported `{error}`");
        }
    }

    #[test]
    fn a_slot_is_named_after_its_index_and_recognized_by_that_name() {
        assert_eq!(slot_name(0), "_$js0");
        assert_eq!(slot_name(12), "_$js12");
        assert!(is_slot_name("_$js0"));
        assert!(is_slot_name("_$js12"));
        assert!(!is_slot_name("_$js"));
        assert!(!is_slot_name("_$jsx"));
        assert!(!is_slot_name("_js0"));
        assert!(!is_slot_name("$js0"));
    }
}

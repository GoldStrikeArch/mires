//! The `#[js_extern]` descriptor: what a declared JavaScript interface is, and how it travels.
//!
//! A declaration reaches the backend as a `link_section` on the marker function the macro writes,
//! which is how `codegen_fn_attrs` carries it across a crate boundary with no new machinery. This
//! module is the format, and it is the ONE source of truth for it: the macro crate compiles it to
//! encode, and the backend includes this same file with `#[path]` to decode. Two copies of a
//! format is how a format drifts, and a drifted descriptor is a wrong emission rather than an
//! error.
//!
//! # The encoding
//!
//! ```text
//! rcgjs.ext.3.<shape><flags>.<slots>.<module-len>,<name-len>.<module><name>
//! ```
//!
//! for example `rcgjs.ext.3.n-.-.8,5.chart.jsChart`, which is
//! `new Chart(..)` on the `Chart` export of `chart.js`.
//!
//! * `<version>` is [`VERSION`], and it is the first thing decoded, so a descriptor written by a
//!   different version is named as such rather than mis-read.
//! * `<shape>` is one letter, [`Shape::letter`].
//! * `<flags>` is a run of letters, or `-` for none. `n` is a nullable return, `g` is a path rooted
//!   at the global scope, and `f` catches a synchronous host throw into `Result`.
//! * `<slots>` is the per-slot contract, `-` where no slot is annotated: a `,` separated run of
//!   `<key><letters>` entries in slot order, the key a decimal parameter index or `*` for the
//!   result, the letters a run of [`Fact`]s. See [`SlotFacts`].
//! * the two lengths are byte counts, and the payload is the module specifier followed by the
//!   JavaScript name, concatenated.
//!
//! Version 2 is the same without the `<slots>` field, and is still decoded: its slots are simply
//! unannotated, which is what every declaration that says nothing about its slots means.
//!
//! # Where a path is rooted
//!
//! Three roots exist and every declaration has exactly one, answered by [`Descriptor::root`]: the
//! call's first argument, a module's imported binding, or the global scope. A module specifier
//! names the second; the `g` flag names the third; the absence of both leaves a member shape on
//! its argument, which is the common case and so the one that needs no spelling.
//!
//! The flag exists because a member shape has an argument to fall back on and a call does not. A
//! `call` or a `new` with no module is already global, because there is no receiver for it to be
//! anything else; a `get` with no module would silently read a property of argument zero. So a
//! global `Math.max(..)` needs no flag and a global `location.href` does.
//!
//! # Why lengths rather than a delimiter
//!
//! A module specifier holds `.`, `/`, `-`, `@` and `:` (`chart.js`, `@scope/pkg`, `./local.js`,
//! `node:fs`) and a JavaScript name holds `$` and `_`. No character is safely excluded from both,
//! so a delimiter would need escaping, and an escaping bug is silent. Lengths need none, and
//! decoding is total: anything malformed is an error naming the descriptor, never a guess.
//!
//! # Why not a binary codec
//!
//! A postcard-and-base64 form would need the same codec crate on both sides, which is a version
//! skew waiting to happen, and its failure mode is a mis-decoded descriptor rather than a refusal.
//! This form is diffable in a fixture, readable in an error message, and version checked first.

// Each of the two crates compiling this file uses only its own half: the macro
// encodes and never decodes, the backend the other way around. Dead code here
// is the design, not an accident.
#![allow(dead_code)]

/// The `link_section` prefix a `#[js_extern]` declaration carries.
pub const PREFIX: &str = "rcgjs.ext.";

/// The descriptor format version. A change to the grammar above bumps this.
///
/// Version 2 added the `g` flag and [`Descriptor::global`]. Version 3 added the `<slots>` field,
/// the per-slot host contract. New shape letters, flags and [`Fact`]s are backward compatible
/// because older descriptors keep their exact encoding; a new FIELD is not, and is what a bump is
/// for.
pub const VERSION: u32 = 3;

/// The versions [`Descriptor::decode`] reads, newest first. A version outside this set is named
/// rather than read, so a mismatched macro and backend report each other.
const READS: [u32; 2] = [3, 2];

/// Which JavaScript operation a declaration names.
///
/// The set is Melange's, which is the vocabulary a declared interface needs: a thing to call, a
/// thing to construct, a method, a property, and an index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shape {
    /// `name(a, b)`: a call of the imported binding, or of a global.
    Call,
    /// `new name(a, b)`.
    New,
    /// `a.name(b, c)`: a method on the first argument.
    Method,
    /// `a.name`: a property of the first argument.
    Get,
    /// `a.name = b`.
    Set,
    /// `a[b]`: the first argument indexed by the second.
    Index,
    /// `a[b] = c`.
    IndexSet,
    /// `Reflect.has(a, b)`, equivalent to `b in a`.
    Has,
    /// `a[b](c, d)`: a computed method on the first argument.
    ComputedMethod,
}

impl Shape {
    /// The letter this shape is written as.
    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Shape::Call => 'c',
            Shape::New => 'n',
            Shape::Method => 's',
            Shape::Get => 'g',
            Shape::Set => 't',
            Shape::Index => 'i',
            Shape::IndexSet => 'j',
            Shape::Has => 'h',
            Shape::ComputedMethod => 'm',
        }
    }

    /// The shape a letter names.
    #[must_use]
    pub fn from_letter(letter: char) -> Option<Shape> {
        Some(match letter {
            'c' => Shape::Call,
            'n' => Shape::New,
            's' => Shape::Method,
            'g' => Shape::Get,
            't' => Shape::Set,
            'i' => Shape::Index,
            'j' => Shape::IndexSet,
            'h' => Shape::Has,
            'm' => Shape::ComputedMethod,
            _ => return None,
        })
    }

    /// Whether the shape produces a value. A setter does not, so `#[js(nullable)]` on one is
    /// meaningless.
    #[must_use]
    pub fn returns_a_value(self) -> bool {
        !matches!(self, Shape::Set | Shape::IndexSet)
    }

    /// How many arguments the shape spends beyond the receiver: none, the value a setter writes,
    /// the key an index reads, or the key and the value together.
    #[must_use]
    pub fn operands(self) -> usize {
        match self {
            Shape::Call | Shape::New | Shape::Method | Shape::Get => 0,
            Shape::Set | Shape::Index | Shape::Has | Shape::ComputedMethod => 1,
            Shape::IndexSet => 2,
        }
    }
}

/// What a declaration's path is rooted at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Root {
    /// The call's first argument, which is therefore not passed on.
    Argument,
    /// The binding imported from the declaration's module.
    Module,
    /// A name the host already has, imported from nothing.
    Global,
}

/// One slot of a declaration: a parameter by position, or the result.
///
/// The ordering is the order [`Descriptor::encode`] writes the slots in, which is what makes the
/// encoding of a set of slot facts unique.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Slot {
    /// The `index`-th declared Rust parameter, counted before the shape spends its receiver and
    /// key, so a slot names the same argument whatever the shape does with it.
    Parameter(u32),
    /// The value the declaration returns.
    Result,
}

impl Slot {
    /// The key this slot is written as: a decimal index, or `*` for the result.
    #[must_use]
    pub fn key(self) -> String {
        match self {
            Slot::Parameter(index) => index.to_string(),
            Slot::Result => "*".to_owned(),
        }
    }

    /// The slot a key names.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Slot> {
        match key {
            "*" => Some(Slot::Result),
            _ => key.parse().ok().map(Slot::Parameter),
        }
    }
}

/// One thing a declaration says about one slot.
///
/// The first five are **carriers the slot accepts**: the representation planner may hand the host
/// a value in that shape without materializing it first. The rest are **how the slot observes the
/// value**, which is what decides whether an edge into the host may move off the baseline at all:
/// a slot that only `reads` is a sink, one that `retains` what it `mutates` pins its edge, and one
/// that `owns` its argument takes it over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fact {
    /// Accepts the baseline representation of the slot's Rust type.
    Baseline,
    /// Accepts a JavaScript string.
    HostString,
    /// Accepts `{ buf, len, cap }` over a native array.
    NativeVec,
    /// Accepts `{ buf, off, len }` over a shared native array.
    SliceView,
    /// Accepts the value as a plain JavaScript number, boolean or string.
    Scalar,
    /// Reads the value, and nothing more.
    Reads,
    /// Observes the value's JavaScript object identity.
    Identity,
    /// Observes the value's bytes rather than its abstract contents.
    Bytes,
    /// Writes through the value.
    Mutates,
    /// Keeps a reference to the value past the call.
    Retains,
    /// Takes ownership of the value.
    Owns,
    /// May throw.
    Throws,
    /// May suspend. Reserved; nothing reads it yet.
    Suspends,
}

impl Fact {
    /// Every fact, in the order [`SlotFacts::letters`] writes them, with the letter it is encoded
    /// as and the word the `#[js(..)]` surface spells it with. Append only: an entry's letter is
    /// part of the wire format.
    const TABLE: [(Fact, char, &'static str); 13] = [
        (Fact::Baseline, 'b', "baseline"),
        (Fact::HostString, 'h', "host_string"),
        (Fact::NativeVec, 'v', "native_vec"),
        (Fact::SliceView, 's', "slice_view"),
        (Fact::Scalar, 'c', "scalar"),
        (Fact::Reads, 'r', "reads"),
        (Fact::Identity, 'd', "identity"),
        (Fact::Bytes, 'y', "bytes"),
        (Fact::Mutates, 'm', "mutates"),
        (Fact::Retains, 't', "retains"),
        (Fact::Owns, 'o', "owns"),
        (Fact::Throws, 'x', "throws"),
        (Fact::Suspends, 'p', "suspends"),
    ];

    /// How many leading [`Fact::TABLE`] entries are carriers a slot accepts.
    const CARRIERS: usize = 5;

    /// Every fact, in encoding order.
    pub fn all() -> impl Iterator<Item = Fact> {
        Self::TABLE.iter().map(|(fact, _, _)| *fact)
    }

    /// The carriers a slot can say it accepts, in encoding order.
    pub fn carriers() -> impl Iterator<Item = Fact> {
        Self::TABLE[..Self::CARRIERS].iter().map(|(f, _, _)| *f)
    }

    /// How a slot can say it observes its value, in encoding order.
    pub fn observations() -> impl Iterator<Item = Fact> {
        Self::TABLE[Self::CARRIERS..].iter().map(|(f, _, _)| *f)
    }

    /// Whether this fact names a carrier the slot accepts rather than an observation.
    #[must_use]
    pub fn is_carrier(self) -> bool {
        Self::carriers().any(|carrier| carrier == self)
    }

    /// The letter this fact is written as.
    #[must_use]
    pub fn letter(self) -> char {
        Self::TABLE
            .iter()
            .find(|(fact, _, _)| *fact == self)
            .map_or('?', |(_, letter, _)| *letter)
    }

    /// The word `#[js(..)]` spells this fact with.
    #[must_use]
    pub fn word(self) -> &'static str {
        Self::TABLE
            .iter()
            .find(|(fact, _, _)| *fact == self)
            .map_or("?", |(_, _, word)| *word)
    }

    /// The fact a letter names.
    #[must_use]
    pub fn from_letter(letter: char) -> Option<Fact> {
        Self::TABLE
            .iter()
            .find(|(_, written, _)| *written == letter)
            .map(|(fact, _, _)| *fact)
    }

    /// The fact a word names.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Fact> {
        Self::TABLE
            .iter()
            .find(|(_, _, spelled)| *spelled == word)
            .map(|(fact, _, _)| *fact)
    }

    fn bit(self) -> u16 {
        let index = Self::TABLE
            .iter()
            .position(|(fact, _, _)| *fact == self)
            .unwrap_or(0);
        1 << index
    }
}

/// What one declaration says about one slot: a set of [`Fact`]s.
///
/// Empty is the default and the thing a version 2 descriptor decodes to everywhere: the
/// declaration says nothing, and the consumer classifies the slot from its Rust type exactly as it
/// did before the field existed.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SlotFacts(u16);

impl SlotFacts {
    /// The facts `letters` spells.
    ///
    /// # Errors
    ///
    /// Returns the letter that names no fact.
    pub fn from_letters(letters: &str) -> Result<SlotFacts, char> {
        let mut facts = SlotFacts::default();
        for letter in letters.chars() {
            facts = facts.with(Fact::from_letter(letter).ok_or(letter)?);
        }
        Ok(facts)
    }

    /// The letters these facts are written as, in [`Fact`]'s order.
    #[must_use]
    pub fn letters(self) -> String {
        Fact::all()
            .filter(|fact| self.has(*fact))
            .map(Fact::letter)
            .collect()
    }

    /// Whether the slot carries `fact`.
    #[must_use]
    pub fn has(self, fact: Fact) -> bool {
        self.0 & fact.bit() != 0
    }

    /// These facts and `fact`.
    #[must_use]
    pub fn with(self, fact: Fact) -> SlotFacts {
        SlotFacts(self.0 | fact.bit())
    }

    /// Whether the declaration said anything at all about the slot.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether the slot names any carrier it accepts. A slot that names none accepts the baseline
    /// alone, which is what saying nothing about carriers means.
    #[must_use]
    pub fn accepts_a_carrier(self) -> bool {
        Fact::carriers().any(|carrier| self.has(carrier))
    }
}

/// One declared JavaScript interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Descriptor {
    pub shape: Shape,
    /// Whether a `null` or `undefined` result becomes `Option::None`.
    pub nullable: bool,
    /// Whether a synchronous host throw becomes `Result::Err` with the thrown value unchanged.
    pub fallible: bool,
    /// Whether the path is rooted at the global scope rather than at an argument. Meaningless
    /// beside a module, which is a root of its own, and unnecessary on a call or a construction,
    /// which have no argument to be rooted at instead.
    pub global: bool,
    /// The module the binding is imported from, or empty for a global.
    pub module: String,
    /// The dotted path the operation walks, from wherever [`Descriptor::root`] roots it. Empty for
    /// an index straight off its receiver.
    pub name: String,
    /// What the declaration says about its slots, by slot, in [`Slot`] order and without repeats.
    /// A slot absent here said nothing, which is every slot of a version 2 descriptor.
    pub slots: Vec<(Slot, SlotFacts)>,
}

impl Descriptor {
    /// What the path walks from.
    ///
    /// A module specifier names a binding to import. The `g` flag names the global scope. With
    /// neither, a member operation acts on the call's first argument, and a call or a construction
    /// is global because it has no receiver to be anything else.
    ///
    /// This is what lets `instance.data.datasets`, `Chart.version` and `location.href` be the same
    /// shape rooted three ways, which is what the three of them are.
    ///
    /// An index is the one shape a module specifier does not move, and deliberately: a collection
    /// is reached THROUGH something, so `chart.data.datasets[k]` is the ordinary declaration and
    /// the block's module says where the library came from rather than where the walk starts. An
    /// index off a global says so with the flag.
    #[must_use]
    pub fn root(&self) -> Root {
        match self.shape {
            Shape::Index | Shape::IndexSet | Shape::Has | Shape::ComputedMethod => {
                match self.global {
                    true => Root::Global,
                    false => Root::Argument,
                }
            }
            Shape::Call | Shape::New => match self.module.is_empty() {
                true => Root::Global,
                false => Root::Module,
            },
            Shape::Method | Shape::Get | Shape::Set => {
                match (self.module.is_empty(), self.global) {
                    (false, _) => Root::Module,
                    (true, true) => Root::Global,
                    (true, false) => Root::Argument,
                }
            }
        }
    }

    /// Whether the operation is rooted at the call's FIRST argument rather than at a module
    /// binding or a global.
    #[must_use]
    pub fn acts_on_argument(&self) -> bool {
        self.root() == Root::Argument
    }

    /// How many leading arguments the operation itself consumes: the receiver where there is one,
    /// then the key and the value the shape needs. Everything after them passes through.
    #[must_use]
    pub fn required_arguments(&self) -> usize {
        usize::from(self.acts_on_argument()) + self.shape.operands()
    }

    /// The path's segments, which is what the emission walks.
    #[must_use]
    pub fn segments(&self) -> Vec<&str> {
        match self.name.is_empty() {
            true => Vec::new(),
            false => self.name.split('.').collect(),
        }
    }

    /// The `link_section` this descriptor travels as, [`PREFIX`] included.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut flags = String::new();
        if self.nullable {
            flags.push('n');
        }
        if self.global {
            flags.push('g');
        }
        if self.fallible {
            flags.push('f');
        }
        if flags.is_empty() {
            flags.push('-');
        }
        let mut slots: Vec<(Slot, SlotFacts)> = self
            .slots
            .iter()
            .copied()
            .filter(|(_, facts)| !facts.is_empty())
            .collect();
        slots.sort_by_key(|(slot, _)| *slot);
        let slots = match slots.is_empty() {
            true => "-".to_owned(),
            false => slots
                .iter()
                .map(|(slot, facts)| format!("{}{}", slot.key(), facts.letters()))
                .collect::<Vec<_>>()
                .join(","),
        };
        format!(
            "{PREFIX}{VERSION}.{}{flags}.{slots}.{},{}.{}{}",
            self.shape.letter(),
            self.module.len(),
            self.name.len(),
            self.module,
            self.name,
        )
    }

    /// What the declaration says about `slot`, or nothing where it said nothing.
    #[must_use]
    pub fn slot(&self, slot: Slot) -> Option<SlotFacts> {
        self.slots
            .iter()
            .find(|(found, _)| *found == slot)
            .map(|(_, facts)| *facts)
    }

    /// The descriptor `section` spells, with [`PREFIX`] already stripped.
    ///
    /// # Errors
    ///
    /// Returns why the text is not a descriptor this version understands. Every failure names the
    /// text, so a mismatched macro and backend report each other rather than emitting something.
    pub fn decode(body: &str) -> Result<Descriptor, String> {
        let bad = |why: &str| format!("`{PREFIX}{body}` is not a `#[js_extern]` descriptor: {why}");

        // The version is read on its own, because it is what says how many fields follow: a
        // version 3 descriptor has the `<slots>` field between the flags and the lengths and a
        // version 2 descriptor does not.
        let Some((version, rest)) = body.split_once('.') else {
            return Err(bad("it has fewer than four fields"));
        };
        let Some(version) = READS.iter().find(|read| read.to_string() == version) else {
            let reads = READS
                .iter()
                .rev()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" or ");
            return Err(bad(&format!(
                "it is version `{version}` and this backend reads version {reads}"
            )));
        };
        let has_slots = *version >= 3;

        let mut parts = rest.splitn(3 + usize::from(has_slots), '.');
        let (Some(kind), slots, Some(lengths), Some(payload)) = (
            parts.next(),
            match has_slots {
                true => parts.next(),
                false => Some("-"),
            },
            parts.next(),
            parts.next(),
        ) else {
            return Err(bad("it has fewer than four fields"));
        };
        let Some(slots) = slots else {
            return Err(bad("it has fewer than four fields"));
        };

        let mut letters = kind.chars();
        let shape = letters
            .next()
            .and_then(Shape::from_letter)
            .ok_or_else(|| bad(&format!("`{kind}` does not begin with a shape letter")))?;
        let mut nullable = false;
        let mut global = false;
        let mut fallible = false;
        for flag in letters {
            match flag {
                'n' => nullable = true,
                'g' => global = true,
                'f' => fallible = true,
                '-' => {}
                other => return Err(bad(&format!("`{other}` is not a flag"))),
            }
        }

        let mut decoded = Vec::new();
        if slots != "-" {
            for entry in slots.split(',') {
                let split = entry
                    .find(|c: char| Fact::from_letter(c).is_some())
                    .ok_or_else(|| bad(&format!("`{entry}` is not a slot entry")))?;
                let (key, letters) = entry.split_at(split);
                let slot = Slot::from_key(key)
                    .ok_or_else(|| bad(&format!("`{key}` is not a slot key")))?;
                let facts = SlotFacts::from_letters(letters)
                    .map_err(|letter| bad(&format!("`{letter}` is not a slot fact")))?;
                if decoded.iter().any(|(found, _)| *found == slot) {
                    return Err(bad(&format!("slot `{key}` is written twice")));
                }
                decoded.push((slot, facts));
            }
        }

        let (module_len, name_len) = lengths
            .split_once(',')
            .ok_or_else(|| bad(&format!("`{lengths}` is not a pair of lengths")))?;
        let module_len: usize = module_len
            .parse()
            .map_err(|_| bad(&format!("`{module_len}` is not a length")))?;
        let name_len: usize = name_len
            .parse()
            .map_err(|_| bad(&format!("`{name_len}` is not a length")))?;

        if payload.len() != module_len + name_len {
            return Err(bad(&format!(
                "the lengths add up to {} and the payload is {} bytes",
                module_len + name_len,
                payload.len()
            )));
        }
        // The lengths are byte counts, so a split that lands inside a character is a malformed
        // descriptor rather than a panic.
        if !payload.is_char_boundary(module_len) {
            return Err(bad(
                "the module length does not land on a character boundary",
            ));
        }
        let (module, name) = payload.split_at(module_len);

        Ok(Descriptor {
            shape,
            nullable,
            fallible,
            global,
            module: module.to_owned(),
            name: name.to_owned(),
            slots: decoded,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A descriptor with everything but the fields a test is about.
    fn descriptor(shape: Shape, module: &str, name: &str) -> Descriptor {
        Descriptor {
            shape,
            nullable: false,
            fallible: false,
            global: false,
            module: module.to_owned(),
            name: name.to_owned(),
            slots: Vec::new(),
        }
    }

    const EVERY_SHAPE: [Shape; 9] = [
        Shape::Call,
        Shape::New,
        Shape::Method,
        Shape::Get,
        Shape::Set,
        Shape::Index,
        Shape::IndexSet,
        Shape::Has,
        Shape::ComputedMethod,
    ];

    fn round_trip(descriptor: &Descriptor) {
        let encoded = descriptor.encode();
        let body = encoded.strip_prefix(PREFIX).expect("the prefix is written");
        assert_eq!(
            Descriptor::decode(body).as_ref(),
            Ok(descriptor),
            "{encoded}"
        );
    }

    #[test]
    fn every_shape_and_flag_combination_round_trips() {
        for shape in EVERY_SHAPE {
            for nullable in [false, true] {
                for fallible in [false, true] {
                    for global in [false, true] {
                        round_trip(&Descriptor {
                            shape,
                            nullable,
                            fallible,
                            global,
                            module: "chart.js".to_owned(),
                            name: "Chart".to_owned(),
                            slots: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    #[test]
    fn the_encoding_is_the_documented_one() {
        assert_eq!(
            descriptor(Shape::New, "chart.js", "Chart").encode(),
            "rcgjs.ext.3.n-.-.8,5.chart.jsChart",
        );
        assert_eq!(
            Descriptor {
                nullable: true,
                ..descriptor(Shape::Get, "", "width")
            }
            .encode(),
            "rcgjs.ext.3.gn.-.0,5.width",
        );
        // Both flags together, in the order `encode` writes them.
        assert_eq!(
            Descriptor {
                nullable: true,
                global: true,
                ..descriptor(Shape::Get, "", "location.href")
            }
            .encode(),
            "rcgjs.ext.3.gng.-.0,13.location.href",
        );
        assert_eq!(
            Descriptor {
                fallible: true,
                ..descriptor(Shape::Call, "", "JSON.parse")
            }
            .encode(),
            "rcgjs.ext.3.cf.-.0,10.JSON.parse",
        );
    }

    #[test]
    fn a_specifier_full_of_delimiters_survives() {
        // The reason the lengths are there: every one of these holds a character a delimiter
        // scheme would have had to escape.
        for module in [
            "chart.js",
            "@scope/pkg",
            "./local.js",
            "node:fs",
            "a.b,c-d/e",
        ] {
            round_trip(&descriptor(Shape::Call, module, "f"));
        }
    }

    #[test]
    fn an_empty_module_is_a_global_and_an_empty_name_is_an_index() {
        round_trip(&descriptor(Shape::New, "", "EventSource"));
        round_trip(&descriptor(Shape::Index, "data.js", ""));
    }

    #[test]
    fn a_wrong_version_is_named_rather_than_read() {
        let error = Descriptor::decode("1.c-.-.0,1.f").unwrap_err();
        assert!(error.contains("version `1`"), "{error}");
        // The refusal names every version this backend reads, so a mismatched pair reports which
        // of the two it is.
        assert!(error.contains("version 2 or 3"), "{error}");
    }

    #[test]
    fn every_malformed_descriptor_is_an_error_and_not_a_guess() {
        for (body, why) in [
            ("3.c-.-.0,1", "fewer than four"),
            ("3.?-.-.0,1.f", "shape letter"),
            ("3.cz.-.0,1.f", "not a flag"),
            ("3.c-.-.01.f", "pair of lengths"),
            ("3.c-.-.x,1.f", "not a length"),
            ("3.c-.-.0,9.f", "add up to"),
            ("3.c-.0.0,1.f", "not a slot entry"),
            ("3.c-.0rq.0,1.f", "not a slot fact"),
            ("3.c-.zr.0,1.f", "not a slot key"),
            ("3.c-.0r,0h.0,1.f", "written twice"),
            ("2.c-.0,1", "fewer than four"),
        ] {
            let error = Descriptor::decode(body).unwrap_err();
            assert!(error.contains(why), "`{body}` reported `{error}`");
            assert!(error.contains(body), "`{body}` reported `{error}`");
        }
    }

    #[test]
    fn a_length_that_splits_a_character_is_refused() {
        // "\u{e9}" is two bytes; a module length of 1 lands inside it.
        let body = "3.c-.-.1,2.\u{e9}f";
        let error = Descriptor::decode(body).unwrap_err();
        assert!(error.contains("character boundary"), "{error}");
    }

    #[test]
    fn a_member_shape_is_rooted_at_its_argument_only_without_a_module_or_the_flag() {
        // `instance.data` against `Chart.version` against `location.href`: one shape, three
        // rootings.
        assert_eq!(descriptor(Shape::Get, "", "x").root(), Root::Argument);
        assert_eq!(descriptor(Shape::Get, "chart.js", "x").root(), Root::Module);
        assert_eq!(
            Descriptor {
                global: true,
                ..descriptor(Shape::Get, "", "x")
            }
            .root(),
            Root::Global,
        );
        assert_eq!(descriptor(Shape::Method, "", "x").root(), Root::Argument);
        assert_eq!(
            descriptor(Shape::Method, "chart.js", "x").root(),
            Root::Module
        );
        // A call and a construction are global without a module and never act on an argument.
        assert_eq!(descriptor(Shape::Call, "", "x").root(), Root::Global);
        assert_eq!(descriptor(Shape::New, "", "x").root(), Root::Global);
        assert_eq!(descriptor(Shape::New, "chart.js", "x").root(), Root::Module);
    }

    #[test]
    fn an_index_is_rooted_at_its_argument_whatever_module_it_came_from() {
        // The chart fixture's own shape: a block module plus an index that walks from the
        // receiver. Moving it to the module binding would change what every index vector emits.
        assert_eq!(
            descriptor(Shape::Index, "chart.js", "data.datasets").root(),
            Root::Argument
        );
        assert_eq!(
            descriptor(Shape::IndexSet, "chart.js", "").root(),
            Root::Argument
        );
        assert_eq!(
            descriptor(Shape::Has, "chart.js", "").root(),
            Root::Argument
        );
        assert_eq!(
            descriptor(Shape::ComputedMethod, "chart.js", "").root(),
            Root::Argument
        );
        // The flag is how an index off a global says so.
        assert_eq!(
            Descriptor {
                global: true,
                ..descriptor(Shape::Index, "", "sessionStorage")
            }
            .root(),
            Root::Global,
        );
    }

    #[test]
    fn the_required_arguments_are_the_receiver_plus_the_shapes_operands() {
        let count = |shape, module: &str| descriptor(shape, module, "x").required_arguments();
        assert_eq!(count(Shape::Call, "m"), 0);
        assert_eq!(count(Shape::New, ""), 0);
        assert_eq!(count(Shape::Get, ""), 1);
        assert_eq!(count(Shape::Get, "m"), 0);
        assert_eq!(count(Shape::Set, ""), 2);
        assert_eq!(count(Shape::Set, "m"), 1);
        assert_eq!(count(Shape::Index, ""), 2);
        assert_eq!(count(Shape::IndexSet, ""), 3);
        assert_eq!(count(Shape::Has, ""), 2);
        assert_eq!(count(Shape::ComputedMethod, ""), 2);
        // The flag takes the receiver away exactly as a module does.
        let global = |shape| {
            Descriptor {
                global: true,
                ..descriptor(shape, "", "x")
            }
            .required_arguments()
        };
        assert_eq!(global(Shape::Get), 0);
        assert_eq!(global(Shape::Set), 1);
        assert_eq!(global(Shape::Index), 1);
        assert_eq!(global(Shape::Has), 1);
        assert_eq!(global(Shape::ComputedMethod), 1);
    }

    #[test]
    fn a_path_splits_into_the_steps_the_emission_walks() {
        let path = |name: &str| {
            descriptor(Shape::Get, "", name)
                .segments()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(path("data.datasets"), ["data", "datasets"]);
        assert_eq!(path("data"), ["data"]);
        assert!(path("").is_empty());
    }

    #[test]
    fn a_version_2_descriptor_still_decodes_and_says_nothing_about_its_slots() {
        // The whole point of reading two versions: a declaration compiled before the field existed
        // keeps its exact meaning, which is that every slot is classified from its Rust type.
        for (v2, v3) in [
            ("2.n-.8,5.chart.jsChart", "3.n-.-.8,5.chart.jsChart"),
            ("2.gn.0,5.width", "3.gn.-.0,5.width"),
            ("2.cf.0,10.JSON.parse", "3.cf.-.0,10.JSON.parse"),
            ("2.i-.0,0.", "3.i-.-.0,0."),
        ] {
            let decoded = Descriptor::decode(v2).expect("version 2 is read");
            assert!(decoded.slots.is_empty(), "{v2}");
            assert_eq!(Descriptor::decode(v3).as_ref(), Ok(&decoded));
            // Re-encoding writes the version this macro speaks, and nothing else moves.
            assert_eq!(decoded.encode(), format!("{PREFIX}{v3}"));
        }
    }

    #[test]
    fn a_version_3_descriptor_carries_its_slot_facts() {
        let decoded = Descriptor::decode("3.s-.0h,1bcrm,*to.0,5.write").expect("version 3 is read");
        assert_eq!(
            decoded.slot(Slot::Parameter(0)),
            Some(SlotFacts::default().with(Fact::HostString))
        );
        assert_eq!(
            decoded.slot(Slot::Parameter(1)),
            Some(
                SlotFacts::default()
                    .with(Fact::Baseline)
                    .with(Fact::Scalar)
                    .with(Fact::Reads)
                    .with(Fact::Mutates)
            )
        );
        assert_eq!(
            decoded.slot(Slot::Result),
            Some(SlotFacts::default().with(Fact::Retains).with(Fact::Owns))
        );
        assert_eq!(decoded.slot(Slot::Parameter(2)), None);
        round_trip(&decoded);
    }

    #[test]
    fn slot_facts_round_trip_one_at_a_time_and_all_at_once() {
        let every = Fact::all().fold(SlotFacts::default(), SlotFacts::with);
        for facts in Fact::all()
            .map(|fact| SlotFacts::default().with(fact))
            .chain([every])
        {
            assert_eq!(SlotFacts::from_letters(&facts.letters()), Ok(facts));
            round_trip(&Descriptor {
                slots: vec![(Slot::Parameter(7), facts), (Slot::Result, facts)],
                ..descriptor(Shape::Method, "chart.js", "update")
            });
        }
        // The letters and the words are each distinct, which is what makes both spellings total.
        let mut letters: Vec<char> = Fact::all().map(Fact::letter).collect();
        letters.sort_unstable();
        letters.dedup();
        assert_eq!(letters.len(), Fact::all().count());
        for fact in Fact::all() {
            assert_eq!(Fact::from_letter(fact.letter()), Some(fact));
            assert_eq!(Fact::from_word(fact.word()), Some(fact));
        }
    }

    #[test]
    fn slots_are_written_in_slot_order_whatever_order_they_arrive_in() {
        // Determinism: the same facts encode to the same section, so a declaration's descriptor is
        // its own identity and not the order the macro happened to collect them.
        let reads = SlotFacts::default().with(Fact::Reads);
        let owns = SlotFacts::default().with(Fact::Owns);
        let forwards = Descriptor {
            slots: vec![(Slot::Parameter(0), reads), (Slot::Parameter(2), owns)],
            ..descriptor(Shape::Call, "", "f")
        };
        let backwards = Descriptor {
            slots: vec![(Slot::Parameter(2), owns), (Slot::Parameter(0), reads)],
            ..descriptor(Shape::Call, "", "f")
        };
        assert_eq!(forwards.encode(), backwards.encode());
        assert_eq!(forwards.encode(), "rcgjs.ext.3.c-.0r,2o.0,1.f");
        // An empty fact set is not a slot the declaration spoke about, so it is not written.
        assert_eq!(
            Descriptor {
                slots: vec![(Slot::Parameter(0), SlotFacts::default())],
                ..descriptor(Shape::Call, "", "f")
            }
            .encode(),
            "rcgjs.ext.3.c-.-.0,1.f",
        );
    }

    #[test]
    fn the_shape_letters_are_distinct_and_total() {
        let mut letters: Vec<char> = EVERY_SHAPE.iter().map(|shape| shape.letter()).collect();
        letters.sort_unstable();
        letters.dedup();
        assert_eq!(letters.len(), EVERY_SHAPE.len());
        for shape in EVERY_SHAPE {
            assert_eq!(Shape::from_letter(shape.letter()), Some(shape));
        }
    }
}

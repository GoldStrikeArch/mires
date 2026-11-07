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
//! rcgjs.ext.<version>.<shape><flags>.<module-len>,<name-len>.<module><name>
//! ```
//!
//! for example `rcgjs.ext.2.n-.8,5.chart.jsChart`, which is
//! `new Chart(..)` on the `Chart` export of `chart.js`.
//!
//! * `<version>` is [`VERSION`], and it is the first thing decoded, so a descriptor written by a
//!   different version is named as such rather than mis-read.
//! * `<shape>` is one letter, [`Shape::letter`].
//! * `<flags>` is a run of letters, or `-` for none. `n` is a nullable return and `g` is a path
//!   rooted at the global scope.
//! * the two lengths are byte counts, and the payload is the module specifier followed by the
//!   JavaScript name, concatenated.
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
/// Version 2 added the `g` flag and [`Descriptor::global`].
pub const VERSION: u32 = 2;

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
            Shape::Set | Shape::Index => 1,
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

/// One declared JavaScript interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Descriptor {
    pub shape: Shape,
    /// Whether a `null` or `undefined` result becomes `Option::None`.
    pub nullable: bool,
    /// Whether the path is rooted at the global scope rather than at an argument. Meaningless
    /// beside a module, which is a root of its own, and unnecessary on a call or a construction,
    /// which have no argument to be rooted at instead.
    pub global: bool,
    /// The module the binding is imported from, or empty for a global.
    pub module: String,
    /// The dotted path the operation walks, from wherever [`Descriptor::root`] roots it. Empty for
    /// an index straight off its receiver.
    pub name: String,
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
            Shape::Index | Shape::IndexSet => match self.global {
                true => Root::Global,
                false => Root::Argument,
            },
            Shape::Call | Shape::New => match self.module.is_empty() {
                true => Root::Global,
                false => Root::Module,
            },
            Shape::Method | Shape::Get | Shape::Set => match (self.module.is_empty(), self.global) {
                (false, _) => Root::Module,
                (true, true) => Root::Global,
                (true, false) => Root::Argument,
            },
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
        if flags.is_empty() {
            flags.push('-');
        }
        format!(
            "{PREFIX}{VERSION}.{}{flags}.{},{}.{}{}",
            self.shape.letter(),
            self.module.len(),
            self.name.len(),
            self.module,
            self.name,
        )
    }

    /// The descriptor `section` spells, with [`PREFIX`] already stripped.
    ///
    /// # Errors
    ///
    /// Returns why the text is not a descriptor this version understands. Every failure names the
    /// text, so a mismatched macro and backend report each other rather than emitting something.
    pub fn decode(body: &str) -> Result<Descriptor, String> {
        let bad = |why: &str| format!("`{PREFIX}{body}` is not a `#[js_extern]` descriptor: {why}");

        let mut parts = body.splitn(4, '.');
        let (Some(version), Some(kind), Some(lengths), Some(payload)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(bad("it has fewer than four fields"));
        };

        if version != VERSION.to_string() {
            return Err(bad(&format!(
                "it is version `{version}` and this backend reads version {VERSION}"
            )));
        }

        let mut letters = kind.chars();
        let shape = letters
            .next()
            .and_then(Shape::from_letter)
            .ok_or_else(|| bad(&format!("`{kind}` does not begin with a shape letter")))?;
        let mut nullable = false;
        let mut global = false;
        for flag in letters {
            match flag {
                'n' => nullable = true,
                'g' => global = true,
                '-' => {}
                other => return Err(bad(&format!("`{other}` is not a flag"))),
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
            return Err(bad("the module length does not land on a character boundary"));
        }
        let (module, name) = payload.split_at(module_len);

        Ok(Descriptor {
            shape,
            nullable,
            global,
            module: module.to_owned(),
            name: name.to_owned(),
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
            global: false,
            module: module.to_owned(),
            name: name.to_owned(),
        }
    }

    const EVERY_SHAPE: [Shape; 7] = [
        Shape::Call,
        Shape::New,
        Shape::Method,
        Shape::Get,
        Shape::Set,
        Shape::Index,
        Shape::IndexSet,
    ];

    fn round_trip(descriptor: &Descriptor) {
        let encoded = descriptor.encode();
        let body = encoded.strip_prefix(PREFIX).expect("the prefix is written");
        assert_eq!(Descriptor::decode(body).as_ref(), Ok(descriptor), "{encoded}");
    }

    #[test]
    fn every_shape_and_flag_combination_round_trips() {
        for shape in EVERY_SHAPE {
            for nullable in [false, true] {
                for global in [false, true] {
                    round_trip(&Descriptor {
                        shape,
                        nullable,
                        global,
                        module: "chart.js".to_owned(),
                        name: "Chart".to_owned(),
                    });
                }
            }
        }
    }

    #[test]
    fn the_encoding_is_the_documented_one() {
        assert_eq!(
            descriptor(Shape::New, "chart.js", "Chart").encode(),
            "rcgjs.ext.2.n-.8,5.chart.jsChart",
        );
        assert_eq!(
            Descriptor { nullable: true, ..descriptor(Shape::Get, "", "width") }.encode(),
            "rcgjs.ext.2.gn.0,5.width",
        );
        // Both flags together, in the order `encode` writes them.
        assert_eq!(
            Descriptor {
                nullable: true,
                global: true,
                ..descriptor(Shape::Get, "", "location.href")
            }
            .encode(),
            "rcgjs.ext.2.gng.0,13.location.href",
        );
    }

    #[test]
    fn a_specifier_full_of_delimiters_survives() {
        // The reason the lengths are there: every one of these holds a character a delimiter
        // scheme would have had to escape.
        for module in ["chart.js", "@scope/pkg", "./local.js", "node:fs", "a.b,c-d/e"] {
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
        let error = Descriptor::decode("1.c-.0,1.f").unwrap_err();
        assert!(error.contains("version `1`"), "{error}");
        assert!(error.contains("version 2"), "{error}");
    }

    #[test]
    fn every_malformed_descriptor_is_an_error_and_not_a_guess() {
        for (body, why) in [
            ("2.c-.0,1", "fewer than four"),
            ("2.?-.0,1.f", "shape letter"),
            ("2.cz.0,1.f", "not a flag"),
            ("2.c-.01.f", "pair of lengths"),
            ("2.c-.x,1.f", "not a length"),
            ("2.c-.0,9.f", "add up to"),
        ] {
            let error = Descriptor::decode(body).unwrap_err();
            assert!(error.contains(why), "`{body}` reported `{error}`");
            assert!(error.contains(body), "`{body}` reported `{error}`");
        }
    }

    #[test]
    fn a_length_that_splits_a_character_is_refused() {
        // "\u{e9}" is two bytes; a module length of 1 lands inside it.
        let body = "2.c-.1,2.\u{e9}f";
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
            Descriptor { global: true, ..descriptor(Shape::Get, "", "x") }.root(),
            Root::Global,
        );
        assert_eq!(descriptor(Shape::Method, "", "x").root(), Root::Argument);
        assert_eq!(descriptor(Shape::Method, "chart.js", "x").root(), Root::Module);
        // A call and a construction are global without a module and never act on an argument.
        assert_eq!(descriptor(Shape::Call, "", "x").root(), Root::Global);
        assert_eq!(descriptor(Shape::New, "", "x").root(), Root::Global);
        assert_eq!(descriptor(Shape::New, "chart.js", "x").root(), Root::Module);
    }

    #[test]
    fn an_index_is_rooted_at_its_argument_whatever_module_it_came_from() {
        // The chart fixture's own shape: a block module plus an index that walks from the
        // receiver. Moving it to the module binding would change what every index vector emits.
        assert_eq!(descriptor(Shape::Index, "chart.js", "data.datasets").root(), Root::Argument);
        assert_eq!(descriptor(Shape::IndexSet, "chart.js", "").root(), Root::Argument);
        // The flag is how an index off a global says so.
        assert_eq!(
            Descriptor { global: true, ..descriptor(Shape::Index, "", "sessionStorage") }.root(),
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
        // The flag takes the receiver away exactly as a module does.
        let global = |shape| {
            Descriptor { global: true, ..descriptor(shape, "", "x") }.required_arguments()
        };
        assert_eq!(global(Shape::Get), 0);
        assert_eq!(global(Shape::Set), 1);
        assert_eq!(global(Shape::Index), 1);
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

//! What a declared host binding does with each of its slots, as terms on the value-flow EDGES
//! that reach it.
//!
//! v1 asked one question of a host call — does any argument or the destination pin? — and answered
//! it by marking the whole ITEM baseline (`representation_plan.rs::collect_host_call`). That is
//! why a single opaque handle anywhere in a function kept every string in it baseline. A host slot
//! constrains the one edge into it and nothing else, so this module answers per slot, and the
//! planner puts the term on that edge.
//!
//! Two sources answer the question, in this order:
//!
//! 1. What the declaration SAYS. A version 3 `#[js_extern]` descriptor carries, per slot, the
//!    carriers the host accepts and how it observes the value (`descriptor::SlotFacts`).
//! 2. What the slot's Rust TYPE implies, for a slot the declaration said nothing about. That is
//!    [`HostParameterClass::of`] exactly, which is what every declaration written before the facts
//!    existed keeps meaning: a version 2 descriptor annotates nothing, so every one of its slots
//!    takes this path and nothing about it moves.

// The planner is what reads this: the legality terms (W4a) and the min-cut edges (W4b) are its
// only consumers, and this half lands first because it is producer visible and the schema freezes
// with it. Until then nothing calls it, which is the design and not an oversight.
#![allow(dead_code)]

use crate::descriptor::{Descriptor, Fact, Slot, SlotFacts};
use crate::jir::semantic::{HostParameterClass, TypeId};
use crate::jir::types::TypeTable;

/// What one slot of a host call does to the edge that feeds it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotTerm {
    /// The slot only reads a value it accepts in that shape, so crossing costs nothing: a
    /// zero-cost edge to the sink side of the cut.
    Sink,
    /// The slot observes something a carrier would change — it writes through the value and keeps
    /// it, observes its identity or its bytes, or accepts no carrier but the baseline. The edge is
    /// must-equal-baseline.
    Pin,
    /// The slot takes the value over. Legal off the baseline only where the source is dead and
    /// unaliased after the edge, which is the planner's directional rule.
    Transfer,
}

/// One slot's contract: what it does to its edge, and the carriers it accepts if it lets the edge
/// move at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlotContract {
    pub slot: Slot,
    pub term: SlotTerm,
    /// The carriers the declaration says the host accepts here. Empty where it said nothing, which
    /// means the baseline and nothing else.
    pub accepts: SlotFacts,
    /// Whether the slot may throw. Feeds the call's effects rather than the cut.
    pub throws: bool,
    /// Whether the slot may suspend. Reserved for Phase 6; nothing reads it yet.
    pub suspends: bool,
}

/// The per-slot contract of one declared host binding.
pub struct HostContract;

impl HostContract {
    /// The contract of `descriptor`'s slots, given the Rust type of each.
    ///
    /// `slots` is the call's slots and their types, in the order the planner will attach the terms
    /// to edges: it is the caller that knows the call, and a descriptor on its own does not.
    pub fn of(
        descriptor: &Descriptor,
        slots: &[(Slot, TypeId)],
        types: &TypeTable,
    ) -> Vec<SlotContract> {
        slots
            .iter()
            .map(|(slot, ty)| Self::slot(descriptor, *slot, *ty, types))
            .collect()
    }

    /// The contract of one slot.
    pub fn slot(
        descriptor: &Descriptor,
        slot: Slot,
        ty: TypeId,
        types: &TypeTable,
    ) -> SlotContract {
        let Some(facts) = descriptor.slot(slot) else {
            // The declaration said nothing, so the type answers, exactly as it did before the
            // facts existed.
            return SlotContract {
                slot,
                term: Self::from_type(ty, types),
                accepts: SlotFacts::default(),
                throws: descriptor.fallible,
                suspends: false,
            };
        };
        let term = if facts.has(Fact::Retains) && facts.has(Fact::Mutates) {
            // The host writes through the value and keeps it: nothing but the baseline can be
            // handed over, because a carrier would have to be written back and there is no point
            // at which to write it.
            SlotTerm::Pin
        } else if facts.has(Fact::Identity) || facts.has(Fact::Bytes) || facts.has(Fact::Mutates) {
            SlotTerm::Pin
        } else if facts.has(Fact::Owns) {
            SlotTerm::Transfer
        } else if facts.accepts_a_carrier() {
            SlotTerm::Sink
        } else {
            // Annotated, but naming no carrier it accepts: the baseline is all it takes, and only
            // a value whose type is the same in both lanes crosses for free.
            Self::from_type(ty, types)
        };
        SlotContract {
            slot,
            term,
            accepts: facts,
            throws: descriptor.fallible || facts.has(Fact::Throws),
            suspends: facts.has(Fact::Suspends),
        }
    }

    /// The term a slot's Rust type implies on its own: v2's `HostParameterClass`, unchanged.
    fn from_type(ty: TypeId, types: &TypeTable) -> SlotTerm {
        match HostParameterClass::of(ty, types) {
            HostParameterClass::Sink => SlotTerm::Sink,
            HostParameterClass::Pin => SlotTerm::Pin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::Shape;
    use crate::jir::types::{JsFacts, LayoutFacts, TypeDescriptor, TypeKind, TypeMeta};

    const I32: TypeId = TypeId(1);
    const STR: TypeId = TypeId(2);
    const REF_STR: TypeId = TypeId(3);
    const MUT_I32: TypeId = TypeId(4);
    /// A host handle: a nominal type with no shape either lane shares.
    const OPAQUE: TypeId = TypeId(5);

    fn types() -> TypeTable {
        let mut table = TypeTable::default();
        for (id, kind) in [
            (
                I32,
                TypeKind::Integer {
                    signed: true,
                    bits: 32,
                },
            ),
            (STR, TypeKind::Str),
            (
                REF_STR,
                TypeKind::Reference {
                    pointee: STR,
                    mutable: false,
                },
            ),
            (
                MUT_I32,
                TypeKind::Reference {
                    pointee: I32,
                    mutable: true,
                },
            ),
            (OPAQUE, TypeKind::Slice(I32)),
        ] {
            table.insert(TypeDescriptor {
                id,
                name: None,
                kind,
                layout: LayoutFacts::default(),
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            });
        }
        table
    }

    fn declaration(slots: Vec<(Slot, SlotFacts)>) -> Descriptor {
        Descriptor {
            shape: Shape::Method,
            nullable: false,
            fallible: false,
            global: false,
            module: String::new(),
            name: "write".to_owned(),
            slots,
        }
    }

    fn facts(list: &[Fact]) -> SlotFacts {
        list.iter()
            .fold(SlotFacts::default(), |facts, fact| facts.with(*fact))
    }

    fn term(descriptor: &Descriptor, slot: Slot, ty: TypeId) -> SlotTerm {
        HostContract::slot(descriptor, slot, ty, &types()).term
    }

    #[test]
    fn an_unannotated_slot_is_classified_exactly_as_it_was_before_the_facts_existed() {
        // The version 2 guarantee, spelled as a test: nothing a declaration does not talk about
        // moves. Scalars and shared strings sink, everything else pins.
        let v2 = declaration(Vec::new());
        for (ty, expected) in [
            (I32, SlotTerm::Sink),
            (REF_STR, SlotTerm::Sink),
            (MUT_I32, SlotTerm::Pin),
            (OPAQUE, SlotTerm::Pin),
        ] {
            assert_eq!(term(&v2, Slot::Parameter(0), ty), expected, "{ty:?}");
            assert_eq!(
                HostContract::slot(&v2, Slot::Parameter(0), ty, &types()).accepts,
                SlotFacts::default()
            );
        }
    }

    #[test]
    fn a_reading_slot_that_accepts_a_carrier_is_a_free_edge() {
        let reads = declaration(vec![(
            Slot::Parameter(0),
            facts(&[Fact::Reads, Fact::HostString]),
        )]);
        // The type alone would pin an owned `String`; the declaration says the host only reads it
        // and takes a host string, so the edge is free.
        assert_eq!(term(&reads, Slot::Parameter(0), OPAQUE), SlotTerm::Sink);
    }

    #[test]
    fn a_slot_that_keeps_what_it_writes_pins_its_edge() {
        for observation in [Fact::Mutates, Fact::Identity, Fact::Bytes] {
            let pinned = declaration(vec![(
                Slot::Parameter(0),
                facts(&[observation, Fact::Retains, Fact::HostString]),
            )]);
            // Naming a carrier does not buy anything back: what the slot observes decides.
            assert_eq!(term(&pinned, Slot::Parameter(0), I32), SlotTerm::Pin);
        }
    }

    #[test]
    fn an_owning_slot_transfers_and_a_carrierless_one_falls_back_to_its_type() {
        let owns = declaration(vec![(
            Slot::Parameter(0),
            facts(&[Fact::Owns, Fact::NativeVec]),
        )]);
        assert_eq!(term(&owns, Slot::Parameter(0), OPAQUE), SlotTerm::Transfer);

        // Annotated, but naming no carrier: the declaration added a fact and took no shape, so the
        // type is still what answers.
        let bare = declaration(vec![(Slot::Parameter(0), facts(&[Fact::Reads]))]);
        assert_eq!(term(&bare, Slot::Parameter(0), OPAQUE), SlotTerm::Pin);
        assert_eq!(term(&bare, Slot::Parameter(0), I32), SlotTerm::Sink);
    }

    #[test]
    fn the_result_slot_is_answered_like_any_other_and_throws_follows_the_declaration() {
        let mut declared =
            declaration(vec![(Slot::Result, facts(&[Fact::Owns, Fact::HostString]))]);
        let contract = HostContract::slot(&declared, Slot::Result, OPAQUE, &types());
        assert_eq!(contract.term, SlotTerm::Transfer);
        assert!(contract.accepts.has(Fact::HostString));
        assert!(!contract.throws);
        // `fallible` is a declaration-wide throw, so every slot of one carries it.
        declared.fallible = true;
        assert!(HostContract::slot(&declared, Slot::Parameter(0), I32, &types()).throws);
    }

    #[test]
    fn every_slot_of_a_call_is_answered_in_the_order_it_was_asked_about() {
        let declared = declaration(vec![(
            Slot::Parameter(1),
            facts(&[Fact::Reads, Fact::HostString]),
        )]);
        let contracts = HostContract::of(
            &declared,
            &[
                (Slot::Parameter(0), OPAQUE),
                (Slot::Parameter(1), OPAQUE),
                (Slot::Result, I32),
            ],
            &types(),
        );
        assert_eq!(
            contracts.iter().map(|c| c.term).collect::<Vec<_>>(),
            [SlotTerm::Pin, SlotTerm::Sink, SlotTerm::Sink]
        );
        assert_eq!(
            contracts.iter().map(|c| c.slot).collect::<Vec<_>>(),
            [Slot::Parameter(0), Slot::Parameter(1), Slot::Result]
        );
    }
}

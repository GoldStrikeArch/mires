use super::super::witness::BoundsFact;
use super::*;
use crate::analysis::arena::ArenaFieldKind;
use crate::jir::repr::FuncBuilder;
use crate::jir::semantic::{AbiSignature, CallSemantic, Callee, DefinitionId, ItemId, Signature};
use crate::jir::types::{
    IntFacts, IntRepr, JsFacts, LayoutFacts, PointerForm, TypeDescriptor, TypeMeta,
};
use crate::jir::{OriginId, StrId};

const U32: TypeId = TypeId(1);
const U8: TypeId = TypeId(2);
const SLICE: TypeId = TypeId(3);
const REF_SLICE: TypeId = TypeId(4);
const NO_ORIGIN: OriginId = OriginId(0);

/// `u32`, `u8`, `[u8]` and `&[u8]`: enough for a bounds check over a slice.
fn table() -> TypeTable {
    let mut table = TypeTable::new();
    let js = |int: Option<IntFacts>, peeled: TypeId| JsFacts {
        int,
        is_indirect: false,
        needs_clone: false,
        crosses_transparent_union: false,
        transparent: None,
        peeled,
        unsized_tail: None,
        enum_repr: None,
        is_primitive_enum: false,
        pointer_form: None,
        byte_element: false,
        fields: Vec::new(),
    };
    let unsigned = |bits| {
        Some(IntFacts {
            signed: false,
            bits,
            repr: IntRepr::Number,
        })
    };
    let sized = LayoutFacts {
        sized: true,
        ..LayoutFacts::default()
    };
    table.insert(TypeDescriptor {
        id: U32,
        name: None,
        kind: TypeKind::Integer {
            signed: false,
            bits: 32,
        },
        layout: sized,
        meta: TypeMeta::default(),
        js: js(unsigned(32), U32),
    });
    table.insert(TypeDescriptor {
        id: U8,
        name: None,
        kind: TypeKind::Integer {
            signed: false,
            bits: 8,
        },
        layout: sized,
        meta: TypeMeta::default(),
        js: js(unsigned(8), U8),
    });
    table.insert(TypeDescriptor {
        id: SLICE,
        name: None,
        kind: TypeKind::Slice(U8),
        layout: LayoutFacts::default(),
        meta: TypeMeta::default(),
        js: js(None, SLICE),
    });
    let mut slice_ref = js(None, REF_SLICE);
    slice_ref.pointer_form = Some(PointerForm::FatSlice);
    table.insert(TypeDescriptor {
        id: REF_SLICE,
        name: None,
        kind: TypeKind::Reference {
            pointee: SLICE,
            mutable: false,
        },
        layout: sized,
        meta: TypeMeta::default(),
        js: slice_ref,
    });
    table
}

fn builder() -> FuncBuilder {
    FuncBuilder::new(
        ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: 0,
            },
            instantiation: 0,
        },
        Signature {
            abi: AbiSignature {
                parameters: 0,
                spread: None,
                caller_location: false,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
            arguments: Vec::new(),
            result: U32,
        },
    )
}

/// `FuncBuilder::finish` with the JavaScript parameter count synced.
fn finished(b: FuncBuilder) -> Func {
    let mut func = b.finish();
    func.sig.abi.parameters = func.params.len() as u16;
    func
}

fn constant(b: &mut FuncBuilder, block: RBlockId, value: u128) -> ValueId {
    b.op(
        block,
        OpKind::Const(ConstTree::Scalar {
            bits: value,
            bytes: 4,
        }),
        U32,
        NO_ORIGIN,
    )
}

fn metadata(b: &mut FuncBuilder, block: RBlockId, slice: ValueId) -> ValueId {
    b.op(
        block,
        OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v: slice,
            ty: REF_SLICE,
        },
        U32,
        NO_ORIGIN,
    )
}

fn compare(b: &mut FuncBuilder, block: RBlockId, op: BinaryOp, l: ValueId, r: ValueId) -> ValueId {
    b.op(block, OpKind::Binary { op, l, r, ty: U32 }, U32, NO_ORIGIN)
}

fn bounds_assert(b: &mut FuncBuilder, block: RBlockId, index: ValueId, length: ValueId) {
    let cond = compare(b, block, BinaryOp::Lt, index, length);
    b.effect(
        block,
        OpKind::Assert {
            cond,
            expected: true,
            msg: RAssertMessage::BoundsCheck { length, index },
            track_caller: None,
        },
        NO_ORIGIN,
    );
}

fn asserts(func: &Func) -> usize {
    func.blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter(|op| matches!(op.kind, OpKind::Assert { .. }))
        .count()
}

fn effect_call(b: &mut FuncBuilder, block: RBlockId) {
    b.effect(
        block,
        OpKind::Call {
            callee: Callee::Direct {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 9,
                    },
                    instantiation: 0,
                },
                link_name: StrId(0),
            },
            function: None,
            args: Vec::new(),
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        NO_ORIGIN,
    );
}

/// A kept block parameter does not hide the exact length of the slice view passed to it.
#[test]
fn a_slice_view_length_flows_through_a_block_parameter() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let source = b.param(REF_SLICE, NO_ORIGIN);
    let body = b.block();
    let slice = b.block_param(body, REF_SLICE, NO_ORIGIN);
    let sixteen = constant(&mut b, entry, 16);
    let view = b.op(
        entry,
        OpKind::SliceView {
            ptr: source,
            len: sixteen,
        },
        REF_SLICE,
        NO_ORIGIN,
    );
    b.terminate(entry, RTerm::Goto(body, vec![view]));
    let length = metadata(&mut b, body, slice);
    let fifteen = constant(&mut b, body, 15);
    bounds_assert(&mut b, body, fifteen, length);
    let zero = constant(&mut b, body, 0);
    b.terminate(body, RTerm::Return(Some(zero)));
    let mut func = finished(b);
    let mut witnesses = Vec::new();

    assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 1);
    assert_eq!(asserts(&func), 0);
    assert!(matches!(
        witnesses.as_slice(),
        [Witness::BoundsRemoved {
            fact: BoundsFact::Numeric {
                index_hi: 15,
                length_lo: 16
            },
            ..
        }]
    ));
}

/// A fixed `[from..to]` projection exposes its literal width to the bounds proof.
#[test]
fn a_fixed_subslice_has_an_exact_length() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let source = b.param(REF_SLICE, NO_ORIGIN);
    let view = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Value(source),
            path: vec![
                Proj::Deref,
                Proj::Subslice {
                    from: 2,
                    to: 6,
                    from_end: false,
                },
            ],
            mutable: false,
        },
        REF_SLICE,
        NO_ORIGIN,
    );
    let length = metadata(&mut b, entry, view);
    let three = constant(&mut b, entry, 3);
    bounds_assert(&mut b, entry, three, length);
    let zero = constant(&mut b, entry, 0);
    b.terminate(entry, RTerm::Return(Some(zero)));
    let mut func = finished(b);

    assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 1);
    assert_eq!(asserts(&func), 0);
}

/// An end-relative subslice keeps the exact source length minus its two fixed trims.
#[test]
fn an_end_relative_subslice_propagates_its_trimmed_length() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let source = b.param(REF_SLICE, NO_ORIGIN);
    let sixteen = constant(&mut b, entry, 16);
    let source = b.op(
        entry,
        OpKind::SliceView {
            ptr: source,
            len: sixteen,
        },
        REF_SLICE,
        NO_ORIGIN,
    );
    let view = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Value(source),
            path: vec![
                Proj::Deref,
                Proj::Subslice {
                    from: 1,
                    to: 2,
                    from_end: true,
                },
            ],
            mutable: false,
        },
        REF_SLICE,
        NO_ORIGIN,
    );
    let length = metadata(&mut b, entry, view);
    let twelve = constant(&mut b, entry, 12);
    bounds_assert(&mut b, entry, twelve, length);
    let zero = constant(&mut b, entry, 0);
    b.terminate(entry, RTerm::Return(Some(zero)));
    let mut func = finished(b);

    assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 1);
    assert_eq!(asserts(&func), 0);
}

/// `i & 7` against a constant length 8: the intervals alone prove the check.
#[test]
fn a_masked_index_against_a_constant_length_is_numeric() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let i = b.param(U32, NO_ORIGIN);
    let seven = constant(&mut b, entry, 7);
    let masked = compare(&mut b, entry, BinaryOp::BitAnd, i, seven);
    let eight = constant(&mut b, entry, 8);
    bounds_assert(&mut b, entry, masked, eight);
    let ret = constant(&mut b, entry, 0);
    b.terminate(entry, RTerm::Return(Some(ret)));
    let mut func = finished(b);
    let mut witnesses = Vec::new();

    assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 1);
    assert_eq!(asserts(&func), 0);
    assert!(matches!(
        witnesses.as_slice(),
        [Witness::BoundsRemoved {
            fact: BoundsFact::Numeric {
                index_hi: 7,
                length_lo: 8
            },
            ..
        }]
    ));
    super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
}

#[test]
fn a_constant_mask_bounds_an_unknown_operand() {
    assert_eq!(
        bitand_transfer(None, Some(Iv::point(15)), width_max(32)),
        Iv { lo: 0, hi: 15 }
    );
    assert_eq!(
        bitand_transfer(Some(Iv::point(7)), None, width_max(32)),
        Iv { lo: 0, hi: 7 }
    );
}

/// The guarded-body shape: `if i < s.len() { s[i] }`, with the assert re-reading the
/// length through a second `PtrMetadata`. Value numbering equates the two reads and the
/// edge fact proves the assert.
fn guarded(effectful: bool) -> (Func, Vec<Witness>, u32) {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let i = b.param(U32, NO_ORIGIN);
    let s = b.param(REF_SLICE, NO_ORIGIN);
    let body = b.block();
    let exit = b.block();
    let n = metadata(&mut b, entry, s);
    let guard = compare(&mut b, entry, BinaryOp::Lt, i, n);
    b.terminate(
        entry,
        RTerm::Switch {
            scrut: guard,
            scrut_ty: U32,
            cases: vec![(0, exit, Vec::new())],
            default: (body, Vec::new()),
        },
    );
    if effectful {
        effect_call(&mut b, body);
    }
    let len2 = metadata(&mut b, body, s);
    bounds_assert(&mut b, body, i, len2);
    let one = constant(&mut b, body, 1);
    b.terminate(body, RTerm::Return(Some(one)));
    let zero = constant(&mut b, exit, 0);
    b.terminate(exit, RTerm::Return(Some(zero)));
    let mut func = finished(b);
    let mut witnesses = Vec::new();
    let removed = run(&mut func, &table(), &mut witnesses).asserts_removed;
    (func, witnesses, removed)
}

/// The dominating guard removes the assert, and the witness re-checks.
#[test]
fn a_dominating_guard_removes_the_assert() {
    let (func, witnesses, removed) = guarded(false);
    assert_eq!(removed, 1);
    assert_eq!(asserts(&func), 0);
    assert!(matches!(
        witnesses.as_slice(),
        [Witness::BoundsRemoved {
            fact: BoundsFact::Dominating {
                offset_index: 0,
                offset_fact: 0,
                strict: true,
                ..
            },
            ..
        }]
    ));
    super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
}

/// An observable op between the guard and the assert refuses the removal (the ruling's
/// letter: a call may do anything, including changing what a length re-read would see).
#[test]
fn an_observable_op_between_keeps_the_assert() {
    let (func, witnesses, removed) = guarded(true);
    assert_eq!(removed, 0);
    assert_eq!(asserts(&func), 1);
    assert!(witnesses.is_empty());
}

/// A corrupted bounds witness fails its independent check.
#[test]
fn a_corrupted_bounds_witness_is_rejected() {
    let (func, mut witnesses, removed) = guarded(false);
    assert_eq!(removed, 1);
    for witness in &mut witnesses {
        if let Witness::BoundsRemoved { fact, .. } = witness {
            *fact = BoundsFact::Dominating {
                block: RBlockId(0),
                cond: ValueId(0),
                offset_index: 5,
                offset_fact: 1,
                strict: false,
            };
        }
    }
    assert!(super::super::witness::check_all(&witnesses, &func, &table()).is_err());
}

/// The chunked-guard shape: `i` bounded, guard `i + 2 <= n`, asserts on `i`, `i + 1` and
/// `i + 2`. The first two are covered (`k < 2`), the third is not.
#[test]
fn a_le_guard_covers_smaller_offsets_only() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let x = b.param(U32, NO_ORIGIN);
    let s = b.param(REF_SLICE, NO_ORIGIN);
    let body = b.block();
    let exit = b.block();
    // i = x & 0xffff bounds the base so `i + k` certifiably cannot wrap.
    let mask_c = constant(&mut b, entry, 0xffff);
    let i = compare(&mut b, entry, BinaryOp::BitAnd, x, mask_c);
    let n = metadata(&mut b, entry, s);
    let two = constant(&mut b, entry, 2);
    let i2 = b.op(
        entry,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: two,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    let guard = compare(&mut b, entry, BinaryOp::Le, i2, n);
    b.terminate(
        entry,
        RTerm::Switch {
            scrut: guard,
            scrut_ty: U32,
            cases: vec![(0, exit, Vec::new())],
            default: (body, Vec::new()),
        },
    );
    let one = constant(&mut b, body, 1);
    let i1 = b.op(
        body,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: one,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    let len2 = metadata(&mut b, body, s);
    bounds_assert(&mut b, body, i, len2); // k = 0 < 2: removed
    bounds_assert(&mut b, body, i1, len2); // k = 1 < 2: removed
    let two_b = constant(&mut b, body, 2);
    let i2b = b.op(
        body,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: two_b,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    bounds_assert(&mut b, body, i2b, len2); // k = 2, not < 2: stays
    let ret = constant(&mut b, body, 0);
    b.terminate(body, RTerm::Return(Some(ret)));
    let zero = constant(&mut b, exit, 0);
    b.terminate(exit, RTerm::Return(Some(zero)));
    let mut func = finished(b);
    let mut witnesses = Vec::new();

    assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 2);
    assert_eq!(asserts(&func), 1, "the k = 2 assert stays");
    super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
}

/// A length read from a different slice proves nothing.
#[test]
fn a_different_slice_length_proves_nothing() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let i = b.param(U32, NO_ORIGIN);
    let s = b.param(REF_SLICE, NO_ORIGIN);
    let t = b.param(REF_SLICE, NO_ORIGIN);
    let body = b.block();
    let exit = b.block();
    let n = metadata(&mut b, entry, s);
    let guard = compare(&mut b, entry, BinaryOp::Lt, i, n);
    b.terminate(
        entry,
        RTerm::Switch {
            scrut: guard,
            scrut_ty: U32,
            cases: vec![(0, exit, Vec::new())],
            default: (body, Vec::new()),
        },
    );
    let other = metadata(&mut b, body, t);
    bounds_assert(&mut b, body, i, other);
    let one = constant(&mut b, body, 1);
    b.terminate(body, RTerm::Return(Some(one)));
    let zero = constant(&mut b, exit, 0);
    b.terminate(exit, RTerm::Return(Some(zero)));
    let mut func = finished(b);

    assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 0);
    assert_eq!(asserts(&func), 1);
}

/// The counted-loop shape the fixture compiles to: `while i + 4 <= n` over a slice, the
/// body indexing `s[i]`. The back-edge refinement bounds `i` by the slice-length cap, the
/// widening round converges, and the peel certificate holds.
#[test]
fn a_counted_loop_guard_removes_the_body_assert() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let s = b.param(REF_SLICE, NO_ORIGIN);
    let header = b.block();
    let i = b.block_param(header, U32, NO_ORIGIN);
    let body = b.block();
    let exit = b.block();
    let zero = constant(&mut b, entry, 0);
    let n = metadata(&mut b, entry, s);
    b.terminate(entry, RTerm::Goto(header, vec![zero]));
    let four = constant(&mut b, header, 4);
    let stepped = b.op(
        header,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: four,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    let guard = compare(&mut b, header, BinaryOp::Le, stepped, n);
    b.terminate(
        header,
        RTerm::Switch {
            scrut: guard,
            scrut_ty: U32,
            cases: vec![(0, exit, Vec::new())],
            default: (body, Vec::new()),
        },
    );
    let len2 = metadata(&mut b, body, s);
    bounds_assert(&mut b, body, i, len2);
    // The increment is a fresh `i + 4`, as MIR spells it; value numbering has to see it as
    // the guard's `stepped`.
    let four_b = constant(&mut b, body, 4);
    let next = b.op(
        body,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: four_b,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    b.terminate(body, RTerm::Goto(header, vec![next]));
    let ret = constant(&mut b, exit, 0);
    b.terminate(exit, RTerm::Return(Some(ret)));
    let mut func = finished(b);
    let mut witnesses = Vec::new();

    assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 1);
    assert_eq!(asserts(&func), 0);
    super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
}

/// A length carried unchanged through a loop header remains the same numbered scalar fact.
#[test]
fn an_unchanged_loop_carried_length_keeps_its_exact_fact() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let header = b.block();
    let i = b.block_param(header, U32, NO_ORIGIN);
    let length = b.block_param(header, U32, NO_ORIGIN);
    let body = b.block();
    let exit = b.block();
    let zero = constant(&mut b, entry, 0);
    let sixteen = constant(&mut b, entry, 16);
    b.terminate(entry, RTerm::Goto(header, vec![zero, sixteen]));

    let fifteen = constant(&mut b, header, 15);
    bounds_assert(&mut b, header, fifteen, length);
    let one = constant(&mut b, header, 1);
    let keep_going = compare(&mut b, header, BinaryOp::Lt, i, one);
    b.terminate(
        header,
        RTerm::Switch {
            scrut: keep_going,
            scrut_ty: U32,
            cases: vec![(0, exit, Vec::new())],
            default: (body, Vec::new()),
        },
    );
    let next = b.op(
        body,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: one,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    b.terminate(body, RTerm::Goto(header, vec![next, length]));
    let ret = constant(&mut b, exit, 0);
    b.terminate(exit, RTerm::Return(Some(ret)));
    let mut func = finished(b);

    assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 1);
    assert_eq!(asserts(&func), 0);
}

/// The same loop with a call in the body keeps its assert: the establishment-free cycle
/// through the body carries an observable op.
#[test]
fn a_counted_loop_with_a_call_keeps_the_assert() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let s = b.param(REF_SLICE, NO_ORIGIN);
    let header = b.block();
    let i = b.block_param(header, U32, NO_ORIGIN);
    let body = b.block();
    let exit = b.block();
    let zero = constant(&mut b, entry, 0);
    let n = metadata(&mut b, entry, s);
    b.terminate(entry, RTerm::Goto(header, vec![zero]));
    let four = constant(&mut b, header, 4);
    let stepped = b.op(
        header,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: four,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    let guard = compare(&mut b, header, BinaryOp::Le, stepped, n);
    b.terminate(
        header,
        RTerm::Switch {
            scrut: guard,
            scrut_ty: U32,
            cases: vec![(0, exit, Vec::new())],
            default: (body, Vec::new()),
        },
    );
    effect_call(&mut b, body);
    let len2 = metadata(&mut b, body, s);
    bounds_assert(&mut b, body, i, len2);
    let four_b = constant(&mut b, body, 4);
    let next = b.op(
        body,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: four_b,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    b.terminate(body, RTerm::Goto(header, vec![next]));
    let ret = constant(&mut b, exit, 0);
    b.terminate(exit, RTerm::Return(Some(ret)));
    let mut func = finished(b);

    assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 0);
    assert_eq!(asserts(&func), 1);
}

#[test]
fn an_aligned_slice_prefix_folds_only_its_two_proven_relations() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let length = b.param(U32, NO_ORIGIN);
    let chunk = constant(&mut b, entry, 16);
    let remainder = b.op(
        entry,
        OpKind::Binary {
            op: BinaryOp::Rem,
            l: length,
            r: chunk,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    let prefix = b.op(
        entry,
        OpKind::Binary {
            op: BinaryOp::Sub,
            l: length,
            r: remainder,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    let zero = constant(&mut b, entry, 0);
    let below_zero = compare(&mut b, entry, BinaryOp::Lt, prefix, zero);
    let within_length = compare(&mut b, entry, BinaryOp::Le, prefix, length);
    // Equality is not implied: when `length` is a multiple of 16 it holds, otherwise it does
    // not. Keeping this comparison is the guard against turning the rule into a theorem
    // prover by accident.
    let maybe_equal = compare(&mut b, entry, BinaryOp::Eq, prefix, length);
    b.terminate(entry, RTerm::Return(Some(within_length)));
    let mut func = finished(b);
    let mut witnesses = Vec::new();

    let stats = run(&mut func, &table(), &mut witnesses);
    assert_eq!(
        stats,
        RangeStats {
            asserts_removed: 0,
            comparisons_folded: 2,
        }
    );
    let kind = |value| {
        func.blocks[0]
            .ops
            .iter()
            .find(|op| op.result == Some(value))
            .map(|op| &op.kind)
            .expect("the value remains defined")
    };
    assert!(matches!(
        kind(below_zero),
        OpKind::Const(ConstTree::Scalar { bits: 0, .. })
    ));
    assert!(matches!(
        kind(within_length),
        OpKind::Const(ConstTree::Scalar { bits: 1, .. })
    ));
    assert!(matches!(
        kind(maybe_equal),
        OpKind::Binary {
            op: BinaryOp::Eq,
            ..
        }
    ));
    super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
}

#[test]
fn a_dominating_constant_bound_removes_smaller_constant_asserts() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let s = b.param(REF_SLICE, NO_ORIGIN);
    let body = b.block();
    let exit = b.block();
    let len = metadata(&mut b, entry, s);
    let fifteen = constant(&mut b, entry, 15);
    let guard = compare(&mut b, entry, BinaryOp::Lt, fifteen, len);
    b.terminate(
        entry,
        RTerm::Switch {
            scrut: guard,
            scrut_ty: U32,
            cases: vec![(0, exit, Vec::new())],
            default: (body, Vec::new()),
        },
    );
    let len2 = metadata(&mut b, body, s);
    for k in [0u128, 1, 7, 15, 16] {
        let idx = constant(&mut b, body, k);
        bounds_assert(&mut b, body, idx, len2);
    }
    let ret = constant(&mut b, body, 1);
    b.terminate(body, RTerm::Return(Some(ret)));
    let zero = constant(&mut b, exit, 0);
    b.terminate(exit, RTerm::Return(Some(zero)));
    let mut func = finished(b);
    let mut witnesses = Vec::new();

    assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 4);
    assert_eq!(asserts(&func), 1, "16 < len is not implied by 15 < len");
    super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
}

#[test]
fn a_dominating_ge_or_eq_constant_bound_removes_constant_asserts() {
    for (op, expected_case, len_bound) in [
        (BinaryOp::Ge, 1u128, 16u128),
        (BinaryOp::Lt, 0, 16), // !(len < 16)
        (BinaryOp::Eq, 1, 16), // len == 16
    ] {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let body = b.block();
        let exit = b.block();
        let len = metadata(&mut b, entry, s);
        let bound = constant(&mut b, entry, len_bound);
        let guard = compare(&mut b, entry, op, len, bound);
        let (cases, default) = if expected_case == 1 {
            (vec![(0, exit, Vec::new())], (body, Vec::new()))
        } else {
            (vec![(0, body, Vec::new())], (exit, Vec::new()))
        };
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: guard,
                scrut_ty: U32,
                cases,
                default,
            },
        );
        for k in [0u128, 15, 16] {
            let idx = constant(&mut b, body, k);
            bounds_assert(&mut b, body, idx, len);
        }
        let ret = constant(&mut b, body, 1);
        b.terminate(body, RTerm::Return(Some(ret)));
        let zero = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();

        assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 2);
        assert_eq!(asserts(&func), 1, "16 < len stays");
        super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
    }
}

#[test]
fn a_switch_matched_constant_length_removes_constant_asserts() {
    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let s = b.param(REF_SLICE, NO_ORIGIN);
    let body = b.block();
    let exit = b.block();
    let len = metadata(&mut b, entry, s);
    b.terminate(
        entry,
        RTerm::Switch {
            scrut: len,
            scrut_ty: U32,
            cases: vec![(16, body, Vec::new())],
            default: (exit, Vec::new()),
        },
    );
    for k in [0u128, 15, 16] {
        let idx = constant(&mut b, body, k);
        bounds_assert(&mut b, body, idx, len);
    }
    let ret = constant(&mut b, body, 1);
    b.terminate(body, RTerm::Return(Some(ret)));
    let zero = constant(&mut b, exit, 0);
    b.terminate(exit, RTerm::Return(Some(zero)));
    let mut func = finished(b);
    let mut witnesses = Vec::new();

    assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 2);
    assert_eq!(asserts(&func), 1);
    super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
}

fn str_and_raw_table() -> (TypeTable, TypeId, TypeId, TypeId) {
    const RAW_U8: TypeId = TypeId(5);
    const STR: TypeId = TypeId(6);
    const REF_STR: TypeId = TypeId(7);

    let mut tbl = table();
    let make = |id, kind, pointer_form| TypeDescriptor {
        id,
        name: None,
        kind,
        layout: LayoutFacts {
            sized: id != STR,
            ..LayoutFacts::default()
        },
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
            pointer_form,
            byte_element: false,
            fields: Vec::new(),
        },
    };
    tbl.insert(make(
        RAW_U8,
        TypeKind::RawPointer {
            pointee: U8,
            mutable: false,
        },
        None,
    ));
    tbl.insert(make(STR, TypeKind::Str, None));
    tbl.insert(make(
        REF_STR,
        TypeKind::Reference {
            pointee: STR,
            mutable: false,
        },
        Some(PointerForm::FatSlice),
    ));
    (tbl, RAW_U8, STR, REF_STR)
}

#[test]
fn peels_raw_parts_byte_slice_ref_and_str_cast_with_interns() {
    let (tbl, raw_u8, _str_ty, ref_str) = str_and_raw_table();
    let mut interns = Interns::default();
    interns.strings.push("slice_from_parts".to_string()); // StrId(0)
    interns.strings.push("byte_slice_ref".to_string()); // StrId(1)
    interns.strings.push("unknown_runtime".to_string()); // StrId(2)

    // Positive: `slice_from_parts(ptr, 16)` -> `byte_slice_ref` -> `&[u8] -> &str` cast -> `PtrMetadata`.
    {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let ptr = b.param(raw_u8, NO_ORIGIN);
        let sixteen = constant(&mut b, entry, 16);
        let raw_slice = b.op(
            entry,
            OpKind::Call {
                callee: Callee::Runtime {
                    member: StrId(0),
                    sysroot: true,
                },
                function: None,
                args: vec![ptr, sixteen],
                track_caller: None,
                semantic: CallSemantic::RawParts,
                host: None,
            },
            REF_SLICE,
            NO_ORIGIN,
        );
        let byte_slice = b.op(
            entry,
            OpKind::Call {
                callee: Callee::Runtime {
                    member: StrId(1),
                    sysroot: true,
                },
                function: None,
                args: vec![raw_slice],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            REF_SLICE,
            NO_ORIGIN,
        );
        let as_str = b.op(
            entry,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: byte_slice,
                from: REF_SLICE,
                to: ref_str,
            },
            ref_str,
            NO_ORIGIN,
        );
        let meta = b.op(
            entry,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: as_str,
                ty: ref_str,
            },
            U32,
            NO_ORIGIN,
        );
        let idx15 = constant(&mut b, entry, 15);
        bounds_assert(&mut b, entry, idx15, meta);
        let idx16 = constant(&mut b, entry, 16);
        bounds_assert(&mut b, entry, idx16, meta);
        b.terminate(entry, RTerm::Return(Some(meta)));

        let mut func = finished(b);
        let mut witnesses = Vec::new();
        let res = run_with_interns_and_facts(&mut func, &tbl, &interns, None, &mut witnesses);
        assert_eq!(res.asserts_removed, 1, "15 < 16 eliminated, 16 < 16 kept");
        assert_eq!(asserts(&func), 1);
        super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
    }

    // Negative controls: `sysroot: false`, `Callee::Foreign`, and unknown sysroot fn must NOT peel.
    for (label, callee) in [
        (
            "sysroot_false",
            Callee::Runtime {
                member: StrId(0),
                sysroot: false,
            },
        ),
        (
            "foreign",
            Callee::Foreign {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 0,
                },
                symbol: StrId(0),
            },
        ),
        (
            "unknown_sysroot",
            Callee::Runtime {
                member: StrId(2),
                sysroot: true,
            },
        ),
    ] {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let ptr = b.param(raw_u8, NO_ORIGIN);
        let sixteen = constant(&mut b, entry, 16);
        let s = b.op(
            entry,
            OpKind::Call {
                callee,
                function: None,
                args: vec![ptr, sixteen],
                track_caller: None,
                semantic: CallSemantic::RawParts,
                host: None,
            },
            REF_SLICE,
            NO_ORIGIN,
        );
        let meta = metadata(&mut b, entry, s);
        let idx15 = constant(&mut b, entry, 15);
        bounds_assert(&mut b, entry, idx15, meta);
        b.terminate(entry, RTerm::Return(Some(meta)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();
        let res = run_with_interns_and_facts(&mut func, &tbl, &interns, None, &mut witnesses);
        assert_eq!(
            res.asserts_removed, 0,
            "{label} must not peel slice_from_parts"
        );
    }
}

#[test]
fn loop_induction_signed_peel_and_local_assert_eliminate_bounds_checks() {
    let tbl = table();
    let interns = Interns::default();

    // 1. Stepped loop `for i in 0..10 { assert(i + 40 < 50); assert(i + 40 < 50); }`
    //    proves `i in [0, 9]` via stepped back-edge refinement and eliminates both asserts
    //    (including when a duplicate local assert follows in the same block).
    {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let header = b.block();
        let body = b.block();
        let exit = b.block();
        let zero = constant(&mut b, entry, 0);
        let one = constant(&mut b, entry, 1);
        let ten = constant(&mut b, entry, 10);
        let forty = constant(&mut b, entry, 40);
        let fifty = constant(&mut b, entry, 50);
        b.terminate(entry, RTerm::Goto(header, vec![zero]));
        let i = b.block_param(header, U32, NO_ORIGIN);
        let cond = compare(&mut b, header, BinaryOp::Lt, i, ten);
        b.terminate(
            header,
            RTerm::Switch {
                scrut: cond,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        let idx = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: forty,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        bounds_assert(&mut b, body, idx, fifty);
        bounds_assert(&mut b, body, idx, fifty);
        let next = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: one,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(body, RTerm::Goto(header, vec![next]));
        let ret = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(ret)));

        let mut func = finished(b);
        let mut witnesses = Vec::new();
        let res = run_with_interns_and_facts(&mut func, &tbl, &interns, None, &mut witnesses);
        assert_eq!(
            res.asserts_removed, 2,
            "both i + 40 < 50 asserts should fold"
        );
        assert_eq!(asserts(&func), 0);
        super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
    }

    // 2. Signed peel across loop `16..64`: `assert(p - 2 < len)` dominates `assert(p - 7 < len)`
    //    and `assert(p - 16 < len)` because widening preserves `lo = 16`, guaranteeing no underflow.
    {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let header = b.block();
        let body = b.block();
        let exit = b.block();
        let slice = b.param(REF_SLICE, NO_ORIGIN);
        let len = metadata(&mut b, entry, slice);
        let c1 = constant(&mut b, entry, 1);
        let c2 = constant(&mut b, entry, 2);
        let c7 = constant(&mut b, entry, 7);
        let c16 = constant(&mut b, entry, 16);
        let c64 = constant(&mut b, entry, 64);
        b.terminate(entry, RTerm::Goto(header, vec![c16]));
        let p = b.block_param(header, U32, NO_ORIGIN);
        let cond = compare(&mut b, header, BinaryOp::Lt, p, c64);
        b.terminate(
            header,
            RTerm::Switch {
                scrut: cond,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        let pm2 = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Sub,
                l: p,
                r: c2,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let pm7 = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Sub,
                l: p,
                r: c7,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let pm16 = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Sub,
                l: p,
                r: c16,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        bounds_assert(&mut b, body, pm2, len);
        bounds_assert(&mut b, body, pm7, len);
        bounds_assert(&mut b, body, pm16, len);
        let next = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: p,
                r: c1,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(body, RTerm::Goto(header, vec![next]));
        let ret = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(ret)));

        let mut func = finished(b);
        let mut witnesses = Vec::new();
        let res = run_with_interns_and_facts(&mut func, &tbl, &interns, None, &mut witnesses);
        assert_eq!(
            res.asserts_removed, 2,
            "p - 7 < len and p - 16 < len must be eliminated from p - 2 < len"
        );
        assert_eq!(asserts(&func), 1);
        super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
    }
}

#[test]
fn cast_wrapped_duplicate_bounds_check_eliminated_across_field_store() {
    let mut tbl = table();
    const USIZE: TypeId = TypeId(20);
    const REF_MUT_U32: TypeId = TypeId(21);
    tbl.insert(TypeDescriptor {
        id: USIZE,
        name: None,
        kind: TypeKind::Integer {
            signed: false,
            bits: 32,
        },
        layout: LayoutFacts {
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: JsFacts {
            int: Some(IntFacts {
                signed: false,
                bits: 32,
                repr: IntRepr::Number,
            }),
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled: USIZE,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        },
    });
    tbl.insert(TypeDescriptor {
        id: REF_MUT_U32,
        name: None,
        kind: TypeKind::Reference {
            pointee: U32,
            mutable: true,
        },
        layout: LayoutFacts {
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled: REF_MUT_U32,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: Some(PointerForm::Slot),
            byte_element: false,
            fields: Vec::new(),
        },
    });

    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let g = b.param(U32, NO_ORIGIN);
    let s = b.param(REF_SLICE, NO_ORIGIN);
    let field_ptr = b.param(REF_MUT_U32, NO_ORIGIN);
    let len1 = metadata(&mut b, entry, s);
    let t = b.op(
        entry,
        OpKind::Cast {
            kind: CastKind::IntToInt,
            v: g,
            from: U32,
            to: USIZE,
        },
        USIZE,
        NO_ORIGIN,
    );
    bounds_assert(&mut b, entry, t, len1);
    // Non-Vec scalar StoreThrough cannot change slice length!
    b.effect(
        entry,
        OpKind::StoreThrough {
            ptr: field_ptr,
            v: g,
        },
        NO_ORIGIN,
    );
    let len2 = metadata(&mut b, entry, s);
    let j = b.op(
        entry,
        OpKind::Cast {
            kind: CastKind::IntToInt,
            v: g,
            from: U32,
            to: USIZE,
        },
        USIZE,
        NO_ORIGIN,
    );
    bounds_assert(&mut b, entry, j, len2);
    let zero = constant(&mut b, entry, 0);
    b.terminate(entry, RTerm::Return(Some(zero)));
    let mut func = finished(b);
    let mut witnesses = Vec::new();

    assert_eq!(run(&mut func, &tbl, &mut witnesses).asserts_removed, 1);
    assert_eq!(asserts(&func), 1);
    super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
}

#[test]
fn bitmask_one_shl_exact_alloc_len_eliminates_and_adversarial_cases_keep() {
    use crate::jir::types::{AllocatorIdentity, RepresentationCandidate};

    let mut tbl = table();
    const VEC_U8: TypeId = TypeId(30);
    const REF_VEC_U8: TypeId = TypeId(31);
    tbl.insert(TypeDescriptor {
        id: VEC_U8,
        name: None,
        kind: TypeKind::Adt {
            def: DefinitionId {
                crate_id: 2,
                local_hash: 77,
            },
            form: crate::jir::types::NominalForm::Struct,
            args: vec![U8],
            variants: Vec::new(),
            niche: None,
        },
        layout: LayoutFacts {
            size: Some(12),
            align: Some(4),
            sized: true,
            zst: false,
            needs_drop: false,
            ..LayoutFacts::default()
        },
        meta: TypeMeta {
            candidate: RepresentationCandidate::Vec,
            allocator: AllocatorIdentity::Global,
            ..TypeMeta::default()
        },
        js: JsFacts {
            int: None,
            is_indirect: true,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled: VEC_U8,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        },
    });
    tbl.insert(TypeDescriptor {
        id: REF_VEC_U8,
        name: None,
        kind: TypeKind::Reference {
            pointee: VEC_U8,
            mutable: false,
        },
        layout: LayoutFacts {
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled: REF_VEC_U8,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: Some(PointerForm::Thin),
            byte_element: false,
            fields: Vec::new(),
        },
    });

    // Positive: `len = 1 << mb`, `mask = len - 1`, `v = nv_zeroed(len)`, `sl = v.as_slice()`, `x & mask < sl.len()`.
    {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let x = b.param(U32, NO_ORIGIN);
        let mb = b.param(U32, NO_ORIGIN);
        let one = constant(&mut b, entry, 1);
        let len = compare(&mut b, entry, BinaryOp::Shl, one, mb);
        let mask_v = compare(&mut b, entry, BinaryOp::Sub, len, one);
        let cell = b.cell(VEC_U8, false, NO_ORIGIN);
        b.effect(entry, OpKind::CellAlloc(cell), NO_ORIGIN);
        let vec_v = b.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 1,
                        },
                        instantiation: 0,
                    },
                    link_name: StrId(0),
                },
                function: None,
                args: vec![len],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecFromElemZeroed),
                host: None,
            },
            VEC_U8,
            NO_ORIGIN,
        );
        b.effect(entry, OpKind::CellStore { cell, v: vec_v }, NO_ORIGIN);
        let vref = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Cell(cell),
                path: Vec::new(),
                mutable: false,
            },
            REF_VEC_U8,
            NO_ORIGIN,
        );
        let sl = b.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 2,
                        },
                        instantiation: 0,
                    },
                    link_name: StrId(0),
                },
                function: None,
                args: vec![vref],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::VecAsSlice),
                host: None,
            },
            REF_SLICE,
            NO_ORIGIN,
        );
        let meta = metadata(&mut b, entry, sl);
        let idx = compare(&mut b, entry, BinaryOp::BitAnd, x, mask_v);
        bounds_assert(&mut b, entry, idx, meta);
        let zero = constant(&mut b, entry, 0);
        b.terminate(entry, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();
        assert_eq!(run(&mut func, &tbl, &mut witnesses).asserts_removed, 1);
        assert_eq!(asserts(&func), 0);
        assert!(matches!(
            witnesses.as_slice(),
            [Witness::BoundsRemoved {
                fact: BoundsFact::Bitmask { .. },
                ..
            }]
        ));
        super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
    }

    // Adversarial A: `mask = sl.len() - 1` when `sl.len()` may be `0` -> MUST keep assert!
    {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let x = b.param(U32, NO_ORIGIN);
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let n = metadata(&mut b, entry, s);
        let one = constant(&mut b, entry, 1);
        let mask_v = compare(&mut b, entry, BinaryOp::Sub, n, one);
        let idx = compare(&mut b, entry, BinaryOp::BitAnd, x, mask_v);
        bounds_assert(&mut b, entry, idx, n);
        let zero = constant(&mut b, entry, 0);
        b.terminate(entry, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        assert_eq!(run(&mut func, &tbl, &mut Vec::new()).asserts_removed, 0);
        assert_eq!(asserts(&func), 1);
    }

    // Adversarial B: wrong mask `x & len` (not `len - 1`) with `len = 1 << mb` -> MUST keep assert!
    {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let x = b.param(U32, NO_ORIGIN);
        let mb = b.param(U32, NO_ORIGIN);
        let one = constant(&mut b, entry, 1);
        let len = compare(&mut b, entry, BinaryOp::Shl, one, mb);
        let idx = compare(&mut b, entry, BinaryOp::BitAnd, x, len);
        bounds_assert(&mut b, entry, idx, len);
        let zero = constant(&mut b, entry, 0);
        b.terminate(entry, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        assert_eq!(run(&mut func, &tbl, &mut Vec::new()).asserts_removed, 0);
        assert_eq!(asserts(&func), 1);
    }
}

#[test]
fn monotone_arena_eliminates_always_valid_and_requires_sentinel_guard() {
    use crate::analysis::arena::ArenaInvariant;
    use crate::jir::types::{FieldDescriptor, FieldKey, NominalForm, VariantDescriptor};

    let mut tbl = table();
    const NODE_TY: TypeId = TypeId(40);
    const SLICE_NODE: TypeId = TypeId(41);
    const REF_SLICE_NODE: TypeId = TypeId(42);
    const REF_U32: TypeId = TypeId(43);
    const NODE_DEF: DefinitionId = DefinitionId {
        crate_id: 42,
        local_hash: 1,
    };
    let mk_js = |peeled, pf| JsFacts {
        int: None,
        is_indirect: false,
        needs_clone: false,
        crosses_transparent_union: false,
        transparent: None,
        peeled,
        unsized_tail: None,
        enum_repr: None,
        is_primitive_enum: false,
        pointer_form: pf,
        byte_element: false,
        fields: Vec::new(),
    };
    tbl.insert(TypeDescriptor {
        id: NODE_TY,
        name: None,
        kind: TypeKind::Adt {
            def: NODE_DEF,
            form: NominalForm::Struct,
            args: Vec::new(),
            variants: vec![VariantDescriptor {
                name: StrId(0),
                fields: vec![
                    FieldDescriptor {
                        key: FieldKey::Index(0),
                        ty: U32,
                        offset: 0,
                    },
                    FieldDescriptor {
                        key: FieldKey::Index(1),
                        ty: U32,
                        offset: 4,
                    },
                ],
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        layout: LayoutFacts {
            size: Some(8),
            align: Some(4),
            sized: true,
            zst: false,
            needs_drop: false,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: mk_js(NODE_TY, None),
    });
    tbl.insert(TypeDescriptor {
        id: SLICE_NODE,
        name: None,
        kind: TypeKind::Slice(NODE_TY),
        layout: LayoutFacts::default(),
        meta: TypeMeta::default(),
        js: mk_js(SLICE_NODE, None),
    });
    tbl.insert(TypeDescriptor {
        id: REF_SLICE_NODE,
        name: None,
        kind: TypeKind::Reference {
            pointee: SLICE_NODE,
            mutable: false,
        },
        layout: LayoutFacts {
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: mk_js(REF_SLICE_NODE, Some(PointerForm::FatSlice)),
    });
    tbl.insert(TypeDescriptor {
        id: REF_U32,
        name: None,
        kind: TypeKind::Reference {
            pointee: U32,
            mutable: false,
        },
        layout: LayoutFacts {
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: mk_js(REF_U32, Some(PointerForm::Slot)),
    });

    let mut facts = ProgramBoundsFacts::default();
    let mut fields = BTreeMap::new();
    fields.insert(0, ArenaFieldKind::AlwaysValid);
    fields.insert(
        1,
        ArenaFieldKind::ValidOrSentinel {
            sentinel: u32::MAX as u128,
        },
    );
    facts.arenas.insert(
        NODE_DEF,
        ArenaInvariant {
            elem_def: NODE_DEF,
            fields,
            holder_fields: BTreeSet::new(),
        },
    );

    let mut b = builder();
    let entry = FuncBuilder::ENTRY;
    let nodes = b.param(REF_SLICE_NODE, NO_ORIGIN);
    let idx = b.param(U32, NO_ORIGIN);
    let len = b.op(
        entry,
        OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v: nodes,
            ty: REF_SLICE_NODE,
        },
        U32,
        NO_ORIGIN,
    );
    let prev_ref = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Value(nodes),
            path: vec![
                Proj::Deref,
                Proj::Index(idx),
                Proj::Field { field: 0, ty: U32 },
            ],
            mutable: false,
        },
        REF_U32,
        NO_ORIGIN,
    );
    let prev = b.op(entry, OpKind::Deref(prev_ref), U32, NO_ORIGIN);
    let next_z_ref = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Value(nodes),
            path: vec![
                Proj::Deref,
                Proj::Index(idx),
                Proj::Field { field: 1, ty: U32 },
            ],
            mutable: false,
        },
        REF_U32,
        NO_ORIGIN,
    );
    let next_z = b.op(entry, OpKind::Deref(next_z_ref), U32, NO_ORIGIN);

    // 1. AlwaysValid field `prev` -> eliminated without guard.
    bounds_assert(&mut b, entry, prev, len);
    // 2. Adversarial: ValidOrSentinel field `next_z` WITHOUT `!= u32::MAX` guard -> MUST STAY!
    bounds_assert(&mut b, entry, next_z, len);

    let sent = constant(&mut b, entry, u32::MAX as u128);
    let ne = compare(&mut b, entry, BinaryOp::Ne, next_z, sent);
    let guarded_blk = b.block();
    let exit_blk = b.block();
    b.terminate(
        entry,
        RTerm::Switch {
            scrut: ne,
            scrut_ty: U32,
            cases: vec![(0, exit_blk, Vec::new())],
            default: (guarded_blk, Vec::new()),
        },
    );
    // 3. ValidOrSentinel field `next_z` WITH dominating `!= u32::MAX` guard -> eliminated!
    bounds_assert(&mut b, guarded_blk, next_z, len);
    let one = constant(&mut b, guarded_blk, 1);
    b.terminate(guarded_blk, RTerm::Return(Some(one)));
    let zero = constant(&mut b, exit_blk, 0);
    b.terminate(exit_blk, RTerm::Return(Some(zero)));

    let mut func = finished(b);
    let mut witnesses = Vec::new();
    let res = run_with_facts(&mut func, &tbl, &facts, &mut witnesses);
    assert_eq!(
        res.asserts_removed, 2,
        "AlwaysValid and guarded ValidOrSentinel removed; unguarded ValidOrSentinel stays"
    );
    assert_eq!(asserts(&func), 1);
    super::super::witness::check_all_with_context(&witnesses, &func, &tbl, Some(&facts))
        .expect("MonotoneArena witnesses must verify");
}

/// The corpus property test, exactly [`super::dce`]'s: every fixture function the pass changes
/// still verifies and still means the same thing under the interpreter — and a wrongly removed
/// bounds check is exactly the kind of change it sees, as a `Trap::Assert` on one side only.
#[cfg(test)]
mod corpus {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use rcgjs_object::object;

    use crate::jir::interns::StrId;
    use crate::jir::interp::Value;
    use crate::jir::repr::Func;
    use crate::jir::semantic::{BodyKind, TypeId};
    use crate::jir::types::TypeTable;
    use crate::lower::ssa;

    const SETS: usize = 4;
    const STEPS: u64 = 5_000;

    #[derive(Default)]
    struct Counts {
        functions: usize,
        changed: usize,
        removed: u32,
        agreed: usize,
        skipped: usize,
        verify_failures: Vec<String>,
        mismatches: Vec<String>,
    }

    #[test]
    fn range_preserves_fixture_semantics() {
        std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn(run_corpus)
            .expect("spawn")
            .join()
            .expect("fixture thread");
    }

    fn run_corpus() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build");
        let mut paths = Vec::new();
        walk(&root, &mut paths);
        let mut counts = Counts::default();
        for path in &paths {
            check_object(&path.display().to_string(), path, &mut counts);
        }
        eprintln!(
            "range property: {} functions, {} changed, {} asserts removed, {} argument sets \
             agreed, {} skipped, {} verify failures, {} mismatches",
            counts.functions,
            counts.changed,
            counts.removed,
            counts.agreed,
            counts.skipped,
            counts.verify_failures.len(),
            counts.mismatches.len(),
        );
        for failure in counts.verify_failures.iter().take(10) {
            eprintln!("  VERIFY   {failure}");
        }
        for mismatch in counts.mismatches.iter().take(10) {
            eprintln!("  MISMATCH {mismatch}");
        }
        ssa::check::require_fixtures(counts.functions, "range property");
        assert!(counts.verify_failures.is_empty(), "verify failures");
        assert!(counts.mismatches.is_empty(), "functions changed meaning");
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            if path.is_dir() {
                if matches!(
                    name.as_deref(),
                    Some("hostmacro-discover" | "sysroot-cache" | "sysroots" | "target")
                ) {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "o") {
                out.push(path);
            }
        }
    }

    fn check_object(label: &str, path: &Path, counts: &mut Counts) {
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        if !object::is_object(&bytes) {
            return;
        }
        let Ok(Some(object)) = object::read(&bytes) else {
            return;
        };
        let table = &object.interns.types;
        let interns = &object.interns;
        let mut decoded = Vec::new();
        let mut results: BTreeMap<StrId, TypeId> = BTreeMap::new();
        for item in &object.summaries {
            let Ok(Some((body, origins))) = object.decode_verified(item) else {
                continue;
            };
            if !matches!(body.kind, BodyKind::Function) {
                continue;
            }
            if let Some(index) = interns
                .strings
                .iter()
                .position(|string| string == &item.key.link_name)
            {
                results.insert(StrId(index as u32), body.signature.result);
            }
            decoded.push((item.key.link_name.clone(), body, origins));
        }
        for (name, body, origins) in decoded {
            let Ok(before) = ssa::build(&body, Some(&origins), table, interns) else {
                continue;
            };
            counts.functions += 1;
            let mut after = before.clone();
            let range = super::run(&mut after, table, &mut Vec::new());
            if !range.changed() {
                continue;
            }
            counts.changed += 1;
            counts.removed += range.asserts_removed + range.comparisons_folded;
            if let Err(error) = after.verify(table) {
                counts
                    .verify_failures
                    .push(format!("{label} {name}: {error}"));
                continue;
            }
            let Some(sets) = ssa::check::argument_sets(
                &before
                    .params
                    .iter()
                    .map(|param| before.values[param.0 as usize].ty)
                    .collect::<Vec<_>>(),
                table,
            ) else {
                continue;
            };
            for set in sets.iter().take(SETS) {
                match compare(&before, &after, table, interns, &results, set) {
                    None => counts.agreed += 1,
                    Some(None) => counts.skipped += 1,
                    Some(Some(why)) => counts.mismatches.push(format!("{label} {name}: {why}")),
                }
            }
        }
    }

    #[allow(clippy::option_option)]
    fn compare(
        before: &Func,
        after: &Func,
        table: &TypeTable,
        interns: &crate::jir::interns::Interns,
        results: &BTreeMap<StrId, TypeId>,
        set: &[(Value, Option<Value>)],
    ) -> Option<Option<String>> {
        match ssa::check::compare_funcs(before, after, table, interns, results, set, STEPS) {
            ssa::check::Verdict::Agree => None,
            ssa::check::Verdict::Skipped(_) => Some(None),
            ssa::check::Verdict::Mismatch(why) => Some(Some(why)),
        }
    }
}

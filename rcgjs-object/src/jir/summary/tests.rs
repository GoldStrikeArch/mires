mod identity_tests {
    use super::super::*;
    use crate::jir::codec;
    use crate::jir::hash::Hash128;
    use crate::jir::interns::{Interns, StrId};
    use crate::jir::semantic::{
        AbiSignature, BasicBlock, BinaryOp, BlockId, Body, BodyKind, BodySemantic, Call,
        CallSemantic, Callee, Constant, DefinitionId, ItemId, Local, LocalId, LocalRole, Operand,
        OperandKind, Place, Rvalue, Signature, Statement, StatementKind, Terminator, Unwind,
    };
    use crate::jir::types::TypeId;
    use crate::object::{BlobRef, ItemKind, Linkage};

    const I32: TypeId = TypeId(3);

    fn item(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: n,
            },
            instantiation: u128::from(n),
        }
    }

    fn place(local: u32) -> Place {
        Place {
            local: LocalId(local),
            projection: Vec::new(),
            ty: I32,
        }
    }

    fn copy(local: u32) -> Operand {
        Operand {
            ty: I32,
            kind: OperandKind::Copy(place(local)),
        }
    }

    fn scalar(bits: u128) -> Operand {
        Operand {
            ty: I32,
            kind: OperandKind::Constant(Constant::Scalar { bits, bytes: 4 }),
        }
    }

    fn assign(destination: u32, left: u32, right: u128) -> Statement {
        Statement {
            kind: StatementKind::Assign {
                destination: place(destination),
                value: Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: copy(left),
                    right: scalar(right),
                },
            },
        }
    }

    fn locals() -> Vec<Local> {
        vec![
            Local {
                id: LocalId(0),
                role: LocalRole::Return,
                ty: I32,
            },
            Local {
                id: LocalId(1),
                role: LocalRole::Argument,
                ty: I32,
            },
            Local {
                id: LocalId(2),
                role: LocalRole::Temporary,
                ty: I32,
            },
        ]
    }

    fn body(blocks: Vec<BasicBlock>) -> Body {
        Body {
            item: item(1),
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![I32],
                result: I32,
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    ..AbiSignature::default()
                },
            },
            locals: locals(),
            blocks,
        }
    }

    /// `fn f(a) { t = a + 1; goto exit; exit: ret = t + 0; return }`, blocks numbered 0 and 1.
    fn plain() -> Body {
        body(vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: vec![assign(2, 1, 1)],
                terminator: Terminator::Goto(BlockId(1)),
            },
            BasicBlock {
                id: BlockId(1),
                cleanup: false,
                statements: vec![assign(0, 2, 0)],
                terminator: Terminator::Return,
            },
        ])
    }

    /// The same function: blocks numbered 7 and 3, storage markers and nops between the
    /// statements, the goto spelled as a false edge, one unreachable block left behind, and one
    /// local (5) that only the storage markers mention.
    fn noisy() -> Body {
        let mut noisy = body(vec![
            BasicBlock {
                id: BlockId(7),
                cleanup: false,
                statements: vec![
                    Statement {
                        kind: StatementKind::StorageLive(LocalId(2)),
                    },
                    Statement {
                        kind: StatementKind::StorageLive(LocalId(5)),
                    },
                    Statement {
                        kind: StatementKind::Nop,
                    },
                    assign(2, 1, 1),
                    Statement {
                        kind: StatementKind::StorageDead(LocalId(5)),
                    },
                ],
                terminator: Terminator::FalseEdge {
                    real: BlockId(3),
                    imaginary: BlockId(9),
                },
            },
            BasicBlock {
                id: BlockId(9),
                cleanup: false,
                statements: vec![assign(0, 1, 77)],
                terminator: Terminator::Unreachable,
            },
            BasicBlock {
                id: BlockId(3),
                cleanup: false,
                statements: vec![
                    Statement {
                        kind: StatementKind::Nop,
                    },
                    assign(0, 2, 0),
                    Statement {
                        kind: StatementKind::StorageDead(LocalId(2)),
                    },
                ],
                terminator: Terminator::Return,
            },
        ]);
        noisy.locals.push(Local {
            id: LocalId(5),
            role: LocalRole::Temporary,
            ty: I32,
        });
        noisy
    }

    #[test]
    fn storage_markers_nops_false_edges_dead_blocks_and_numbering_do_not_change_the_hash() {
        let interns = Interns::new();
        let plain = plain();
        let noisy = noisy();
        assert_ne!(
            plain, noisy,
            "the two bodies must differ, or the test proves nothing"
        );
        assert_ne!(
            Hash128::of(&codec::encode_body(&plain)),
            Hash128::of(&codec::encode_body(&noisy)),
            "the blob hash is a hash of bytes and must see the difference"
        );
        assert_eq!(
            semantic_hash(&plain, &interns),
            semantic_hash(&noisy, &interns),
            "the normalized hash must not"
        );
    }

    #[test]
    fn physical_summaries_are_canonical_over_the_normalized_body() {
        let interns = Interns::new();
        let plain = plain();
        let noisy = noisy();
        let hash = semantic_hash(&plain, &interns);
        assert_eq!(hash, semantic_hash(&noisy, &interns));
        assert_eq!(
            PhysicalSummary::of(&plain, &interns.types, hash),
            PhysicalSummary::of(&noisy, &interns.types, hash)
        );
    }

    #[test]
    fn stale_and_overclaiming_physical_summaries_are_rejected() {
        let interns = Interns::new();
        let body = plain();
        let hash = semantic_hash(&body, &interns);
        let summary = PhysicalSummary::of(&body, &interns.types, hash).expect("a function");
        assert_eq!(summary.validate(&body, &interns.types, hash), Ok(()));

        let mut stale = summary.clone();
        stale.format += 1;
        assert_eq!(
            stale.validate(&body, &interns.types, hash),
            Err(PhysicalSummaryValidation::Format)
        );

        let mut overclaim = summary.clone();
        overclaim.parameters[0].field_reads += 1;
        assert_eq!(
            overclaim.validate(&body, &interns.types, hash),
            Err(PhysicalSummaryValidation::Facts)
        );

        assert_eq!(
            summary.validate(&body, &interns.types, Hash128::of(b"another body")),
            Err(PhysicalSummaryValidation::Body)
        );
    }

    #[test]
    fn the_normal_form_drops_what_it_says_it_drops() {
        let normalized = NormalizedBody::of(&noisy(), &Interns::new());
        assert_eq!(normalized.body.blocks.len(), 2, "the dead block is gone");
        assert_eq!(
            normalized
                .body
                .blocks
                .iter()
                .map(|block| block.id)
                .collect::<Vec<_>>(),
            vec![BlockId(0), BlockId(1)],
            "blocks are renumbered from the entry"
        );
        assert!(
            normalized
                .body
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .all(|statement| matches!(statement.kind, StatementKind::Assign { .. })),
            "storage markers and nops are gone"
        );
        assert!(
            matches!(
                normalized.body.blocks[0].terminator,
                Terminator::Goto(BlockId(1))
            ),
            "the false edge became the edge it really is"
        );
        assert_eq!(
            normalized.body.locals.len(),
            3,
            "the local only the storage markers mentioned is gone"
        );
    }

    #[test]
    fn a_changed_operand_changes_the_hash() {
        let interns = Interns::new();
        let mut changed = plain();
        changed.blocks[0].statements[0] = assign(2, 1, 2);
        assert_ne!(
            semantic_hash(&plain(), &interns),
            semantic_hash(&changed, &interns)
        );
    }

    #[test]
    fn a_changed_terminator_changes_the_hash() {
        let interns = Interns::new();
        let mut changed = plain();
        changed.blocks[1].terminator = Terminator::Unreachable;
        assert_ne!(
            semantic_hash(&plain(), &interns),
            semantic_hash(&changed, &interns)
        );
    }

    /// Two codegen units intern their strings in whatever order they meet them; the hash must not
    /// see the ids, only the text.
    #[test]
    fn interned_strings_hash_by_content_not_by_id() {
        let call = |name: StrId| BasicBlock {
            id: BlockId(0),
            cleanup: false,
            statements: Vec::new(),
            terminator: Terminator::Call(Call {
                function: scalar(0),
                callee: Callee::Direct {
                    item: item(2),
                    link_name: name,
                },
                semantic: CallSemantic::Ordinary,
                arguments: vec![copy(1)],
                destination: place(0),
                target: None,
                unwind: Unwind::Continue,
                track_caller: None,
                host: None,
            }),
        };

        let mut left = Interns::new();
        let left_name = left.intern_str("callee$h1");
        let mut right = Interns::new();
        // A different id for the same text, and the same id for different text.
        right.intern_str("something$else");
        right.intern_str("and$another");
        let right_name = right.intern_str("callee$h1");
        assert_ne!(left_name, right_name);

        assert_eq!(
            semantic_hash(&body(vec![call(left_name)]), &left),
            semantic_hash(&body(vec![call(right_name)]), &right)
        );
        assert_ne!(
            semantic_hash(&body(vec![call(left_name)]), &left),
            semantic_hash(&body(vec![call(StrId(0))]), &right),
            "different text must still differ"
        );
    }

    // -- the identity rule --

    fn summary(name: &str, item: Option<ItemId>, hash: Option<Hash128>) -> ItemSummary {
        ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
                item,
            },
            body_hash: hash,
            semantic_hash: hash,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef::default(),
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::empty(),
        }
    }

    #[test]
    fn equal_normalized_hashes_merge() {
        let hash = Hash128::of(b"same");
        let left = summary("f", Some(item(1)), Some(hash));
        let right = summary("f", Some(item(1)), Some(hash));
        assert_eq!(merge_duplicates(&left, &right), MergeDecision::Merge);
    }

    #[test]
    fn different_normalized_hashes_pick_the_smaller_one_either_way_round() {
        let low = Hash128 { lo: 1, hi: 0 };
        let high = Hash128 { lo: 2, hi: 0 };
        let left = summary("f", Some(item(1)), Some(low));
        let right = summary("f", Some(item(1)), Some(high));
        let forward = merge_duplicates(&left, &right);
        let backward = merge_duplicates(&right, &left);
        match (&forward, &backward) {
            (
                MergeDecision::Choose {
                    keep: Side::Left,
                    chosen,
                    rejected,
                    decline,
                },
                MergeDecision::Choose {
                    keep: Side::Right,
                    chosen: back_chosen,
                    rejected: back_rejected,
                    decline: back_decline,
                },
            ) => {
                assert_eq!(*chosen, low);
                assert_eq!(*rejected, high);
                assert_eq!(chosen, back_chosen);
                assert_eq!(rejected, back_rejected);
                assert_eq!(decline.code(), "merge.body-choice");
                assert_eq!(decline, back_decline);
            }
            other => panic!("expected a deterministic choice, got {other:?}"),
        }
    }

    /// A copy with cleanup edges is kept over a copy without them whatever their hashes, for
    /// duplicates and aliases alike, and either way round. Otherwise the smaller hash wins.
    #[test]
    fn a_copy_with_cleanup_edges_beats_a_smaller_hash_without_them() {
        let low = Hash128 { lo: 1, hi: 0 };
        let high = Hash128 { lo: 2, hi: 0 };
        let copy = |hash, cleanup| {
            let mut copy = summary("f", Some(item(1)), Some(hash));
            if cleanup {
                copy.class.insert(ItemClass::UNWIND);
            }
            copy
        };
        for merge in [merge_duplicates, merge_aliases] {
            for (low_cleanup, high_cleanup, expected) in [
                (false, true, high),
                (true, false, low),
                (true, true, low),
                (false, false, low),
            ] {
                let (left, right) = (copy(low, low_cleanup), copy(high, high_cleanup));
                for (a, b, keep) in [
                    (
                        &left,
                        &right,
                        if expected == low {
                            Side::Left
                        } else {
                            Side::Right
                        },
                    ),
                    (
                        &right,
                        &left,
                        if expected == low {
                            Side::Right
                        } else {
                            Side::Left
                        },
                    ),
                ] {
                    match merge(a, b) {
                        MergeDecision::Choose {
                            keep: kept, chosen, ..
                        } => {
                            assert_eq!(kept, keep);
                            assert_eq!(chosen, expected);
                        }
                        other => panic!("expected a choice, got {other:?}"),
                    }
                }
            }
        }
    }

    #[test]
    fn a_body_on_one_side_only_keeps_that_side_without_a_decline() {
        let hash = Hash128::of(b"body");
        let left = summary("f", Some(item(1)), Some(hash));
        let right = summary("f", Some(item(1)), None);
        assert_eq!(
            merge_duplicates(&left, &right),
            MergeDecision::Available(Side::Left)
        );
        assert_eq!(
            merge_duplicates(&right, &left),
            MergeDecision::Available(Side::Right)
        );
        let neither = summary("f", Some(item(1)), None);
        assert_eq!(merge_duplicates(&right, &neither), MergeDecision::Merge);
    }

    #[test]
    fn one_name_over_two_items_is_an_error_not_a_degrade() {
        let left = summary("f", Some(item(1)), Some(Hash128::of(b"a")));
        let right = summary("f", Some(item(2)), Some(Hash128::of(b"a")));
        match merge_duplicates(&left, &right) {
            MergeDecision::Error(MergeError::NameCollision { link_name, .. }) => {
                assert_eq!(link_name, "f")
            }
            other => panic!("expected a name collision, got {other:?}"),
        }
        let unrelated = summary("g", Some(item(1)), None);
        assert!(matches!(
            merge_duplicates(&left, &unrelated),
            MergeDecision::Error(MergeError::DifferentNames { .. })
        ));
    }

    /// The writer fills `semantic_hash`, so two objects written from bodies that differ only in
    /// what normalization drops deduplicate straight out of their summary indexes, without either
    /// body being inflated.
    #[test]
    fn two_objects_written_from_noisy_and_plain_bodies_merge_from_their_indexes_alone() {
        use crate::jir::origin::OriginTable;
        use crate::object::{self, ItemInput, Linkage as ObjectLinkage};

        let origins = |body: &Body| OriginTable {
            origins: Vec::new(),
            statements: vec![crate::jir::origin::OriginId(0); body.statement_count()],
            terminators: vec![crate::jir::origin::OriginId(0); body.blocks.len()],
            locals: vec![crate::jir::origin::OriginId(0); body.locals.len()],
            item_path: StrId(0),
            root: None,
        };
        let write = |body: &Body, crate_name: &str| {
            let mut interns = Interns::new();
            interns.intern_str("path::to::f");
            let mut writer = object::Writer::new(object::test_header(crate_name), "", interns);
            writer.push_item(
                "function f(a){return a}",
                ItemInput {
                    key: SemanticKey {
                        link_name: "f".to_owned(),
                        item: Some(item(1)),
                    },
                    availability: Availability::Available,
                    deps: Vec::new(),
                    kind: ItemKind::Fn,
                    linkage: ObjectLinkage::internal(),
                    order: None,
                    dts: false,
                    body: Some((body, &origins(body))),
                },
            );
            writer.finish(&[], &std::collections::BTreeMap::new())
        };

        let left = write(&plain(), "left");
        let right = write(&noisy(), "right");
        let left = object::read(&left).expect("read").expect("an object");
        let right = object::read(&right).expect("read").expect("an object");
        assert_eq!(merge_headers(&left.header, &right.header), Ok(()));
        assert_ne!(
            left.summaries[0].body_hash, right.summaries[0].body_hash,
            "the blobs differ"
        );
        assert_eq!(
            left.summaries[0].physical, right.summaries[0].physical,
            "normalization noise cannot change producer facts"
        );
        assert_eq!(
            merge_duplicates(&left.summaries[0], &right.summaries[0]),
            MergeDecision::Merge,
            "and the items are the same item"
        );
    }

    #[test]
    fn headers_have_to_agree_field_by_field() {
        let base = crate::object::test_header("a");
        assert_eq!(merge_headers(&base, &base), Ok(()));
        for change in [
            (|header: &mut crate::object::Header| header.jir_schema += 1) as fn(&mut _),
            |header: &mut crate::object::Header| header.rustc_version_hash ^= 1,
            |header: &mut crate::object::Header| header.data_layout.push('x'),
            |header: &mut crate::object::Header| header.emit_args.push_str("js-minify=on"),
        ] {
            let mut other = base.clone();
            change(&mut other);
            let decline = merge_headers(&base, &other).expect_err("must not agree");
            assert_eq!(decline.code(), "merge.header");
        }
        // The crate name is not one of them: two crates contribute items to one program.
        let mut other = base.clone();
        other.crate_name = "b".to_owned();
        assert_eq!(merge_headers(&base, &other), Ok(()));
    }
}

mod alias_tests {
    use super::super::*;
    use crate::jir::hash::Hash128;
    use crate::jir::semantic::{DefinitionId, ItemId};
    use crate::object::{BlobRef, ItemKind, Linkage};

    fn summary(name: &str, local_hash: u64, hash: Option<Hash128>) -> ItemSummary {
        ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
                item: Some(ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash,
                    },
                    instantiation: u128::from(local_hash),
                }),
            },
            body_hash: hash,
            semantic_hash: hash,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef::default(),
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::empty(),
        }
    }

    /// The 74-conflict shape: two different items with the same printed text. v1 declined the
    /// winner; the rule here picks a body and stays available.
    #[test]
    fn two_different_items_with_one_text_pick_a_body_instead_of_declining() {
        let winner = summary("a$h1", 1, Some(Hash128 { lo: 9, hi: 0 }));
        let loser = summary("a$h2", 2, Some(Hash128 { lo: 4, hi: 0 }));
        match merge_aliases(&winner, &loser) {
            MergeDecision::Choose {
                keep: Side::Right,
                chosen,
                rejected,
                decline,
            } => {
                assert_eq!(chosen, Hash128 { lo: 4, hi: 0 });
                assert_eq!(rejected, Hash128 { lo: 9, hi: 0 });
                assert_eq!(decline.code(), "merge.body-choice");
            }
            other => panic!("expected a body choice, got {other:?}"),
        }
        // The same pair merged as duplicates would be a hard error: they are not one item.
        assert!(matches!(
            merge_duplicates(&winner, &loser),
            MergeDecision::Error(MergeError::DifferentNames { .. })
        ));
    }

    #[test]
    fn identical_bodies_behind_two_names_just_merge() {
        let hash = Some(Hash128::of(b"one body"));
        assert_eq!(
            merge_aliases(&summary("a$h1", 1, hash), &summary("a$h2", 2, hash)),
            MergeDecision::Merge
        );
    }
}

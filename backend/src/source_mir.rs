//! Session-local owned MIR snapshots using rustc's own semantic codec.
//!
//! Tokens retain no TyCtxt-borrowed values. Spans, definition and allocation IDs belong to the
//! current compiler session, so this is not an object format or an incremental cache.

use rustc_data_structures::fx::FxHashMap;
use rustc_hir::def_id::{CrateNum, DefId, DefIndex};
use rustc_middle::mir::{interpret::AllocId, Body};
use rustc_middle::ty::{
    self,
    codec::{TyDecoder, TyEncoder},
    print::with_no_trimmed_paths,
    Ty, TyCtxt,
};
use rustc_serialize::{Decodable, Decoder, Encodable, Encoder};
use rustc_span::{
    AttrId, BlobDecoder, ByteSymbol, ExpnId, Span, SpanDecoder, SpanEncoder, Symbol, SyntaxContext,
};

pub(crate) struct SourceMir {
    tokens: Vec<Token>,
    original: Option<String>,
}

impl SourceMir {
    pub(crate) fn capture<'tcx>(body: &Body<'tcx>) -> Self {
        let mut encoder = Capture::<false> {
            tokens: Vec::new(),
            types: FxHashMap::default(),
            predicates: FxHashMap::default(),
        };
        body.encode(&mut encoder);
        Self {
            tokens: encoder.tokens,
            original: crate::jir::encode::self_check().then(|| Self::debug_body(body)),
        }
    }

    // Rustc intentionally omits recomputable CFG caches from its MIR codec. Compare every
    // remaining debug field as an independent check, including data excluded from stable hashes.
    fn debug_body(body: &Body<'_>) -> String {
        let mut body = body.clone();
        body.basic_blocks.invalidate_cfg_cache();
        with_no_trimmed_paths!(format!("{body:#?}"))
    }

    pub(crate) fn body<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Body<'tcx> {
        let mut decoder = Restore::<false> {
            tokens: &self.tokens,
            position: 0,
            tcx,
            allocations: None,
        };
        let body = Body::decode(&mut decoder);
        assert_eq!(
            decoder.position,
            self.tokens.len(),
            "source MIR decoder left tokens"
        );
        if let Some(original) = &self.original {
            assert_eq!(
                *original,
                Self::debug_body(&body),
                "source MIR codec changed the body"
            );
            assert_eq!(
                self.tokens,
                Self::capture(&body).tokens,
                "source MIR codec changed semantic tokens"
            );
            eprintln!(
                "source-mir-selfcheck: {} {} {}",
                if body.coroutine.is_some() {
                    "coroutine"
                } else {
                    "ordinary"
                },
                body.basic_blocks.len(),
                body.local_decls.len(),
            );
        }
        body
    }
}

macro_rules! numbers {
    ($callback:ident) => {
        $callback! {
            (Usize, usize, emit_usize, read_usize),
            (U128, u128, emit_u128, read_u128),
            (U64, u64, emit_u64, read_u64),
            (U32, u32, emit_u32, read_u32),
            (U16, u16, emit_u16, read_u16),
            (U8, u8, emit_u8, read_u8),
            (Isize, isize, emit_isize, read_isize),
            (I128, i128, emit_i128, read_i128),
            (I64, i64, emit_i64, read_i64),
            (I32, i32, emit_i32, read_i32),
            (I16, i16, emit_i16, read_i16)
        }
    };
}
macro_rules! tokens {
    ($(($variant:ident, $ty:ty, $emit:ident, $read:ident)),*) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        enum Token {
            $($variant($ty),)*
            Text(String), Bytes(Vec<u8>), Span(Span), Symbol(Symbol), ByteSymbol(ByteSymbol),
            Expansion(ExpnId), Context(SyntaxContext), Crate(CrateNum), Index(DefIndex),
            Definition(DefId), Allocation(AllocId),
        }
    };
}
numbers!(tokens);

struct Capture<'tcx, const CLEAR: bool> {
    tokens: Vec<Token>,
    types: FxHashMap<Ty<'tcx>, usize>,
    predicates: FxHashMap<ty::PredicateKind<'tcx>, usize>,
}

macro_rules! emit_numbers {
    ($(($variant:ident, $ty:ty, $emit:ident, $read:ident)),*) => {
        $(fn $emit(&mut self, value: $ty) { self.tokens.push(Token::$variant(value)); })*
    };
}
impl<const CLEAR: bool> Encoder for Capture<'_, CLEAR> {
    numbers!(emit_numbers);
    fn emit_str(&mut self, value: &str) {
        self.tokens.push(Token::Text(value.to_owned()));
    }
    fn emit_byte_str(&mut self, value: &[u8]) {
        self.emit_raw_bytes(value);
    }
    fn emit_raw_bytes(&mut self, value: &[u8]) {
        self.tokens.push(Token::Bytes(value.to_vec()));
    }
}

impl<const CLEAR: bool> SpanEncoder for Capture<'_, CLEAR> {
    fn encode_span(&mut self, value: Span) {
        self.tokens.push(Token::Span(value));
    }
    fn encode_symbol(&mut self, value: Symbol) {
        self.tokens.push(Token::Symbol(value));
    }
    fn encode_byte_symbol(&mut self, value: ByteSymbol) {
        self.tokens.push(Token::ByteSymbol(value));
    }
    fn encode_expn_id(&mut self, value: ExpnId) {
        self.tokens.push(Token::Expansion(value));
    }
    fn encode_syntax_context(&mut self, value: SyntaxContext) {
        self.tokens.push(Token::Context(value));
    }
    fn encode_crate_num(&mut self, value: CrateNum) {
        self.tokens.push(Token::Crate(value));
    }
    fn encode_def_index(&mut self, value: DefIndex) {
        self.tokens.push(Token::Index(value));
    }
    fn encode_def_id(&mut self, value: DefId) {
        self.tokens.push(Token::Definition(value));
    }
}

impl<'tcx, const CLEAR: bool> TyEncoder<'tcx> for Capture<'tcx, CLEAR> {
    const CLEAR_CROSS_CRATE: bool = CLEAR;
    fn position(&self) -> usize {
        self.tokens.len()
    }
    // Token positions are not byte offsets. Expand every type rather than emitting byte shorthands.
    fn type_shorthands(&mut self) -> &mut FxHashMap<Ty<'tcx>, usize> {
        self.types.clear();
        &mut self.types
    }
    fn predicate_shorthands(&mut self) -> &mut FxHashMap<ty::PredicateKind<'tcx>, usize> {
        self.predicates.clear();
        &mut self.predicates
    }
    fn encode_alloc_id(&mut self, value: &AllocId) {
        self.tokens.push(Token::Allocation(*value));
    }
}

struct Restore<'a, 'tcx, const CLEAR: bool = false> {
    tokens: &'a [Token],
    position: usize,
    tcx: TyCtxt<'tcx>,
    allocations: Option<&'a rustc_middle::mir::interpret::AllocDecodingState>,
}
impl<const CLEAR: bool> Restore<'_, '_, CLEAR> {
    fn take(&mut self) -> &Token {
        let token = &self.tokens[self.position];
        self.position += 1;
        token
    }
}
macro_rules! read_numbers {
    ($(($variant:ident, $ty:ty, $emit:ident, $read:ident)),*) => {
        $(fn $read(&mut self) -> $ty {
            let Token::$variant(value) = self.take() else { panic!("source MIR token kind mismatch"); };
            *value
        })*
    };
}
impl<const CLEAR: bool> Decoder for Restore<'_, '_, CLEAR> {
    numbers!(read_numbers);
    fn read_str(&mut self) -> &str {
        let Token::Text(value) = self.take() else {
            panic!("source MIR text expected");
        };
        value
    }
    fn read_byte_str(&mut self) -> &[u8] {
        let Token::Bytes(value) = self.take() else {
            panic!("source MIR bytes expected");
        };
        value
    }
    fn read_raw_bytes(&mut self, len: usize) -> &[u8] {
        let value = self.read_byte_str();
        assert_eq!(value.len(), len);
        value
    }
    fn peek_byte(&self) -> u8 {
        let Token::U8(value) = self.tokens[self.position] else {
            panic!("source MIR byte expected");
        };
        value
    }
    fn position(&self) -> usize {
        self.position
    }
}
impl<const CLEAR: bool> BlobDecoder for Restore<'_, '_, CLEAR> {
    fn decode_symbol(&mut self) -> Symbol {
        let Token::Symbol(value) = self.take() else {
            panic!("source MIR symbol expected");
        };
        *value
    }
    fn decode_byte_symbol(&mut self) -> ByteSymbol {
        let Token::ByteSymbol(value) = self.take() else {
            panic!("source MIR byte symbol expected");
        };
        *value
    }
    fn decode_def_index(&mut self) -> DefIndex {
        let Token::Index(value) = self.take() else {
            panic!("source MIR definition index expected");
        };
        *value
    }
}
impl<const CLEAR: bool> SpanDecoder for Restore<'_, '_, CLEAR> {
    fn decode_span(&mut self) -> Span {
        let Token::Span(value) = self.take() else {
            panic!("source MIR span expected");
        };
        *value
    }
    fn decode_expn_id(&mut self) -> ExpnId {
        let Token::Expansion(value) = self.take() else {
            panic!("source MIR expansion expected");
        };
        *value
    }
    fn decode_syntax_context(&mut self) -> SyntaxContext {
        let Token::Context(value) = self.take() else {
            panic!("source MIR syntax context expected");
        };
        *value
    }
    fn decode_crate_num(&mut self) -> CrateNum {
        let Token::Crate(value) = self.take() else {
            panic!("source MIR crate expected");
        };
        *value
    }
    fn decode_def_id(&mut self) -> DefId {
        let Token::Definition(value) = self.take() else {
            panic!("source MIR definition expected");
        };
        *value
    }
    fn decode_attr_id(&mut self) -> AttrId {
        panic!("source MIR contains an unsupported attribute ID")
    }
}
impl<'tcx, const CLEAR: bool> rustc_type_ir::InternerDecoder for Restore<'_, 'tcx, CLEAR> {
    type Interner = TyCtxt<'tcx>;
    fn interner(&self) -> TyCtxt<'tcx> {
        self.tcx
    }
}
impl<'tcx, const CLEAR: bool> TyDecoder<'tcx> for Restore<'_, 'tcx, CLEAR> {
    const CLEAR_CROSS_CRATE: bool = CLEAR;
    fn positioned_at_shorthand(&self) -> bool {
        false
    }
    fn cached_ty_for_shorthand<F>(&mut self, _: usize, _: F) -> Ty<'tcx>
    where
        F: FnOnce(&mut Self) -> Ty<'tcx>,
    {
        panic!("source MIR type shorthand is disabled")
    }
    fn with_position<F, R>(&mut self, pos: usize, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let old = self.position;
        self.position = pos;
        let result = f(self);
        self.position = old;
        result
    }
    fn decode_alloc_id(&mut self) -> AllocId {
        if let Some(allocations) = self.allocations {
            return allocations.new_decoding_session().decode_alloc_id(self);
        }
        let Token::Allocation(value) = self.take() else {
            panic!("source MIR allocation expected");
        };
        *value
    }
}

mod portable;
pub(crate) use portable::PortableMir;

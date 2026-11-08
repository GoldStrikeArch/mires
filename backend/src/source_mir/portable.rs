//! Owned MIR transport through stable definition, source-file, allocation and hygiene identities.

use super::*;
use rustc_hir::def_id::{DefPathHash, StableCrateId, LOCAL_CRATE};
use rustc_middle::mir::interpret::{specialized_encode_alloc_id, AllocDecodingState};
use rustc_span::hygiene::{self, ExpnData, ExpnHash, ExpnIndex, Transparency};
use rustc_span::{BytePos, StableSourceFileId};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const ROOT_EXPANSION: u32 = u32::MAX;

#[derive(Serialize, Deserialize)]
pub(crate) struct PortableMir {
    version: u32,
    bodies: Vec<Vec<PortableToken>>,
    allocations: Vec<Vec<PortableToken>>,
    expansions: Vec<Expansion>,
    #[serde(skip)]
    restored: std::sync::OnceLock<std::sync::Mutex<Restored>>,
}

impl PortableMir {
    pub(crate) fn capture_bodies<'a, 'tcx: 'a>(
        tcx: TyCtxt<'tcx>,
        bodies: impl IntoIterator<Item = &'a Body<'tcx>>,
    ) -> Self {
        let mut builder = Builder {
            tcx,
            alloc_ids: FxHashMap::default(),
            alloc_queue: Vec::new(),
            expn_ids: FxHashMap::default(),
            expn_queue: Vec::new(),
        };
        let bodies = bodies
            .into_iter()
            .map(|body| builder.tokens(capture(body)))
            .collect();
        let mut allocations = Vec::new();
        let mut expansions = Vec::new();
        loop {
            if allocations.len() < builder.alloc_queue.len() {
                let id = builder.alloc_queue[allocations.len()];
                let mut encoder = encoder();
                specialized_encode_alloc_id(&mut encoder, tcx, id);
                allocations.push(builder.tokens(encoder.tokens));
            } else if expansions.len() < builder.expn_queue.len() {
                let id = builder.expn_queue[expansions.len()];
                let data = builder.tokens(capture(&id.expn_data()));
                expansions.push(Expansion {
                    krate: scalar(&tcx.stable_crate_id(id.krate)),
                    index: id.local_id.as_u32(),
                    hash: scalar(&id.expn_hash()),
                    data,
                });
            } else {
                break;
            }
        }
        Self {
            version: VERSION,
            bodies,
            allocations,
            expansions,
            restored: std::sync::OnceLock::new(),
        }
    }

    pub(crate) fn roundtrip<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> Body<'tcx> {
        let portable = Self::capture_bodies(tcx, [body, body]);
        let bytes = postcard::to_stdvec(&portable).expect("encode portable MIR");
        let portable: Self = postcard::from_bytes(&bytes).expect("decode portable MIR");
        let restored = portable.body(tcx);
        assert_eq!(
            capture(&restored),
            capture(&portable.body_at(tcx, 1)),
            "portable MIR changed allocation identity across bodies"
        );
        assert_eq!(
            capture(&restored),
            capture(&portable.body(tcx)),
            "portable MIR changed allocation identity on repeated decoding"
        );
        assert_eq!(
            bytes,
            postcard::to_stdvec(&Self::capture_bodies(tcx, [&restored, &restored])).unwrap(),
            "portable MIR changed semantic tokens"
        );
        eprintln!("portable-mir-selfcheck: {} bytes", bytes.len());
        restored
    }

    pub(crate) fn body<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Body<'tcx> {
        self.body_at(tcx, 0)
    }

    pub(crate) fn body_count(&self) -> usize {
        assert_eq!(
            self.version, VERSION,
            "unsupported source MIR transport version"
        );
        self.bodies.len()
    }

    pub(crate) fn body_at<'tcx>(&self, tcx: TyCtxt<'tcx>, index: usize) -> Body<'tcx> {
        assert_eq!(
            self.version, VERSION,
            "unsupported source MIR transport version"
        );
        let restored = self.restored.get_or_init(|| {
            let mut resolver = Resolver {
                tcx,
                portable: self,
            };
            let mut tokens = Vec::new();
            let mut bodies = Vec::new();
            for body in &self.bodies {
                let start = tokens.len();
                tokens.extend(resolver.tokens(body));
                bodies.push((start, tokens.len()));
            }
            let mut offsets = Vec::new();
            for allocation in &self.allocations {
                offsets.push(tokens.len() as u64);
                tokens.extend(resolver.tokens(allocation));
            }
            std::sync::Mutex::new(Restored {
                tokens,
                bodies,
                allocations: AllocDecodingState::new(offsets),
            })
        });
        let restored = restored
            .lock()
            .expect("portable MIR allocation cache poisoned");
        let (start, end) = restored.bodies[index];
        let mut decoder = Restore::<true> {
            tokens: &restored.tokens,
            position: start,
            tcx,
            allocations: Some(&restored.allocations),
        };
        let body = Body::decode(&mut decoder);
        assert_eq!(
            decoder.position, end,
            "portable source MIR decoder left tokens"
        );
        body
    }
}

// Session-local IDs only. The containing crate cache is cleared at backend initialization.
struct Restored {
    tokens: Vec<Token>,
    bodies: Vec<(usize, usize)>,
    allocations: AllocDecodingState,
}

#[derive(Serialize, Deserialize)]
struct Expansion {
    krate: Vec<PortableToken>,
    index: u32,
    hash: Vec<PortableToken>,
    data: Vec<PortableToken>,
}

#[derive(Serialize, Deserialize)]
struct PortableSpan {
    location: Option<SpanLocation>,
    marks: Vec<(u32, u8)>,
    parent: Option<Vec<PortableToken>>,
}

#[derive(Serialize, Deserialize)]
struct SpanLocation {
    krate: Vec<PortableToken>,
    file: Vec<PortableToken>,
    exported_file: Option<Vec<PortableToken>>,
    lo: u32,
    hi: u32,
}

macro_rules! portable_tokens {
    ($(($variant:ident, $ty:ty, $emit:ident, $read:ident)),*) => {
        #[derive(Serialize, Deserialize)]
        enum PortableToken {
            $($variant($ty),)*
            Text(String), Bytes(Vec<u8>), Span(PortableSpan), Symbol(String), ByteSymbol(Vec<u8>),
            Expansion(u32), Context(Vec<(u32, u8)>), Crate(Vec<PortableToken>), Index(u32),
            Definition(Vec<PortableToken>), Allocation(u32),
        }
        fn number(token: &Token) -> Option<PortableToken> {
            match token { $(Token::$variant(v) => Some(PortableToken::$variant(*v)),)* _ => None }
        }
        fn raw_number(token: &PortableToken) -> Option<Token> {
            match token { $(PortableToken::$variant(v) => Some(Token::$variant(*v)),)* _ => None }
        }
    };
}
numbers!(portable_tokens);

fn encoder<'tcx>() -> Capture<'tcx, true> {
    Capture {
        tokens: Vec::new(),
        types: FxHashMap::default(),
        predicates: FxHashMap::default(),
    }
}

fn capture<'tcx, T: Encodable<Capture<'tcx, true>>>(value: &T) -> Vec<Token> {
    let mut encoder = encoder();
    value.encode(&mut encoder);
    encoder.tokens
}

fn scalar<'tcx, T: Encodable<Capture<'tcx, true>>>(value: &T) -> Vec<PortableToken> {
    capture(value)
        .iter()
        .map(|token| {
            number(token).unwrap_or_else(|| match token {
                Token::Bytes(bytes) => PortableToken::Bytes(bytes.clone()),
                _ => panic!("non-scalar stable identity"),
            })
        })
        .collect()
}

struct Builder<'tcx> {
    tcx: TyCtxt<'tcx>,
    alloc_ids: FxHashMap<AllocId, u32>,
    alloc_queue: Vec<AllocId>,
    expn_ids: FxHashMap<ExpnId, u32>,
    expn_queue: Vec<ExpnId>,
}

impl Builder<'_> {
    fn expansion(&mut self, id: ExpnId) -> u32 {
        if id == ExpnId::root() {
            return ROOT_EXPANSION;
        }
        *self.expn_ids.entry(id).or_insert_with(|| {
            let index = self.expn_queue.len() as u32;
            self.expn_queue.push(id);
            index
        })
    }

    fn marks(&mut self, context: SyntaxContext) -> Vec<(u32, u8)> {
        context
            .marks()
            .into_iter()
            .map(|(id, transparency)| {
                let transparency = match transparency {
                    Transparency::Transparent => 0,
                    Transparency::SemiOpaque => 1,
                    Transparency::Opaque => 2,
                };
                (self.expansion(id), transparency)
            })
            .collect()
    }

    fn span(&mut self, span: Span) -> PortableSpan {
        let data = span.data_untracked();
        let location = if data.is_dummy() {
            None
        } else {
            let file = self.tcx.sess.source_map().lookup_source_file(data.lo);
            Some(SpanLocation {
                krate: scalar(&self.tcx.stable_crate_id(file.cnum)),
                file: scalar(&file.stable_id),
                exported_file: (file.cnum == LOCAL_CRATE).then(|| {
                    let mut name = file.name.clone();
                    if let rustc_span::FileName::Real(name) = &mut name {
                        name.update_for_crate_metadata();
                    }
                    scalar(&StableSourceFileId::from_filename_for_export(
                        &name,
                        self.tcx.stable_crate_id(LOCAL_CRATE),
                    ))
                }),
                lo: data.lo.0 - file.start_pos.0,
                hi: data.hi.0 - file.start_pos.0,
            })
        };
        PortableSpan {
            location,
            marks: self.marks(data.ctxt),
            parent: data
                .parent
                .map(|id| scalar(&self.tcx.def_path_hash(id.to_def_id()))),
        }
    }

    fn tokens(&mut self, tokens: Vec<Token>) -> Vec<PortableToken> {
        tokens
            .into_iter()
            .map(|token| {
                if let Some(number) = number(&token) {
                    return number;
                }
                match token {
                    Token::Text(v) => PortableToken::Text(v),
                    Token::Bytes(v) => PortableToken::Bytes(v),
                    Token::Span(v) => PortableToken::Span(self.span(v)),
                    Token::Symbol(v) => PortableToken::Symbol(v.as_str().to_owned()),
                    Token::ByteSymbol(v) => PortableToken::ByteSymbol(v.as_byte_str().to_vec()),
                    Token::Expansion(v) => PortableToken::Expansion(self.expansion(v)),
                    Token::Context(v) => PortableToken::Context(self.marks(v)),
                    Token::Crate(v) => PortableToken::Crate(scalar(&self.tcx.stable_crate_id(v))),
                    Token::Index(v) => PortableToken::Index(v.as_u32()),
                    Token::Definition(v) => {
                        PortableToken::Definition(scalar(&self.tcx.def_path_hash(v)))
                    }
                    Token::Allocation(v) => {
                        let index = *self.alloc_ids.entry(v).or_insert_with(|| {
                            let index = self.alloc_queue.len() as u32;
                            self.alloc_queue.push(v);
                            index
                        });
                        PortableToken::Allocation(index)
                    }
                    _ => unreachable!("numeric token handled above"),
                }
            })
            .collect()
    }
}

struct Resolver<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    portable: &'a PortableMir,
}

impl<'tcx> Resolver<'_, 'tcx> {
    fn decode<T>(&mut self, encoded: &[PortableToken]) -> T
    where
        for<'b> T: Decodable<Restore<'b, 'tcx, true>>,
    {
        let tokens = self.tokens(encoded);
        let mut decoder = Restore::<true> {
            tokens: &tokens,
            position: 0,
            tcx: self.tcx,
            allocations: None,
        };
        let value = T::decode(&mut decoder);
        assert_eq!(
            decoder.position,
            tokens.len(),
            "portable identity decoder left tokens"
        );
        value
    }

    fn expansion(&mut self, index: u32) -> ExpnId {
        if index == ROOT_EXPANSION {
            return ExpnId::root();
        }
        let expansion = &self.portable.expansions[index as usize];
        let hash: ExpnHash = self.decode(&expansion.hash);
        if let Some(id) = ExpnId::from_hash(hash) {
            return id;
        }
        let krate: StableCrateId = self.decode(&expansion.krate);
        let krate = self.tcx.stable_crate_id_to_crate_num(krate);
        let data: ExpnData = self.decode(&expansion.data);
        if krate == LOCAL_CRATE {
            hygiene::register_local_expn_id(data, hash)
        } else {
            hygiene::register_expn_id(krate, ExpnIndex::from_u32(expansion.index), data, hash)
        }
    }

    fn context(&mut self, marks: &[(u32, u8)]) -> SyntaxContext {
        let mut context = SyntaxContext::root();
        for &(id, transparency) in marks {
            let transparency = match transparency {
                0 => Transparency::Transparent,
                1 => Transparency::SemiOpaque,
                2 => Transparency::Opaque,
                _ => panic!("invalid source MIR transparency"),
            };
            // Rebuild a serialized context with rustc's decoder. apply_mark additionally
            // adjusts call-site hygiene and would duplicate marks already present in the data.
            let expansion = self.expansion(id);
            let tokens = [Token::U32(1)];
            let mut decoder = Restore::<true> {
                tokens: &tokens,
                position: 0,
                tcx: self.tcx,
                allocations: None,
            };
            context = hygiene::decode_syntax_context(
                &mut decoder,
                &hygiene::HygieneDecodeContext::default(),
                |_, _| (context, expansion, transparency),
            );
        }
        context
    }

    fn definition(&mut self, encoded: &[PortableToken]) -> DefId {
        let hash: DefPathHash = self.decode(encoded);
        self.tcx
            .def_path_hash_to_def_id(hash)
            .expect("source MIR definition is missing")
    }

    fn span(&mut self, span: &PortableSpan) -> Span {
        let (lo, hi) = if let Some(location) = &span.location {
            let krate: StableCrateId = self.decode(&location.krate);
            let krate = self.tcx.stable_crate_id_to_crate_num(krate);
            if krate != LOCAL_CRATE {
                self.tcx.import_source_files(krate);
            }
            let id: StableSourceFileId = self.decode(if krate == LOCAL_CRATE {
                &location.file
            } else {
                location.exported_file.as_ref().unwrap_or(&location.file)
            });
            let file = self
                .tcx
                .sess
                .source_map()
                .source_file_by_stable_id(id)
                .unwrap_or_else(|| panic!("source MIR source file {id:?} in {krate:?} is missing"));
            assert!(
                location.lo <= location.hi && location.hi <= file.normalized_source_len.0,
                "source MIR span is outside its source file"
            );
            (
                BytePos(
                    file.start_pos
                        .0
                        .checked_add(location.lo)
                        .expect("source MIR span overflow"),
                ),
                BytePos(
                    file.start_pos
                        .0
                        .checked_add(location.hi)
                        .expect("source MIR span overflow"),
                ),
            )
        } else {
            (BytePos(0), BytePos(0))
        };
        let parent = span
            .parent
            .as_ref()
            .and_then(|parent| self.definition(parent).as_local());
        Span::new(lo, hi, self.context(&span.marks), parent)
    }

    fn tokens(&mut self, tokens: &[PortableToken]) -> Vec<Token> {
        tokens
            .iter()
            .map(|token| {
                if let Some(number) = raw_number(token) {
                    return number;
                }
                match token {
                    PortableToken::Text(v) => Token::Text(v.clone()),
                    PortableToken::Bytes(v) => Token::Bytes(v.clone()),
                    PortableToken::Span(v) => Token::Span(self.span(v)),
                    PortableToken::Symbol(v) => Token::Symbol(Symbol::intern(v)),
                    PortableToken::ByteSymbol(v) => Token::ByteSymbol(ByteSymbol::intern(v)),
                    PortableToken::Expansion(v) => Token::Expansion(self.expansion(*v)),
                    PortableToken::Context(v) => Token::Context(self.context(v)),
                    PortableToken::Crate(v) => {
                        let id: StableCrateId = self.decode(v);
                        Token::Crate(self.tcx.stable_crate_id_to_crate_num(id))
                    }
                    PortableToken::Index(v) => Token::Index(DefIndex::from_u32(*v)),
                    PortableToken::Definition(v) => Token::Definition(self.definition(v)),
                    PortableToken::Allocation(v) => Token::U32(*v),
                    _ => unreachable!("numeric token handled above"),
                }
            })
            .collect()
    }
}

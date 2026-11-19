//! Index options, search options and their merging.
//!
//! MiniSearch merges option objects with spread (`{ ...a, ...b }`): a key `b` holds replaces `a`'s
//! whole value, so object options (`boost`, `weights`, `bm25`) are replaced, never merged key by
//! key. Every option here is an `Option`, `None` meaning "not given", and [`SearchOptions::over`]
//! is that spread.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::text::utf16;

/// A JSON scalar a document field holds.
#[derive(Clone, PartialEq)]
pub enum Value {
    Null,
    Number(f64),
    String(Vec<u16>),
}

/// A document id: a string, or a number (an integer in the signed 32-bit range, CONTRACT.md).
/// `1` and `"1"` are different ids, as they are different `Map` keys.
#[derive(Clone, PartialEq)]
pub enum DocId {
    Number(i32),
    String(String),
}

impl DocId {
    /// The id a field value is, or `None` for the `null` MiniSearch rejects.
    pub fn from_value(value: &Value) -> Option<DocId> {
        match value {
            Value::Null => None,
            Value::Number(number) => {
                let integer = *number as i32;
                assert!(f64::from(integer) == *number, "document ids must be 32-bit integers or strings");
                Some(DocId::Number(integer))
            }
            Value::String(units) => match String::from_utf16(units) {
                Ok(text) => Some(DocId::String(text)),
                Err(_) => panic!("string ids must be well-formed UTF-16"),
            },
        }
    }

    /// The id as a template literal writes it (`${id}`).
    pub fn push_to(&self, out: &mut Vec<u16>) {
        match self {
            DocId::Number(number) => crate::text::push_integer(out, f64::from(*number)),
            DocId::String(text) => out.extend(text.encode_utf16()),
        }
    }
}

/// `prefix`: a boolean, or autoSuggest's default `(term, i, terms) => i === terms.length - 1`.
#[derive(Clone, Copy)]
pub enum Prefix {
    Bool(bool),
    LastTerm,
}

/// `fuzzy`: a boolean or a number.
#[derive(Clone, Copy)]
pub enum Fuzzy {
    Bool(bool),
    Number(f64),
}

#[derive(Clone, Copy)]
pub struct Bm25 {
    pub k: f64,
    pub b: f64,
    pub d: f64,
}

#[derive(Clone, Copy, Default)]
pub struct Weights {
    pub fuzzy: Option<f64>,
    pub prefix: Option<f64>,
}

/// The declarative `filter`: `(result) => values.includes(result[field])`.
#[derive(Clone)]
pub struct Filter {
    pub field: String,
    pub values: Vec<Value>,
}

/// Search options as one object gives them. The values behind an `Rc` are shared, never changed,
/// so a merge copies references, as spreading a JavaScript object does.
#[derive(Clone, Default)]
pub struct SearchOptions {
    pub fields: Option<Rc<[String]>>,
    /// The own keys of the `boost` object, in order, with their values.
    pub boost: Option<Rc<[(String, f64)]>>,
    pub prefix: Option<Prefix>,
    pub fuzzy: Option<Fuzzy>,
    pub max_fuzzy: Option<f64>,
    pub combine_with: Option<Rc<[u16]>>,
    pub weights: Option<Weights>,
    pub bm25: Option<Bm25>,
    pub filter: Option<Rc<Filter>>,
    /// The declarative `boostDocument`: `(id) => factor of the first pair for id, else 1`.
    pub boost_document: Option<Rc<[(DocId, f64)]>>,
    /// The declarative `boostTerm`: `(term) => factor of the first pair for term, else 1`.
    pub boost_term: Option<Rc<[(Vec<u16>, f64)]>>,
}

impl SearchOptions {
    /// MiniSearch's `defaultSearchOptions`.
    pub fn library_defaults() -> SearchOptions {
        SearchOptions {
            combine_with: Some(Rc::from(utf16("or"))),
            prefix: Some(Prefix::Bool(false)),
            fuzzy: Some(Fuzzy::Bool(false)),
            max_fuzzy: Some(6.0),
            boost: Some(Rc::from(Vec::new())),
            weights: Some(Weights { fuzzy: Some(0.45), prefix: Some(0.375) }),
            bm25: Some(Bm25 { k: 1.2, b: 0.7, d: 0.5 }),
            ..SearchOptions::default()
        }
    }

    /// MiniSearch's `defaultAutoSuggestOptions`.
    pub fn auto_suggest_defaults() -> SearchOptions {
        SearchOptions { combine_with: Some(Rc::from(utf16("and"))), prefix: Some(Prefix::LastTerm), ..SearchOptions::default() }
    }

    /// `{ ...self, ...over }`.
    pub fn over(&self, over: &SearchOptions) -> SearchOptions {
        SearchOptions {
            fields: over.fields.clone().or_else(|| self.fields.clone()),
            boost: over.boost.clone().or_else(|| self.boost.clone()),
            prefix: over.prefix.or(self.prefix),
            fuzzy: over.fuzzy.or(self.fuzzy),
            max_fuzzy: over.max_fuzzy.or(self.max_fuzzy),
            combine_with: over.combine_with.clone().or_else(|| self.combine_with.clone()),
            weights: over.weights.or(self.weights),
            bm25: over.bm25.or(self.bm25),
            filter: over.filter.clone().or_else(|| self.filter.clone()),
            boost_document: over.boost_document.clone().or_else(|| self.boost_document.clone()),
            boost_term: over.boost_term.clone().or_else(|| self.boost_term.clone()),
        }
    }

    /// `getOwnProperty(options.boost, field) || 1`.
    pub fn field_boost(&self, field: &str) -> f64 {
        let boost = self.boost.as_ref().and_then(|boost| boost.iter().find(|(name, _)| name == field)).map(|(_, value)| *value);
        match boost {
            Some(value) if value != 0.0 && !value.is_nan() => value,
            _ => 1.0,
        }
    }
}

/// `autoVacuum` when it is not `false`. Missing (or zero) values fall back to the defaults where
/// MiniSearch reads them (`batchSize || 1000`, `minDirtCount || 20`, `minDirtFactor || 0.1`).
#[derive(Clone, Copy, Default)]
pub struct AutoVacuum {
    pub batch_size: Option<f64>,
    pub min_dirt_count: Option<f64>,
    pub min_dirt_factor: Option<f64>,
}

pub struct IndexOptions {
    pub fields: Vec<String>,
    pub id_field: String,
    pub store_fields: Vec<String>,
    /// `defaultSearchOptions` under the constructor's `searchOptions`.
    pub search_options: SearchOptions,
    /// `defaultAutoSuggestOptions` under the constructor's `autoSuggestOptions`.
    pub auto_suggest_options: SearchOptions,
    /// `None` for `autoVacuum: false`; `true` or absent is the default conditions.
    pub auto_vacuum: Option<AutoVacuum>,
}

/// `x || fallback` for a number.
pub fn or_default(value: Option<f64>, fallback: f64) -> f64 {
    match value {
        Some(number) if number != 0.0 && !number.is_nan() => number,
        _ => fallback,
    }
}

//! The boundary: the corpus input (a host object) read into Rust values, the operation script run
//! on [`MiniSearch`], and its results written back as a host object (CONTRACT.md, "Output").
//!
//! Strings that reach the index cross as UTF-16 code units (`charCodeAt`), so lone surrogates
//! survive; names (fields, option keys) cross as host strings read into Rust UTF-8.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use mires_js::{js_bindings, JsValue};

use crate::index::{Document, Error, Hit, MiniSearch, Query, RemoveAll, Serialized, Stats, Suggestion};
use crate::options::{AutoVacuum, Bm25, DocId, Filter, Fuzzy, IndexOptions, Prefix, SearchOptions, Value, Weights};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn property(target: JsValue, key: JsValue) -> JsValue;

    #[js(call = "Object.keys")]
    fn own_keys(target: JsValue) -> JsValue;

    #[js(get = "length")]
    fn length(value: JsValue) -> usize;

    #[js(index)]
    fn element(array: JsValue, index: usize) -> JsValue;

    #[js(method = "charCodeAt")]
    fn code_unit(text: JsValue, index: usize) -> u16;

    #[js(call = "String.fromCharCode")]
    fn from_code_unit(unit: u16) -> JsValue;

    #[js(new = "Object")]
    fn new_object() -> JsValue;

    #[js(new = "Array")]
    fn new_array() -> JsValue;

    #[js(method = "push")]
    fn push_element(array: JsValue, value: JsValue) -> usize;

    #[js(index_set)]
    fn set_property(target: JsValue, key: JsValue, value: JsValue);
}

// ---------------------------------------------------------------------------------------------
// Reading.

fn get(object: JsValue, key: &str) -> Option<JsValue> {
    let value = property(object, JsValue::erase(key));
    (!value.is_undefined()).then_some(value)
}

fn elements(array: JsValue) -> impl Iterator<Item = JsValue> {
    (0..length(array)).map(move |index| element(array, index))
}

fn units(text: JsValue) -> Vec<u16> {
    (0..length(text)).map(|index| code_unit(text, index)).collect()
}

fn name(text: JsValue) -> String {
    String::from(text.read::<&str>())
}

fn number(value: JsValue) -> f64 {
    value.read::<f64>()
}

fn value(value: JsValue) -> Value {
    if value.is_null() {
        Value::Null
    } else if value.is_string() {
        Value::String(units(value))
    } else if value.is_number() {
        Value::Number(value.read::<f64>())
    } else {
        panic!("document values are strings, numbers or null")
    }
}

fn id(value_: JsValue) -> DocId {
    DocId::from_value(&value(value_)).expect("ids are strings or numbers")
}

fn names(array: JsValue) -> Vec<String> {
    elements(array).map(name).collect()
}

fn search_options(object: JsValue) -> SearchOptions {
    SearchOptions {
        fields: get(object, "fields").map(|fields| Rc::from(names(fields))),
        boost: get(object, "boost").map(|boost| elements(own_keys(boost)).map(|key| (name(key), number(property(boost, key)))).collect()),
        prefix: get(object, "prefix").map(|prefix| Prefix::Bool(prefix.read::<bool>())),
        fuzzy: get(object, "fuzzy").map(|fuzzy| if fuzzy.is_boolean() { Fuzzy::Bool(fuzzy.read::<bool>()) } else { Fuzzy::Number(number(fuzzy)) }),
        max_fuzzy: get(object, "maxFuzzy").map(number),
        combine_with: get(object, "combineWith").map(|text| Rc::from(units(text))),
        weights: get(object, "weights").map(|weights| Weights { fuzzy: get(weights, "fuzzy").map(number), prefix: get(weights, "prefix").map(number) }),
        // A missing parameter is undefined in MiniSearch's arithmetic: NaN.
        bm25: get(object, "bm25").map(|bm25| {
            let parameter = |key| get(bm25, key).map_or(f64::NAN, number);
            Bm25 { k: parameter("k"), b: parameter("b"), d: parameter("d") }
        }),
        filter: get(object, "filter").map(|filter| {
            Rc::new(Filter { field: name(property(filter, JsValue::erase("field"))), values: elements(property(filter, JsValue::erase("in"))).map(value).collect() })
        }),
        boost_document: get(object, "boostDocument").map(|pairs| elements(pairs).map(|pair| (id(element(pair, 0)), number(element(pair, 1)))).collect()),
        boost_term: get(object, "boostTerm").map(|pairs| elements(pairs).map(|pair| (units(element(pair, 0)), number(element(pair, 1)))).collect()),
    }
}

fn optional_search_options(object: JsValue, key: &str) -> SearchOptions {
    get(object, key).map(search_options).unwrap_or_default()
}

fn query(value: JsValue) -> Query {
    if value.is_string() {
        return Query::Text(units(value));
    }
    if get(value, "wildcard").is_some_and(|wildcard| wildcard.is_truthy()) {
        return Query::Wildcard;
    }
    Query::Tree { options: search_options(value), queries: elements(property(value, JsValue::erase("queries"))).map(query).collect() }
}

fn index_options(object: JsValue) -> IndexOptions {
    IndexOptions {
        fields: names(property(object, JsValue::erase("fields"))),
        id_field: get(object, "idField").map_or_else(|| String::from("id"), name),
        store_fields: get(object, "storeFields").map(names).unwrap_or_default(),
        search_options: SearchOptions::library_defaults().over(&optional_search_options(object, "searchOptions")),
        auto_suggest_options: SearchOptions::auto_suggest_defaults().over(&optional_search_options(object, "autoSuggestOptions")),
        auto_vacuum: match get(object, "autoVacuum") {
            None => Some(AutoVacuum::default()),
            Some(flag) if flag.is_boolean() => flag.read::<bool>().then(AutoVacuum::default),
            Some(conditions) => Some(AutoVacuum {
                batch_size: get(conditions, "batchSize").map(number),
                min_dirt_count: get(conditions, "minDirtCount").map(number),
                min_dirt_factor: get(conditions, "minDirtFactor").map(number),
            }),
        },
    }
}

/// The fields MiniSearch reads from a document: the id, the indexed and the stored fields.
fn document(object: JsValue, keys: &[String]) -> Document {
    Document { values: keys.iter().filter_map(|key| get(object, key).map(|field| (key.clone(), value(field)))).collect() }
}

enum Operation {
    Add(Document),
    AddAll(Vec<Document>),
    Remove(Document),
    /// `None` for `removeAll()`, `Some(None)` for `removeAll(null)`.
    RemoveAll(Option<Option<Vec<Document>>>),
    Discard(DocId),
    DiscardAll(Vec<DocId>),
    Replace(Document),
    Vacuum(Option<f64>),
    Search(Query, SearchOptions),
    AutoSuggest(Vec<u16>, SearchOptions),
    Has(DocId),
    GetStoredFields(DocId),
    Stats,
    ToJson,
}

fn operation(object: JsValue, keys: &[String]) -> (String, Operation) {
    let op = name(property(object, JsValue::erase("op")));
    let field = |key: &str| property(object, JsValue::erase(key));
    let documents = |value: JsValue| elements(value).map(|item| document(item, keys)).collect();
    let operation = match op.as_str() {
        "add" => Operation::Add(document(field("document"), keys)),
        "addAll" => Operation::AddAll(documents(field("documents"))),
        "remove" => Operation::Remove(document(field("document"), keys)),
        "removeAll" => Operation::RemoveAll(get(object, "documents").map(|value| (!value.is_null()).then(|| documents(value)))),
        "discard" => Operation::Discard(id(field("id"))),
        "discardAll" => Operation::DiscardAll(elements(field("ids")).map(id).collect()),
        "replace" => Operation::Replace(document(field("document"), keys)),
        "vacuum" => Operation::Vacuum(get(object, "options").and_then(|options| get(options, "batchSize")).map(number)),
        "search" => Operation::Search(query(field("query")), optional_search_options(object, "options")),
        "autoSuggest" => Operation::AutoSuggest(units(field("query")), optional_search_options(object, "options")),
        "has" => Operation::Has(id(field("id"))),
        "getStoredFields" => Operation::GetStoredFields(id(field("id"))),
        "stats" => Operation::Stats,
        "toJSON" => Operation::ToJson,
        _ => panic!("unknown operation"),
    };
    (op, operation)
}

// ---------------------------------------------------------------------------------------------
// Writing.

/// A host string with exactly these code units (lone surrogates included), built on the host
/// from its units: no Rust `String` and no UTF-8 decoding in between.
fn string(units: &[u16]) -> JsValue {
    units.iter().fold(JsValue::erase(""), |text, &unit| text + from_code_unit(unit))
}

fn scalar(value: &Value) -> JsValue {
    match value {
        Value::Null => JsValue::null(),
        Value::Number(number) => JsValue::erase(*number),
        Value::String(units) => string(units),
    }
}

fn document_id(id: &DocId) -> JsValue {
    match id {
        DocId::Number(number) => JsValue::erase(f64::from(*number)),
        DocId::String(text) => JsValue::erase(text.as_str()),
    }
}

/// `JSON.stringify` writes a non-finite number as `null`.
fn json_number(number: f64) -> JsValue {
    if number.is_finite() { JsValue::erase(number) } else { JsValue::null() }
}

struct Object(JsValue);

impl Object {
    fn new() -> Object {
        Object(new_object())
    }

    fn set(&self, key: &str, value: JsValue) -> &Object {
        set_property(self.0, JsValue::erase(key), value);
        self
    }

    fn set_index(&self, key: u32, value: JsValue) -> &Object {
        set_property(self.0, JsValue::erase(f64::from(key)), value);
        self
    }
}

fn array(values: impl IntoIterator<Item = JsValue>) -> JsValue {
    let array = new_array();
    for value in values {
        push_element(array, value);
    }
    array
}

fn strings<T: AsRef<[u16]>>(values: impl IntoIterator<Item = T>) -> JsValue {
    array(values.into_iter().map(|units| string(units.as_ref())))
}

fn hit(hit: &Hit, index: &MiniSearch) -> JsValue {
    let fields = index.field_names();
    let object = Object::new();
    object
        .set("id", document_id(&hit.id))
        .set("score", JsValue::erase(hit.score))
        .set("terms", strings(hit.matches.iter().map(|(term, _)| term)))
        .set("queryTerms", strings(&hit.query_terms))
        .set("match", array(hit.matches.iter().map(|(term, matched)| {
            array([string(term), array(matched.iter().map(|&field| JsValue::erase(fields[field].as_str())))])
        })));
    // `Object.assign(result, storedFields)`: a stored `id` replaces the id in place.
    for (key, value) in index.stored_fields(hit.short_id) {
        object.set(key, scalar(value));
    }
    object.0
}

fn suggestion(suggestion: &Suggestion) -> JsValue {
    let object = Object::new();
    object.set("suggestion", string(&suggestion.suggestion)).set("terms", strings(&suggestion.terms)).set("score", JsValue::erase(suggestion.score));
    object.0
}

fn stored(fields: &[(String, Value)]) -> JsValue {
    let object = Object::new();
    for (key, value) in fields {
        object.set(key, scalar(value));
    }
    object.0
}

fn serialized(index: &Serialized<'_>) -> JsValue {
    let document_ids = Object::new();
    for (short_id, id) in &index.document_ids {
        document_ids.set_index(*short_id, document_id(id));
    }
    let field_ids = Object::new();
    for (field, name) in index.field_ids.iter().enumerate() {
        field_ids.set(name, JsValue::erase(field as f64));
    }
    let field_length = Object::new();
    for (short_id, lengths) in &index.field_length {
        field_length.set_index(*short_id, array(lengths.iter().map(|length| length.map_or_else(JsValue::null, |length| JsValue::erase(f64::from(length))))));
    }
    let stored_fields = Object::new();
    for (short_id, fields) in &index.stored_fields {
        stored_fields.set_index(*short_id, stored(fields));
    }
    let terms = array(index.index.iter().map(|(term, fields)| {
        let data = Object::new();
        for (field, posting) in fields {
            let frequencies = Object::new();
            for (short_id, frequency) in posting.iter() {
                frequencies.set_index(*short_id, JsValue::erase(f64::from(*frequency)));
            }
            data.set_index(*field as u32, frequencies.0);
        }
        array([string(term), data.0])
    }));
    let object = Object::new();
    object
        .set("documentCount", JsValue::erase(index.document_count as f64))
        .set("nextId", JsValue::erase(f64::from(index.next_id)))
        .set("documentIds", document_ids.0)
        .set("fieldIds", field_ids.0)
        .set("fieldLength", field_length.0)
        .set("averageFieldLength", array(index.average_field_length.iter().map(|average| average.map_or_else(JsValue::null, json_number))))
        .set("storedFields", stored_fields.0)
        .set("dirtCount", JsValue::erase(index.dirt_count as f64))
        .set("index", terms)
        .set("serializationVersion", JsValue::erase(2.0));
    object.0
}

fn stats(record: &Object, stats: &Stats) {
    record
        .set("documentCount", JsValue::erase(stats.document_count as f64))
        .set("termCount", JsValue::erase(stats.term_count as f64))
        .set("dirtCount", JsValue::erase(stats.dirt_count as f64))
        .set("dirtFactor", JsValue::erase(stats.dirt_factor))
        .set("isVacuuming", JsValue::erase(stats.is_vacuuming));
}

// ---------------------------------------------------------------------------------------------
// The script.

/// Runs one corpus input: `{ options, operations }` to `{ results }`.
pub fn run(input: JsValue) -> JsValue {
    let options = index_options(property(input, JsValue::erase("options")));
    // Every name `extractField` is asked for: the id, the indexed and the stored fields.
    let mut keys = Vec::new();
    for key in core::iter::once(&options.id_field).chain(&options.fields).chain(&options.store_fields) {
        if !keys.contains(key) {
            keys.push(key.clone());
        }
    }
    let operations: Vec<(String, Operation)> = elements(property(input, JsValue::erase("operations"))).map(|item| operation(item, &keys)).collect();

    let mut index = MiniSearch::new(options);
    let results = new_array();
    for (op, operation) in &operations {
        let record = Object::new();
        record.set("op", JsValue::erase(op.as_str()));
        let outcome = execute(&mut index, operation, &record);
        let warnings = index.take_warnings();
        if !warnings.is_empty() {
            record.set("warnings", strings(&warnings));
        }
        if let Err(Error(message)) = outcome {
            record.set("error", string(&message));
        }
        push_element(results, record.0);
    }
    let output = Object::new();
    output.set("results", results);
    output.0
}

fn execute(index: &mut MiniSearch, operation: &Operation, record: &Object) -> Result<(), Error> {
    match operation {
        Operation::Add(document) => index.add(document),
        Operation::AddAll(documents) => index.add_all(documents),
        Operation::Remove(document) => index.remove(document),
        Operation::RemoveAll(argument) => index.remove_all(match argument {
            None => RemoveAll::Everything,
            Some(None) => RemoveAll::Null,
            Some(Some(documents)) => RemoveAll::Documents(documents),
        }),
        Operation::Discard(id) => index.discard(id),
        Operation::DiscardAll(ids) => index.discard_all(ids),
        Operation::Replace(document) => index.replace(document),
        Operation::Vacuum(batch_size) => {
            index.vacuum(*batch_size);
            Ok(())
        }
        Operation::Search(query, options) => {
            let hits = index.search(query, options)?;
            record.set("hits", array(hits.iter().map(|found| hit(found, index))));
            Ok(())
        }
        Operation::AutoSuggest(query, options) => {
            let suggestions = index.auto_suggest(query, options)?;
            record.set("suggestions", array(suggestions.iter().map(suggestion)));
            Ok(())
        }
        Operation::Has(id) => {
            record.set("value", JsValue::erase(index.has(id)));
            Ok(())
        }
        Operation::GetStoredFields(id) => {
            record.set("value", index.get_stored_fields(id).map_or_else(JsValue::null, stored));
            Ok(())
        }
        Operation::Stats => {
            stats(record, &index.stats());
            Ok(())
        }
        Operation::ToJson => {
            record.set("value", serialized(&index.serialized()));
            Ok(())
        }
    }
}

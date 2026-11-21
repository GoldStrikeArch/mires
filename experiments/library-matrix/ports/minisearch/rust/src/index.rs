//! The `MiniSearch` class: documents, the inverted index, search, autoSuggest and vacuuming,
//! following `minisearch/dist/es/index.js` (7.2.0) method by method.

use alloc::borrow::Cow;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

use mires_js::collections::HashMap;

use crate::options::{or_default, AutoVacuum, Bm25, DocId, Fuzzy, IndexOptions, Prefix, SearchOptions, Value};
use crate::searchable_map::SearchableMap;
use crate::text::{join, lower_case_into, property_order, push_integer, to_lower_case, tokenize, utf16};

/// A document's position in insertion order (`shortDocumentId`).
pub type ShortId = u32;
/// A field's position in `fields` (`fieldId`).
pub type FieldId = usize;
/// A term in a result: shared, as a JavaScript string is, by every result that names it.
pub type Term = Rc<[u16]>;

/// The fields of a document MiniSearch reads (`extractField`), absent keys left out.
pub struct Document {
    pub values: Vec<(String, Value)>,
}

impl Document {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.iter().find(|(key, _)| key == name).map(|(_, value)| value)
    }
}

/// A term's index data (`fieldsData`): field id to posting list, in insertion order.
///
/// A posting list (`fieldIndex`, short id to term frequency) is a `Map` in MiniSearch whose keys
/// are always inserted in increasing order: a document only ever adds postings under its own
/// short id, which is larger than every short id before it. So the list here is a vector sorted by
/// short id, which is that `Map`'s insertion order.
pub struct TermData {
    fields: Vec<(FieldId, Vec<(ShortId, u32)>)>,
}

impl TermData {
    fn new() -> TermData {
        TermData { fields: Vec::new() }
    }

    fn posting(&self, field: FieldId) -> Option<&Vec<(ShortId, u32)>> {
        self.fields.iter().find(|(id, _)| *id == field).map(|(_, posting)| posting)
    }

    fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// One document's accumulated result (`{ score, terms, match }`).
pub struct RawResult {
    score: f64,
    /// The query terms that matched (`terms`).
    terms: Vec<Term>,
    /// Derived (document) term to the fields it matched in (`match`), keys in insertion order.
    matches: Vec<(Term, Vec<FieldId>)>,
}

/// `Map<shortId, RawResult>`: a host `Map`, whose insertion order is the result order before
/// sorting (and the order of equal scores after it).
type RawResults = HashMap<ShortId, RawResult>;

pub struct Hit {
    pub id: DocId,
    /// The document, whose stored fields `Object.assign` copies into the result.
    pub short_id: ShortId,
    pub score: f64,
    /// `match` keys in `Object.keys` order, with their fields.
    pub matches: Vec<(Term, Vec<FieldId>)>,
    pub query_terms: Vec<Term>,
}

pub struct Suggestion {
    pub suggestion: Vec<u16>,
    pub terms: Vec<Term>,
    pub score: f64,
}

/// A query: a string, `MiniSearch.wildcard`, or a tree node with its own options.
pub enum Query {
    Text(Vec<u16>),
    Wildcard,
    Tree { options: SearchOptions, queries: Vec<Query> },
}

/// What `removeAll` is called with: documents, an explicit `null`, or no argument at all.
pub enum RemoveAll<'a> {
    Documents(&'a [Document]),
    Null,
    Everything,
}

/// An error MiniSearch throws, with its message.
pub struct Error(pub Vec<u16>);

fn error_with_id(before: &str, id: &DocId, after: &str) -> Error {
    let mut message = utf16(before);
    id.push_to(&mut message);
    message.extend(after.encode_utf16());
    Error(message)
}

/// Postings of discarded documents a search met, removed after the term is scored (see
/// [`MiniSearch::execute_query_spec`]).
type Cleanup = Vec<(Vec<u16>, FieldId, ShortId)>;

pub struct Stats {
    pub document_count: usize,
    pub term_count: usize,
    pub dirt_count: usize,
    pub dirt_factor: f64,
    pub is_vacuuming: bool,
}

/// The value of `JSON.parse(JSON.stringify(index))`, as references into the index.
pub struct Serialized<'a> {
    pub document_count: usize,
    pub next_id: ShortId,
    pub document_ids: Vec<(ShortId, &'a DocId)>,
    pub field_ids: &'a [String],
    pub field_length: Vec<(ShortId, &'a [Option<u32>])>,
    pub average_field_length: &'a [Option<f64>],
    pub stored_fields: Vec<(ShortId, &'a [(String, Value)])>,
    pub dirt_count: usize,
    /// Terms in iteration order; each term's postings by ascending field id.
    pub index: Vec<(Vec<u16>, Vec<(FieldId, &'a [(ShortId, u32)])>)>,
}

#[derive(Default)]
struct IdMap {
    numbers: HashMap<i32, ShortId>,
    strings: HashMap<String, ShortId>,
}

impl IdMap {
    fn get(&self, id: &DocId) -> Option<ShortId> {
        match id {
            DocId::Number(number) => self.numbers.get(number).copied(),
            DocId::String(text) => self.strings.get(text.as_str()).copied(),
        }
    }

    fn insert(&mut self, id: DocId, short_id: ShortId) {
        match id {
            DocId::Number(number) => {
                self.numbers.insert(number, short_id);
            }
            DocId::String(text) => {
                self.strings.insert(text, short_id);
            }
        }
    }

    fn remove(&mut self, id: &DocId) {
        match id {
            DocId::Number(number) => {
                self.numbers.remove(number);
            }
            DocId::String(text) => {
                self.strings.remove(text.as_str());
            }
        }
    }
}

pub struct MiniSearch {
    options: IndexOptions,
    index: SearchableMap<TermData>,
    document_count: usize,
    /// `_documentIds`, indexed by short id; a removed or discarded document is `None`. Short ids
    /// are handed out in increasing order, so ascending short id is that `Map`'s insertion order.
    document_ids: Vec<Option<DocId>>,
    id_to_short_id: IdMap,
    /// `_fieldLength`: per short id, the unique token count of each field the document has.
    field_length: Vec<Option<Vec<Option<u32>>>>,
    /// `_avgFieldLength`, holes included (`None`).
    avg_field_length: Vec<Option<f64>>,
    next_id: ShortId,
    stored_fields: Vec<Option<Vec<(String, Value)>>>,
    dirt_count: usize,
    /// `_currentVacuum != null`. Once a vacuum starts, MiniSearch clears this only in a later
    /// microtask, so for the rest of a synchronous run it stays set (CONTRACT.md, "Vacuuming").
    vacuuming: bool,
    /// What the `logger` option received since the last [`MiniSearch::take_warnings`].
    warnings: Vec<Vec<u16>>,
}

impl MiniSearch {
    pub fn new(options: IndexOptions) -> MiniSearch {
        MiniSearch {
            options,
            index: SearchableMap::new(),
            document_count: 0,
            document_ids: Vec::new(),
            id_to_short_id: IdMap::default(),
            field_length: Vec::new(),
            avg_field_length: Vec::new(),
            next_id: 0,
            stored_fields: Vec::new(),
            dirt_count: 0,
            vacuuming: false,
            warnings: Vec::new(),
        }
    }

    pub fn take_warnings(&mut self) -> Vec<Vec<u16>> {
        core::mem::take(&mut self.warnings)
    }

    pub fn field_names(&self) -> &[String] {
        &self.options.fields
    }

    fn field_id(&self, name: &str) -> Option<FieldId> {
        self.options.fields.iter().position(|field| field == name)
    }

    fn document_id(&self, short_id: ShortId) -> Option<&DocId> {
        self.document_ids.get(short_id as usize).and_then(Option::as_ref)
    }

    fn document_id_of(&self, document: &Document) -> Result<DocId, Error> {
        document.get(&self.options.id_field).and_then(DocId::from_value).ok_or_else(|| {
            let mut message = utf16("MiniSearch: document does not have ID field \"");
            message.extend(self.options.id_field.encode_utf16());
            message.push(0x22);
            Error(message)
        })
    }

    // -----------------------------------------------------------------------------------------
    // Adding and removing documents.

    pub fn add(&mut self, document: &Document) -> Result<(), Error> {
        let id = self.document_id_of(document)?;
        if self.id_to_short_id.get(&id).is_some() {
            return Err(error_with_id("MiniSearch: duplicate ID ", &id, ""));
        }
        let short_id = self.add_document_id(id);
        self.save_stored_fields(short_id, document);
        for field in 0..self.options.fields.len() {
            let Some(text) = field_text(document.get(&self.options.fields[field])) else { continue };
            let tokens = tokenize(&text);
            let unique = unique_count(&tokens);
            self.add_field_length(short_id, field, self.document_count - 1, unique);
            let mut term = Vec::new();
            for token in tokens {
                lower_case_into(&mut term, token);
                if !term.is_empty() {
                    self.add_term(field, short_id, &term);
                }
            }
        }
        Ok(())
    }

    pub fn add_all(&mut self, documents: &[Document]) -> Result<(), Error> {
        documents.iter().try_for_each(|document| self.add(document))
    }

    pub fn remove(&mut self, document: &Document) -> Result<(), Error> {
        let id = self.document_id_of(document)?;
        let Some(short_id) = self.id_to_short_id.get(&id) else {
            return Err(error_with_id("MiniSearch: cannot remove document with ID ", &id, ": it is not in the index"));
        };
        for field in 0..self.options.fields.len() {
            let Some(text) = field_text(document.get(&self.options.fields[field])) else { continue };
            let tokens = tokenize(&text);
            let unique = unique_count(&tokens);
            self.remove_field_length(field, self.document_count, unique);
            let mut term = Vec::new();
            for token in tokens {
                lower_case_into(&mut term, token);
                if !term.is_empty() {
                    self.remove_term(field, short_id, &term);
                }
            }
        }
        self.stored_fields[short_id as usize] = None;
        self.document_ids[short_id as usize] = None;
        self.id_to_short_id.remove(&id);
        self.field_length[short_id as usize] = None;
        self.document_count -= 1;
        Ok(())
    }

    /// `removeAll(documents)`; `removeAll(null)`, an error; or `removeAll()`: a fresh index that
    /// keeps `dirtCount` and the vacuum state.
    pub fn remove_all(&mut self, argument: RemoveAll<'_>) -> Result<(), Error> {
        match argument {
            RemoveAll::Documents(documents) => return documents.iter().try_for_each(|document| self.remove(document)),
            RemoveAll::Null => return Err(Error(utf16("Expected documents to be present. Omit the argument to remove all documents."))),
            RemoveAll::Everything => {}
        }
        self.index = SearchableMap::new();
        self.document_count = 0;
        self.document_ids = Vec::new();
        self.id_to_short_id = IdMap::default();
        self.field_length = Vec::new();
        self.avg_field_length = Vec::new();
        self.stored_fields = Vec::new();
        self.next_id = 0;
        Ok(())
    }

    pub fn discard(&mut self, id: &DocId) -> Result<(), Error> {
        self.discard_document(id)?;
        self.maybe_auto_vacuum();
        Ok(())
    }

    fn discard_document(&mut self, id: &DocId) -> Result<(), Error> {
        let Some(short_id) = self.id_to_short_id.get(id) else {
            return Err(error_with_id("MiniSearch: cannot discard document with ID ", id, ": it is not in the index"));
        };
        self.id_to_short_id.remove(id);
        self.document_ids[short_id as usize] = None;
        self.stored_fields[short_id as usize] = None;
        if let Some(lengths) = self.field_length[short_id as usize].take() {
            // `forEach` over the array skips its holes: fields the document did not have.
            for (field, length) in lengths.iter().enumerate() {
                if let Some(length) = *length {
                    self.remove_field_length(field, self.document_count, length);
                }
            }
        }
        self.document_count -= 1;
        self.dirt_count += 1;
        Ok(())
    }

    /// `discardAll`: automatic vacuuming is off while the ids are discarded, then considered once.
    /// A failing discard ends it there, without that vacuum.
    pub fn discard_all(&mut self, ids: &[DocId]) -> Result<(), Error> {
        for id in ids {
            self.discard_document(id)?;
        }
        self.maybe_auto_vacuum();
        Ok(())
    }

    pub fn replace(&mut self, document: &Document) -> Result<(), Error> {
        let id = document.get(&self.options.id_field).and_then(DocId::from_value).expect("replace needs a document with an id");
        self.discard(&id)?;
        self.add(document)
    }

    pub fn has(&self, id: &DocId) -> bool {
        self.id_to_short_id.get(id).is_some()
    }

    pub fn get_stored_fields(&self, id: &DocId) -> Option<&[(String, Value)]> {
        let short_id = self.id_to_short_id.get(id)?;
        self.stored_fields.get(short_id as usize)?.as_deref()
    }

    fn add_document_id(&mut self, id: DocId) -> ShortId {
        let short_id = self.next_id;
        self.id_to_short_id.insert(id.clone(), short_id);
        self.document_ids.push(Some(id));
        self.field_length.push(None);
        self.stored_fields.push(None);
        self.document_count += 1;
        self.next_id += 1;
        short_id
    }

    fn save_stored_fields(&mut self, short_id: ShortId, document: &Document) {
        if self.options.store_fields.is_empty() {
            return;
        }
        let mut stored: Vec<(String, Value)> = Vec::new();
        for name in &self.options.store_fields {
            if let Some(value) = document.get(name) {
                match stored.iter_mut().find(|(key, _)| key == name) {
                    Some(entry) => entry.1 = value.clone(),
                    None => stored.push((name.clone(), value.clone())),
                }
            }
        }
        self.stored_fields[short_id as usize] = Some(stored);
    }

    fn add_field_length(&mut self, short_id: ShortId, field: FieldId, count: usize, length: u32) {
        let lengths = self.field_length[short_id as usize].get_or_insert_with(Vec::new);
        set_slot(lengths, field, length);
        let average = or_default(self.avg_field_length.get(field).copied().flatten(), 0.0);
        let total = (average * count as f64) + f64::from(length);
        set_slot(&mut self.avg_field_length, field, total / (count as f64 + 1.0));
    }

    fn remove_field_length(&mut self, field: FieldId, count: usize, length: u32) {
        if count == 1 {
            set_slot(&mut self.avg_field_length, field, 0.0);
            return;
        }
        // A hole reads as undefined, which makes the average NaN.
        let average = self.avg_field_length.get(field).copied().flatten().unwrap_or(f64::NAN);
        let total = (average * count as f64) - f64::from(length);
        set_slot(&mut self.avg_field_length, field, total / (count as f64 - 1.0));
    }

    fn add_term(&mut self, field: FieldId, short_id: ShortId, term: &[u16]) {
        let data = self.index.fetch(term, TermData::new);
        match data.fields.iter_mut().find(|(id, _)| *id == field) {
            None => data.fields.push((field, vec![(short_id, 1)])),
            // The document being added has the largest short id, so it is last or absent.
            Some((_, posting)) => match posting.last_mut() {
                Some((last, frequency)) if *last == short_id => *frequency += 1,
                _ => posting.push((short_id, 1)),
            },
        }
    }

    fn remove_term(&mut self, field: FieldId, short_id: ShortId, term: &[u16]) {
        if !self.index.has(term) {
            self.warn_document_changed(short_id, field, term);
            return;
        }
        let data = self.index.fetch(term, TermData::new);
        let found = data.fields.iter().position(|(id, _)| *id == field).and_then(|slot| {
            data.fields[slot].1.binary_search_by_key(&short_id, |(id, _)| *id).ok().map(|at| (slot, at))
        });
        match found {
            None => self.warn_document_changed(short_id, field, term),
            Some((slot, at)) => {
                let posting = &mut data.fields[slot].1;
                if posting[at].1 <= 1 {
                    if posting.len() <= 1 {
                        data.fields.remove(slot);
                    } else {
                        posting.remove(at);
                    }
                } else {
                    posting[at].1 -= 1;
                }
            }
        }
        if self.index.get(term).is_some_and(TermData::is_empty) {
            self.index.delete(term);
        }
    }

    fn warn_document_changed(&mut self, short_id: ShortId, field: FieldId, term: &[u16]) {
        let Some(name) = self.options.fields.get(field) else { return };
        let mut message = utf16("MiniSearch: document with ID ");
        match self.document_id(short_id) {
            Some(id) => id.push_to(&mut message),
            None => message.extend(utf16("undefined")),
        }
        message.extend(utf16(" has changed before removal: term \""));
        message.extend_from_slice(term);
        message.extend(utf16("\" was not present in field \""));
        message.extend(name.encode_utf16());
        message.extend(utf16("\". Removing a document after it has changed can corrupt the index!"));
        self.warnings.push(message);
    }

    // -----------------------------------------------------------------------------------------
    // Vacuuming.

    fn maybe_auto_vacuum(&mut self) {
        let Some(auto) = self.options.auto_vacuum else { return };
        self.conditional_vacuum(auto.batch_size, Some(auto));
    }

    /// `vacuum(options)`: unconditional, unless a vacuum is already current.
    pub fn vacuum(&mut self, batch_size: Option<f64>) {
        self.conditional_vacuum(batch_size, None);
    }

    fn conditional_vacuum(&mut self, batch_size: Option<f64>, conditions: Option<AutoVacuum>) {
        // A request made while a vacuum is current is chained after it, which in a synchronous
        // run is never.
        if self.vacuuming {
            return;
        }
        if let Some(conditions) = conditions {
            if !self.vacuum_conditions_met(conditions) {
                return;
            }
        }
        self.vacuuming = true;
        self.perform_vacuuming(batch_size);
    }

    fn vacuum_conditions_met(&self, conditions: AutoVacuum) -> bool {
        let min_dirt_count = or_default(conditions.min_dirt_count, 20.0);
        let min_dirt_factor = or_default(conditions.min_dirt_factor, 0.1);
        self.dirt_count as f64 >= min_dirt_count && self.dirt_factor() >= min_dirt_factor
    }

    /// The synchronous part of `performVacuuming`: terms in iteration order until the first batch
    /// boundary, where MiniSearch awaits a timer and the rest happens after the run.
    ///
    /// MiniSearch deletes terms from the tree while its iterator walks it; the iterator holds a
    /// snapshot of each node's keys and only the term just visited is ever deleted, so it visits
    /// exactly the terms of the tree as it was when the walk began, which are collected first here.
    fn perform_vacuuming(&mut self, batch_size: Option<f64>) {
        let initial_dirt_count = self.dirt_count;
        let batch_size = or_default(batch_size, 1000.0);
        let mut terms = Vec::new();
        self.index.for_each(|term, _| terms.push(term.to_vec()));
        for (position, term) in terms.iter().enumerate() {
            let document_ids = &self.document_ids;
            let present = |short_id: ShortId| matches!(document_ids.get(short_id as usize), Some(Some(_)));
            let emptied = match self.index.get_mut(term) {
                Some(data) => {
                    for (_, posting) in &mut data.fields {
                        posting.retain(|(short_id, _)| present(*short_id));
                    }
                    data.fields.retain(|(_, posting)| !posting.is_empty());
                    data.is_empty()
                }
                None => false,
            };
            if emptied {
                self.index.delete(term);
            }
            if (position + 1) as f64 % batch_size == 0.0 {
                return;
            }
        }
        self.dirt_count -= initial_dirt_count;
    }

    // -----------------------------------------------------------------------------------------
    // State.

    fn dirt_factor(&self) -> f64 {
        self.dirt_count as f64 / (1.0 + self.document_count as f64 + self.dirt_count as f64)
    }

    pub fn stats(&self) -> Stats {
        Stats {
            document_count: self.document_count,
            term_count: self.index.size(),
            dirt_count: self.dirt_count,
            dirt_factor: self.dirt_factor(),
            is_vacuuming: self.vacuuming,
        }
    }

    pub fn serialized(&self) -> Serialized<'_> {
        let mut index = Vec::new();
        self.index.for_each(|term, data| {
            // `Object.fromEntries` keyed by field id: integer keys, so ascending field id.
            let mut fields: Vec<(FieldId, &[(ShortId, u32)])> = Vec::new();
            for field in 0..self.options.fields.len() {
                if let Some(posting) = data.posting(field) {
                    fields.push((field, posting.as_slice()));
                }
            }
            index.push((term.to_vec(), fields));
        });
        Serialized {
            document_count: self.document_count,
            next_id: self.next_id,
            document_ids: enumerate_present(&self.document_ids),
            field_ids: &self.options.fields,
            field_length: enumerate_present(&self.field_length).into_iter().map(|(id, lengths)| (id, lengths.as_slice())).collect(),
            average_field_length: &self.avg_field_length,
            stored_fields: enumerate_present(&self.stored_fields).into_iter().map(|(id, stored)| (id, stored.as_slice())).collect(),
            dirt_count: self.dirt_count,
            index,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Search.

    pub fn search(&mut self, query: &Query, options: &SearchOptions) -> Result<Vec<Hit>, Error> {
        let with_defaults = self.options.search_options.over(options);
        let mut raw = self.execute_query(query, options)?;
        let mut hits = Vec::new();
        for short_id in keys(&raw) {
            let Some(result) = raw.remove(&short_id) else { continue };
            let quality = if result.terms.is_empty() { 1.0 } else { result.terms.len() as f64 };
            // `Object.keys(match)`: array-index keys first, numerically, the rest as inserted.
            let order = stable_order(result.matches.len(), &|a, b| property_order(&result.matches[a].0, &result.matches[b].0));
            let matches = permuted(result.matches, &order);
            let hit = Hit {
                id: self.document_id(short_id).expect("results only hold present documents").clone(),
                short_id,
                score: result.score * quality,
                matches,
                query_terms: result.terms,
            };
            let keep = match &with_defaults.filter {
                None => true,
                Some(filter) => self.result_property(&hit, &filter.field).is_some_and(|value| filter.values.contains(&value)),
            };
            if keep {
                hits.push(hit);
            }
        }
        // A wildcard query without boostDocument is left unsorted: every score is 1, and the
        // sort is stable, so sorting would not change it either.
        if !(matches!(query, Query::Wildcard) && with_defaults.boost_document.is_none()) {
            let order = stable_order(hits.len(), &|a, b| by_score(hits[a].score, hits[b].score));
            hits = permuted(hits, &order);
        }
        Ok(hits)
    }

    /// `result[field]` on MiniSearch's result object: a stored field (`Object.assign`), else the id.
    fn result_property(&self, hit: &Hit, field: &str) -> Option<Value> {
        if let Some((_, value)) = self.stored_fields(hit.short_id).iter().find(|(name, _)| name == field) {
            return Some(value.clone());
        }
        (field == "id").then(|| match &hit.id {
            DocId::Number(number) => Value::Number(f64::from(*number)),
            DocId::String(text) => Value::String(utf16(text)),
        })
    }

    /// The fields stored for a document (none when `storeFields` is empty).
    pub fn stored_fields(&self, short_id: ShortId) -> &[(String, Value)] {
        self.stored_fields.get(short_id as usize).and_then(Option::as_deref).unwrap_or(&[])
    }

    pub fn auto_suggest(&mut self, query: &[u16], options: &SearchOptions) -> Result<Vec<Suggestion>, Error> {
        let options = self.options.auto_suggest_options.over(options);
        let mut groups: Vec<(Vec<u16>, f64, Vec<Term>, u32)> = Vec::new();
        for hit in self.search(&Query::Text(query.to_vec()), &options)? {
            let terms: Vec<Term> = hit.matches.iter().map(|(term, _)| Rc::clone(term)).collect();
            let phrase = join(&terms, 0x20);
            match groups.iter_mut().find(|(existing, ..)| *existing == phrase) {
                Some(group) => {
                    group.1 += hit.score;
                    group.3 += 1;
                }
                None => groups.push((phrase, hit.score, terms, 1)),
            }
        }
        let suggestions: Vec<Suggestion> = groups
            .into_iter()
            .map(|(suggestion, score, terms, count)| Suggestion { suggestion, terms, score: score / f64::from(count) })
            .collect();
        let order = stable_order(suggestions.len(), &|a, b| by_score(suggestions[a].score, suggestions[b].score));
        Ok(permuted(suggestions, &order))
    }

    fn execute_query(&mut self, query: &Query, options: &SearchOptions) -> Result<RawResults, Error> {
        match query {
            Query::Wildcard => Ok(self.execute_wildcard_query(options)),
            Query::Tree { options: node, queries } => {
                // `{ ...searchOptions, ...query }`: the constructor's options are not merged in, so
                // a node without combineWith (here or above) combines with OR.
                let options = options.over(node);
                let mut results = Vec::with_capacity(queries.len());
                for subquery in queries {
                    results.push(self.execute_query(subquery, &options)?);
                }
                combine_results(results, options.combine_with.as_deref())
            }
            Query::Text(text) => {
                let options = self.options.search_options.over(options);
                let terms: Vec<Vec<u16>> = tokenize(text).into_iter().map(to_lower_case).filter(|term| !term.is_empty()).collect();
                let mut results = Vec::with_capacity(terms.len());
                for (position, term) in terms.iter().enumerate() {
                    let spec = term_to_query_spec(&options, term, position, terms.len());
                    results.push(self.execute_query_spec(&spec, &options));
                }
                combine_results(results, options.combine_with.as_deref())
            }
        }
    }

    /// `executeQuerySpec`: the exact term, then prefix matches, then fuzzy matches.
    ///
    /// Scoring meets postings of discarded documents and removes them (`removeTerm`), which may
    /// delete the term and merge radix nodes. MiniSearch removes each as it meets it; here a term's
    /// removals are applied right after the term is scored. That is the same sequence of index
    /// changes, and nothing in between reads what they change: each term reads only its own
    /// postings, and the prefix and fuzzy walks run over snapshots. The exact term's removals are
    /// applied before the prefix and fuzzy walks start, as in MiniSearch, because the merge they
    /// may cause changes the order of those walks.
    fn execute_query_spec(&mut self, query: &QuerySpec, search_options: &SearchOptions) -> RawResults {
        let options = self.options.search_options.over(search_options);
        let boosts = self.field_boosts(&options);
        let weights = options.weights.unwrap_or_default();
        let fuzzy_weight = weights.fuzzy.unwrap_or(0.45);
        let prefix_weight = weights.prefix.unwrap_or(0.375);
        let context = Scoring {
            boosts: &boosts,
            boost_document: options.boost_document.as_deref(),
            bm25: options.bm25.unwrap_or(Bm25 { k: f64::NAN, b: f64::NAN, d: f64::NAN }),
            term_boost: query.term_boost,
        };
        let source: Term = Rc::from(query.term.clone());
        let term = query.term.as_slice();
        let mut results = RawResults::new();
        let mut cleanup = Cleanup::new();
        self.term_results(&source, &source, 1.0, self.index.get(term), &context, &mut results, &mut cleanup);
        self.clean_up(cleanup);

        let mut fuzzy_matches = Vec::new();
        if let Some(fuzzy) = query.fuzzy.filter(|fuzzy| truthy(*fuzzy)) {
            let fuzzy = match fuzzy {
                Fuzzy::Bool(_) => 0.2,
                Fuzzy::Number(number) => number,
            };
            let max_fuzzy = options.max_fuzzy.unwrap_or(f64::NAN);
            let max_distance = if fuzzy < 1.0 { max_fuzzy.min(js_round(term.len() as f64 * fuzzy)) } else { fuzzy };
            if max_distance != 0.0 && !max_distance.is_nan() {
                fuzzy_matches = self.index.fuzzy_get(term, max_distance as usize);
            }
        }

        if query.prefix {
            let mut cleanup = Cleanup::new();
            self.index.for_each_with_prefix(term, |derived, data| {
                let distance = derived.len() - term.len();
                if distance == 0 {
                    return;
                }
                let weight = prefix_weight * derived.len() as f64 / (derived.len() as f64 + 0.3 * distance as f64);
                self.term_results(&source, &Rc::from(derived.to_vec()), weight, Some(data), &context, &mut results, &mut cleanup);
            });
            self.clean_up(cleanup);
            // A term that is also a prefix match is scored as one only (`fuzzyMatches.delete`):
            // the prefix matches are exactly the index terms that extend the query term.
            fuzzy_matches.retain(|(derived, _)| !(derived.len() > term.len() && derived.starts_with(term)));
        }

        for (derived, distance) in &fuzzy_matches {
            if *distance == 0 {
                continue;
            }
            let weight = fuzzy_weight * derived.len() as f64 / (derived.len() as f64 + f64::from(*distance));
            let mut cleanup = Cleanup::new();
            let data = self.index.get(derived);
            self.term_results(&source, &Rc::from(derived.clone()), weight, data, &context, &mut results, &mut cleanup);
            self.clean_up(cleanup);
        }
        results
    }

    fn execute_wildcard_query(&self, search_options: &SearchOptions) -> RawResults {
        let options = self.options.search_options.over(search_options);
        let mut results = RawResults::new();
        for (short_id, id) in self.document_ids.iter().enumerate() {
            let Some(id) = id else { continue };
            let score = match &options.boost_document {
                Some(pairs) => document_boost(pairs, id),
                None => 1.0,
            };
            results.insert(short_id as ShortId, RawResult { score, terms: Vec::new(), matches: Vec::new() });
        }
        results
    }

    /// The fields to score, in order, with their boost (`fieldBoosts`): an object built from the
    /// field list, so a repeated field keeps its first position.
    fn field_boosts(&self, options: &SearchOptions) -> Vec<(Option<FieldId>, f64)> {
        let fields = options.fields.as_deref().unwrap_or(&self.options.fields);
        let mut boosts: Vec<(&str, f64)> = Vec::new();
        for field in fields {
            if !boosts.iter().any(|(name, _)| *name == field.as_str()) {
                boosts.push((field.as_str(), options.field_boost(field)));
            }
        }
        boosts.into_iter().map(|(name, boost)| (self.field_id(name), boost)).collect()
    }

    /// `termResults`: BM25+ for every posting of `data` in the boosted fields. Postings of
    /// discarded documents are skipped and recorded in `cleanup`, and lower `matchingFields` for
    /// the postings after them.
    #[allow(clippy::too_many_arguments)]
    fn term_results(
        &self,
        source: &Term,
        derived: &Term,
        term_weight: f64,
        data: Option<&TermData>,
        context: &Scoring<'_>,
        results: &mut RawResults,
        cleanup: &mut Cleanup,
    ) {
        let Some(data) = data else { return };
        for &(field, field_boost) in context.boosts {
            let Some(field) = field else { continue };
            let Some(posting) = data.posting(field) else { continue };
            let mut matching = posting.len();
            let average = self.avg_field_length.get(field).copied().flatten().unwrap_or(f64::NAN);
            for &(short_id, frequency) in posting {
                let Some(id) = self.document_id(short_id) else {
                    cleanup.push((derived.to_vec(), field, short_id));
                    matching -= 1;
                    continue;
                };
                let document_boost = match context.boost_document {
                    Some(pairs) => document_boost(pairs, id),
                    None => 1.0,
                };
                if document_boost == 0.0 || document_boost.is_nan() {
                    continue;
                }
                let field_length = self.field_length[short_id as usize]
                    .as_ref()
                    .and_then(|lengths| lengths.get(field).copied().flatten())
                    .map_or(f64::NAN, f64::from);
                let raw = bm25_score(f64::from(frequency), matching as f64, self.document_count as f64, field_length, average, context.bm25);
                let weighted = term_weight * context.term_boost * field_boost * document_boost * raw;
                match results.get_mut(&short_id) {
                    Some(result) => {
                        result.score += weighted;
                        assign_unique_term(&mut result.terms, source);
                        match result.matches.iter_mut().find(|(term, _)| term == derived) {
                            Some((_, fields)) => fields.push(field),
                            None => result.matches.push((Rc::clone(derived), vec![field])),
                        }
                    }
                    None => {
                        results.insert(short_id, RawResult { score: weighted, terms: vec![Rc::clone(source)], matches: vec![(Rc::clone(derived), vec![field])] });
                    }
                }
            }
        }
    }

    fn clean_up(&mut self, cleanup: Cleanup) {
        for (term, field, short_id) in cleanup {
            self.remove_term(field, short_id, &term);
        }
    }
}

/// What `termResults` needs besides the term and its data.
struct Scoring<'a> {
    boosts: &'a [(Option<FieldId>, f64)],
    boost_document: Option<&'a [(DocId, f64)]>,
    bm25: Bm25,
    term_boost: f64,
}

/// One query term with its options (`termToQuerySpec`).
struct QuerySpec {
    term: Vec<u16>,
    fuzzy: Option<Fuzzy>,
    prefix: bool,
    term_boost: f64,
}

fn term_to_query_spec(options: &SearchOptions, term: &[u16], position: usize, count: usize) -> QuerySpec {
    // `options.fuzzy || false`: a falsy fuzzy is `false`.
    let fuzzy = options.fuzzy.filter(|fuzzy| truthy(*fuzzy));
    let prefix = match options.prefix {
        Some(Prefix::LastTerm) => position == count - 1,
        Some(Prefix::Bool(value)) => value,
        None => false,
    };
    let term_boost = match &options.boost_term {
        Some(pairs) => pairs.iter().find(|(key, _)| key.as_slice() == term).map_or(1.0, |(_, factor)| *factor),
        None => 1.0,
    };
    QuerySpec { term: term.to_vec(), fuzzy, prefix, term_boost }
}

fn truthy(fuzzy: Fuzzy) -> bool {
    match fuzzy {
        Fuzzy::Bool(value) => value,
        Fuzzy::Number(number) => number != 0.0 && !number.is_nan(),
    }
}

fn document_boost(pairs: &[(DocId, f64)], id: &DocId) -> f64 {
    pairs.iter().find(|(key, _)| key == id).map_or(1.0, |(_, factor)| *factor)
}

/// `calcBM25Score`, operation for operation.
fn bm25_score(term_frequency: f64, matching: f64, total: f64, field_length: f64, average: f64, Bm25 { k, b, d }: Bm25) -> f64 {
    let inverse_document_frequency = crate::math_log(1.0 + (total - matching + 0.5) / (matching + 0.5));
    inverse_document_frequency * (d + term_frequency * (k + 1.0) / (term_frequency + k * (1.0 - b + b * field_length / average)))
}

/// `Math.round` of a non-negative number: halves round up.
fn js_round(value: f64) -> f64 {
    let floor = value as u32 as f64;
    if value - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// `byScore`, `(a, b) => b.score - a.score`, for a stable sort.
fn by_score(a: f64, b: f64) -> Ordering {
    b.partial_cmp(&a).unwrap_or(Ordering::Equal)
}

fn assign_unique_term(target: &mut Vec<Term>, term: &Term) {
    if !target.contains(term) {
        target.push(Rc::clone(term));
    }
}

fn assign_unique_terms(target: &mut Vec<Term>, source: Vec<Term>) {
    for term in source {
        if !target.contains(&term) {
            target.push(term);
        }
    }
}

/// `Object.assign(target, source)` on two `match` objects: an existing key keeps its place and
/// takes the new value, a new key is added last.
fn assign_matches(target: &mut Vec<(Term, Vec<FieldId>)>, source: Vec<(Term, Vec<FieldId>)>) {
    for (term, fields) in source {
        match target.iter_mut().find(|(existing, _)| *existing == term) {
            Some(entry) => entry.1 = fields,
            None => target.push((term, fields)),
        }
    }
}

fn keys(results: &RawResults) -> Vec<ShortId> {
    results.iter().map(|(&short_id, _)| short_id).collect()
}

/// `combineResults`: no results is an empty map whatever the operator; otherwise the operator is
/// looked up in lower case, and the results are folded from the left.
fn combine_results(results: Vec<RawResults>, combine_with: Option<&[u16]>) -> Result<RawResults, Error> {
    if results.is_empty() {
        return Ok(RawResults::new());
    }
    let combine_with = combine_with.map_or_else(|| utf16("or"), <[u16]>::to_vec);
    let operator = to_lower_case(&combine_with);
    let combinator: fn(RawResults, RawResults) -> RawResults = if operator == utf16("or") {
        combine_or
    } else if operator == utf16("and") {
        combine_and
    } else if operator == utf16("and_not") {
        combine_and_not
    } else {
        let mut message = utf16("Invalid combination operator: ");
        message.extend(combine_with);
        return Err(Error(message));
    };
    let mut results = results.into_iter();
    let first = results.next().unwrap_or_default();
    Ok(results.fold(first, combinator))
}

fn combine_or(mut a: RawResults, mut b: RawResults) -> RawResults {
    for short_id in keys(&b) {
        let Some(incoming) = b.remove(&short_id) else { continue };
        match a.get_mut(&short_id) {
            None => {
                a.insert(short_id, incoming);
            }
            Some(existing) => {
                existing.score = existing.score + incoming.score;
                assign_matches(&mut existing.matches, incoming.matches);
                assign_unique_terms(&mut existing.terms, incoming.terms);
            }
        }
    }
    a
}

/// AND keeps `b`'s order: the combined map is built by walking `b`.
fn combine_and(mut a: RawResults, mut b: RawResults) -> RawResults {
    let mut combined = RawResults::new();
    for short_id in keys(&b) {
        let Some(mut existing) = a.remove(&short_id) else { continue };
        let Some(incoming) = b.remove(&short_id) else { continue };
        assign_unique_terms(&mut existing.terms, incoming.terms);
        assign_matches(&mut existing.matches, incoming.matches);
        combined.insert(short_id, RawResult { score: existing.score + incoming.score, terms: existing.terms, matches: existing.matches });
    }
    combined
}

fn combine_and_not(mut a: RawResults, b: RawResults) -> RawResults {
    for short_id in keys(&b) {
        a.remove(&short_id);
    }
    a
}

/// The text of a field value (`stringifyField`, `fieldValue.toString()`), or `None` for the
/// `null` and absent values MiniSearch skips.
fn field_text(value: Option<&Value>) -> Option<Cow<'_, [u16]>> {
    match value? {
        Value::Null => None,
        Value::String(text) => Some(Cow::Borrowed(text)),
        Value::Number(number) => {
            assert!(*number % 1.0 == 0.0 && *number < 9_007_199_254_740_992.0 && *number > -9_007_199_254_740_992.0, "indexed numbers must be safe integers");
            let mut text = Vec::new();
            push_integer(&mut text, *number);
            Some(Cow::Owned(text))
        }
    }
}

/// `new Set(tokens).size`: distinct raw tokens, before lower-casing, the empty token included.
fn unique_count(tokens: &[&[u16]]) -> u32 {
    let order = stable_order(tokens.len(), &|a, b| tokens[a].cmp(tokens[b]));
    let mut count = 0;
    for (position, &at) in order.iter().enumerate() {
        if position == 0 || tokens[order[position - 1]] != tokens[at] {
            count += 1;
        }
    }
    count
}

/// The positions `0..length` in the order a stable sort by `compare` puts them. Every sort in
/// the port goes through this one `sort_by`: under mires each instantiation of core's sort is
/// about 18 KB of JavaScript (NOTES.md, "Size").
fn stable_order(length: usize, compare: &dyn Fn(usize, usize) -> Ordering) -> Vec<usize> {
    let mut order: Vec<usize> = (0..length).collect();
    order.sort_by(|&a, &b| compare(a, b));
    order
}

/// `items` rearranged into `order` (a permutation of their positions).
fn permuted<T>(items: Vec<T>, order: &[usize]) -> Vec<T> {
    let mut slots: Vec<Option<T>> = Vec::with_capacity(items.len());
    for item in items {
        slots.push(Some(item));
    }
    order.iter().filter_map(|&at| slots[at].take()).collect()
}

/// `array[index] = value`, growing the array with holes.
fn set_slot<T>(slots: &mut Vec<Option<T>>, index: usize, value: T) {
    if slots.len() <= index {
        slots.resize_with(index + 1, || None);
    }
    slots[index] = Some(value);
}

fn enumerate_present<T>(slots: &[Option<T>]) -> Vec<(ShortId, &T)> {
    slots.iter().enumerate().filter_map(|(short_id, slot)| slot.as_ref().map(|value| (short_id as ShortId, value))).collect()
}

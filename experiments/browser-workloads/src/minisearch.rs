//! A compact inverted-index workload inspired by MiniSearch.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone)]
struct Document<'a> {
    id: &'a str,
    title: &'a str,
    body: &'a str,
}

struct TermEntry {
    term: String,
    postings: Vec<Posting>,
}

struct Posting {
    id: String,
    score: u32,
}

/// Indexes `id|title|body` rows and returns ranked `id:score` pairs.
#[must_use]
pub fn search_packed(documents: &str, query: &str) -> String {
    let documents = parse_documents(documents);
    let mut index = Vec::<TermEntry>::new();
    for document in &documents {
        add_terms(&mut index, document.id, document.title, 3);
        add_terms(&mut index, document.id, document.body, 1);
    }

    let mut scores = Vec::<Posting>::new();
    let query_bytes = query.as_bytes();
    let mut query_start = 0;
    while query_start < query_bytes.len() {
        while query_start < query_bytes.len() && query_bytes[query_start].is_ascii_whitespace() {
            query_start += 1;
        }
        let mut query_end = query_start;
        while query_end < query_bytes.len() && !query_bytes[query_end].is_ascii_whitespace() {
            query_end += 1;
        }
        if query_start == query_end {
            break;
        }
        let prefix = query_bytes[query_end - 1] == b'*';
        let term_end = if prefix { query_end - 1 } else { query_end };
        let term = ascii_lower(&query[query_start..term_end]);
        for candidate in &index {
            if candidate.term == term || (prefix && candidate.term.starts_with(&term)) {
                for posting in &candidate.postings {
                    add_score(&mut scores, &posting.id, posting.score);
                }
            }
        }
        query_start = query_end + 1;
    }

    scores.sort_by(|left, right| right.score.cmp(&left.score).then_with(|| left.id.cmp(&right.id)));
    let mut result = String::new();
    for (index, posting) in scores.iter().enumerate() {
        if index > 0 {
            result.push(',');
        }
        result.push_str(&posting.id);
        result.push(':');
        result.push_str(&posting.score.to_string());
    }
    result
}

fn parse_documents(input: &str) -> Vec<Document<'_>> {
    let mut documents = Vec::new();
    let bytes = input.as_bytes();
    let mut row_start = 0;
    while row_start <= bytes.len() {
        let row_end = find_byte(bytes, row_start, b'\n').unwrap_or(bytes.len());
        let Some(first) = find_byte(bytes, row_start, b'|').filter(|index| *index < row_end) else {
            if row_end == bytes.len() { break } else { row_start = row_end + 1; continue }
        };
        let Some(second) = find_byte(bytes, first + 1, b'|').filter(|index| *index < row_end) else {
            if row_end == bytes.len() { break } else { row_start = row_end + 1; continue }
        };
        documents.push(Document {
            id: &input[row_start..first],
            title: &input[first + 1..second],
            body: &input[second + 1..row_end],
        });
        if row_end == bytes.len() {
            break;
        }
        row_start = row_end + 1;
    }
    documents
}

fn find_byte(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] == needle {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn add_terms(
    index: &mut Vec<TermEntry>,
    id: &str,
    text: &str,
    weight: u32,
) {
    let mut seen = Vec::<(String, u32)>::new();
    for term in tokenize(text) {
        if let Some((_, score)) = seen.iter_mut().find(|(candidate, _)| *candidate == term) {
            *score += weight;
        } else {
            seen.push((term, weight));
        }
    }
    for (term, score) in seen {
        let term_index = index.iter().position(|entry| entry.term == term);
        let entry = if let Some(term_index) = term_index {
            &mut index[term_index]
        } else {
            index.push(TermEntry { term, postings: Vec::new() });
            index.last_mut().unwrap()
        };
        add_score(&mut entry.postings, id, score);
    }
}

fn add_score(postings: &mut Vec<Posting>, id: &str, score: u32) {
    if let Some(posting) = postings.iter_mut().find(|posting| posting.id == id) {
        posting.score += score;
    } else {
        postings.push(Posting { id: id.to_string(), score });
    }
}

fn tokenize(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut terms = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        while start < bytes.len() && !bytes[start].is_ascii_alphanumeric() {
            start += 1;
        }
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
            end += 1;
        }
        if end.saturating_sub(start) > 1 {
            terms.push(ascii_lower(&text[start..end]));
        }
        start = end.saturating_add(1);
    }
    terms
}

fn ascii_lower(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for byte in value.bytes() {
        result.push(char::from(byte.to_ascii_lowercase()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_title_matches_and_prefixes() {
        let documents = "a|Rust browser compiler|Fast code\nb|Compiler notes|Browser tools";
        assert_eq!(search_packed(documents, "browser comp*"), "a:6,b:4");
    }
}

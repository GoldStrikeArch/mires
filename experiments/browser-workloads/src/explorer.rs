//! Offline data-explorer composition over storage, search, Markdown, gzip, and canvas.

use alloc::string::String;

use mires_js::{JsValue, future_to_promise};

use crate::host::{get, object, set};
use crate::{fflate, idb, marked, minisearch, uplot};

/// Persists packed rows and returns a rendered, searchable snapshot.
#[must_use]
pub fn run(options: JsValue) -> JsValue {
    future_to_promise(async move {
        let database = get(options, "database");
        let records = get(options, "records");
        let query = get(options, "query");
        let database = if database.is_string() {
            database.read()
        } else {
            "browser-explorer"
        };
        let records = if records.is_string() {
            records.read()
        } else {
            ""
        };
        let query = if query.is_string() { query.read() } else { "" };
        let stored = idb::roundtrip_records(database, records).await?;
        let documents = records_to_documents(&stored);
        let hits = minisearch::search_packed(&documents, query);
        let selected = hits
            .split_once(':')
            .map_or("", |(id, _)| markdown_for(&stored, id));
        let html = marked::render(selected);
        let compressed = fflate::gzip_store(stored.as_bytes());
        let chart = uplot::render_series(
            get(options, "canvas"),
            get(options, "xs"),
            get(options, "ys"),
        );
        let result = object();
        set(result, "hits", JsValue::erase(hits.as_str()));
        set(result, "html", JsValue::erase(html.as_str()));
        set(result, "gzipBytes", JsValue::erase(compressed.len() as f64));
        set(result, "storedBytes", JsValue::erase(stored.len() as f64));
        set(result, "chart", chart);
        Ok(result)
    })
}

fn records_to_documents(records: &str) -> String {
    let mut documents = String::new();
    let bytes = records.as_bytes();
    let mut row_start = 0;
    while row_start <= bytes.len() {
        let row_end = find_byte(bytes, row_start, b'\n').unwrap_or(bytes.len());
        let value_start = find_byte(bytes, row_start, b'=')
            .filter(|index| *index < row_end)
            .map_or(row_start, |index| index + 1);
        let Some(first) = find_byte(bytes, value_start, b'|').filter(|index| *index < row_end)
        else {
            if row_end == bytes.len() {
                break;
            }
            row_start = row_end + 1;
            continue;
        };
        let Some(second) = find_byte(bytes, first + 1, b'|').filter(|index| *index < row_end)
        else {
            if row_end == bytes.len() {
                break;
            }
            row_start = row_end + 1;
            continue;
        };
        if !documents.is_empty() {
            documents.push('\n');
        }
        documents.push_str(&records[value_start..first]);
        documents.push('|');
        documents.push_str(&records[first + 1..second]);
        documents.push('|');
        documents.push_str(&records[second + 1..row_end]);
        if row_end == bytes.len() {
            break;
        }
        row_start = row_end + 1;
    }
    documents
}

fn markdown_for<'a>(records: &'a str, id: &str) -> &'a str {
    let bytes = records.as_bytes();
    let mut row_start = 0;
    while row_start <= bytes.len() {
        let row_end = find_byte(bytes, row_start, b'\n').unwrap_or(bytes.len());
        let value_start = find_byte(bytes, row_start, b'=')
            .filter(|index| *index < row_end)
            .map_or(row_start, |index| index + 1);
        let first = find_byte(bytes, value_start, b'|').filter(|index| *index < row_end);
        let second = first
            .and_then(|first| find_byte(bytes, first + 1, b'|').filter(|index| *index < row_end));
        if let (Some(first), Some(second)) = (first, second) {
            if &records[value_start..first] == id {
                return &records[second + 1..row_end];
            }
        }
        if row_end == bytes.len() {
            break;
        }
        row_start = row_end + 1;
    }
    ""
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

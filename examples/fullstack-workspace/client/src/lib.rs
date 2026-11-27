//! Browser/Node JavaScript entry point compiled from ordinary Rust via `cargo mires`.
//!
//! Exports `shared` domain logic directly to JavaScript/TypeScript with automatic
//! boundary marshaling (`String`, `Vec<T>`, `Option<T>`, `Result<T, E>` -> JS `throw`,
//! plain structs and enums) and zero serialization boilerplate.

use mires::js_export;
use shared::{FilterOptions, TaskItem, TaskSummary};

#[js_export(name = "filterTasks")]
pub fn filter_tasks(tasks: Vec<TaskItem>, options: Option<FilterOptions>) -> Vec<TaskItem> {
    shared::filter_tasks(&tasks, options.as_ref())
}

#[js_export(name = "summarizeTasks")]
pub fn summarize_tasks(tasks: Vec<TaskItem>) -> TaskSummary {
    shared::summarize_tasks(&tasks)
}

#[js_export(name = "validateTitle")]
pub fn validate_title(title: &str) -> Result<String, String> {
    shared::validate_title(title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{Priority, TagCount};

    fn fixture() -> Vec<TaskItem> {
        vec![
            TaskItem {
                id: 1,
                title: "Ship ESM output".to_owned(),
                done: true,
                priority: Priority::High,
                tags: vec!["rust".to_owned(), "js".to_owned()],
            },
            TaskItem {
                id: 2,
                title: "Document quick start".to_owned(),
                done: false,
                priority: Priority::Medium,
                tags: vec!["docs".to_owned(), "rust".to_owned()],
            },
        ]
    }

    #[test]
    fn exported_functions_work_in_native_cargo_test() {
        let filtered = filter_tasks(
            fixture(),
            Some(FilterOptions {
                done: Some(true),
                tag: None,
                min_priority: Some(Priority::High),
            }),
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, 1);

        let summary = summarize_tasks(fixture());
        assert_eq!(summary.total, 2);
        assert_eq!(summary.completed, 1);
        assert_eq!(
            summary.by_tag,
            vec![
                TagCount {
                    tag: "docs".to_owned(),
                    count: 1,
                },
                TagCount {
                    tag: "js".to_owned(),
                    count: 1,
                },
                TagCount {
                    tag: "rust".to_owned(),
                    count: 2,
                },
            ]
        );

        assert_eq!(validate_title("  Ship v1  "), Ok("Ship v1".to_owned()));
        assert!(validate_title("   ").is_err());
    }
}

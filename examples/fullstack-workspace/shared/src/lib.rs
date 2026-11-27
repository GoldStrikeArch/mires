//! Shared domain types and business logic compiled for both native Rust (`server`)
//! and JavaScript (`client` via `cargo mires`).
//!
//! Notice that this crate has ZERO dependency on `mires` or `mires-js`, uses ordinary
//! `std` collections (`HashMap`, `Vec`, `String`), and needs no serialization derives.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItem {
    pub id: u32,
    pub title: String,
    pub done: bool,
    pub priority: Priority,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterOptions {
    pub done: Option<bool>,
    pub tag: Option<String>,
    pub min_priority: Option<Priority>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagCount {
    pub tag: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSummary {
    pub total: u32,
    pub completed: u32,
    pub by_tag: Vec<TagCount>,
}

/// Filters `tasks` according to optional completion status, tag membership, and minimum priority.
pub fn filter_tasks(tasks: &[TaskItem], options: Option<&FilterOptions>) -> Vec<TaskItem> {
    let Some(opts) = options else {
        return tasks.to_vec();
    };

    tasks
        .iter()
        .filter(|task| {
            if let Some(want_done) = opts.done {
                if task.done != want_done {
                    return false;
                }
            }
            if let Some(want_tag) = opts.tag.as_deref() {
                if !task.tags.iter().any(|tag| tag == want_tag) {
                    return false;
                }
            }
            if let Some(min_priority) = opts.min_priority {
                if task.priority < min_priority {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

/// Computes total/completed counts and a sorted per-tag breakdown using `std::collections::HashMap`.
pub fn summarize_tasks(tasks: &[TaskItem]) -> TaskSummary {
    let mut completed = 0u32;
    let mut tag_counts: HashMap<String, u32> = HashMap::new();

    for task in tasks {
        if task.done {
            completed += 1;
        }
        for tag in &task.tags {
            *tag_counts.entry(tag.clone()).or_insert(0) += 1;
        }
    }

    let mut by_tag: Vec<TagCount> = tag_counts
        .into_iter()
        .map(|(tag, count)| TagCount { tag, count })
        .collect();
    by_tag.sort_by(|a, b| a.tag.cmp(&b.tag));

    TaskSummary {
        total: tasks.len() as u32,
        completed,
        by_tag,
    }
}

/// Trims and validates a task title, returning an error message if it is blank or too long.
pub fn validate_title(title: &str) -> Result<String, String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err("task title cannot be empty".to_owned());
    }
    if trimmed.chars().count() > 120 {
        return Err("task title must be 120 characters or fewer".to_owned());
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tasks() -> Vec<TaskItem> {
        vec![
            TaskItem {
                id: 1,
                title: "Set up cargo-mires".to_owned(),
                done: true,
                priority: Priority::High,
                tags: vec!["rust".to_owned(), "tooling".to_owned()],
            },
            TaskItem {
                id: 2,
                title: "Write client filters".to_owned(),
                done: false,
                priority: Priority::Medium,
                tags: vec!["rust".to_owned(), "frontend".to_owned()],
            },
            TaskItem {
                id: 3,
                title: "Polish release notes".to_owned(),
                done: false,
                priority: Priority::Low,
                tags: vec!["docs".to_owned()],
            },
        ]
    }

    #[test]
    fn filters_tasks_by_options() {
        let tasks = sample_tasks();
        assert_eq!(filter_tasks(&tasks, None).len(), 3);

        let done_only = filter_tasks(
            &tasks,
            Some(&FilterOptions {
                done: Some(true),
                ..Default::default()
            }),
        );
        assert_eq!(done_only.len(), 1);
        assert_eq!(done_only[0].id, 1);

        let rust_tag = filter_tasks(
            &tasks,
            Some(&FilterOptions {
                tag: Some("rust".to_owned()),
                ..Default::default()
            }),
        );
        assert_eq!(rust_tag.len(), 2);

        let high_only = filter_tasks(
            &tasks,
            Some(&FilterOptions {
                min_priority: Some(Priority::High),
                ..Default::default()
            }),
        );
        assert_eq!(high_only.len(), 1);
        assert_eq!(high_only[0].id, 1);
    }

    #[test]
    fn summarizes_tasks_with_sorted_tag_counts() {
        let summary = summarize_tasks(&sample_tasks());
        assert_eq!(summary.total, 3);
        assert_eq!(summary.completed, 1);
        assert_eq!(
            summary.by_tag,
            vec![
                TagCount {
                    tag: "docs".to_owned(),
                    count: 1,
                },
                TagCount {
                    tag: "frontend".to_owned(),
                    count: 1,
                },
                TagCount {
                    tag: "rust".to_owned(),
                    count: 2,
                },
                TagCount {
                    tag: "tooling".to_owned(),
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn validates_titles() {
        assert_eq!(validate_title("  Ship v1  "), Ok("Ship v1".to_owned()));
        assert_eq!(
            validate_title("   "),
            Err("task title cannot be empty".to_owned())
        );
        let too_long = "a".repeat(121);
        assert!(validate_title(&too_long).is_err());
    }
}

//! Native Rust server binary sharing the exact same `shared` domain types and
//! validation logic that `client` compiles to JavaScript via `cargo mires`.
//!
//! To build the matching browser/Node ES module bundle alongside this server:
//! ```sh
//! cargo mires build -p client --release --out-dir dist
//! ```
//! You can then serve `dist/client.mjs` directly from an HTTP route (or compile it
//! automatically from a `build.rs` script via `mires_build::Crate::new("../client")`).

use std::path::Path;

use shared::{filter_tasks, summarize_tasks, validate_title, FilterOptions, Priority, TaskItem};

/// Resolves the path where `cargo mires build -p client` publishes the compiled ES module.
fn client_bundle_path(dist_dir: &Path) -> std::path::PathBuf {
    dist_dir.join("client.mjs")
}

fn seed_tasks() -> Result<Vec<TaskItem>, String> {
    Ok(vec![
        TaskItem {
            id: 1,
            title: validate_title("  Compile Rust client to ESM  ")?,
            done: true,
            priority: Priority::High,
            tags: vec!["rust".to_owned(), "mires".to_owned()],
        },
        TaskItem {
            id: 2,
            title: validate_title("  Serve client.mjs from Rust server  ")?,
            done: false,
            priority: Priority::Medium,
            tags: vec!["rust".to_owned(), "web".to_owned()],
        },
    ])
}

fn main() {
    let tasks = seed_tasks().expect("seed task titles are valid");
    let summary = summarize_tasks(&tasks);
    let pending = filter_tasks(
        &tasks,
        Some(&FilterOptions {
            done: Some(false),
            tag: None,
            min_priority: None,
        }),
    );

    println!(
        "Loaded {} tasks ({} completed, {} pending)",
        summary.total,
        summary.completed,
        pending.len()
    );
    for entry in &summary.by_tag {
        println!("  #{}: {}", entry.tag, entry.count);
    }

    let bundle = client_bundle_path(Path::new("dist"));
    if bundle.is_file() {
        println!(
            "Compiled client bundle ready to serve: {}",
            bundle.display()
        );
    } else {
        println!(
            "Tip: run `cargo mires build -p client --release --out-dir dist` to generate {}",
            bundle.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_uses_shared_validation_and_summary() {
        let tasks = seed_tasks().unwrap();
        let summary = summarize_tasks(&tasks);
        assert_eq!(summary.total, 2);
        assert_eq!(summary.completed, 1);
        assert_eq!(tasks[0].title, "Compile Rust client to ESM");
        assert_eq!(
            client_bundle_path(Path::new("dist")),
            Path::new("dist/client.mjs")
        );
    }
}

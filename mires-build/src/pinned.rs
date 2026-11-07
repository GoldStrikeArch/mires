use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

use crate::scrub;

/// The channel assumed by a checkout whose toolchain file cannot be read.
const DEFAULT_CHANNEL: &str = "nightly";

/// The Rust toolchain a spike checkout pins, read from its
/// `rust-toolchain.toml`.
///
/// The codegen backend is a rustc plugin, so it only loads into the exact
/// nightly it was built against. Everything that compiles a client crate names
/// that toolchain in a child process, which is what leaves the crate doing the
/// asking free to be built with any toolchain at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    channel: String,
    components: Vec<String>,
    profile: Option<String>,
}

impl Pinned {
    /// The toolchain pinned by the checkout at `root`.
    ///
    /// A checkout with no readable toolchain file reads as plain `nightly` with
    /// no components: the same thing an environment that pins nothing gets.
    #[must_use]
    pub fn read(root: &Path) -> Self {
        Self::parse(&fs::read_to_string(root.join("rust-toolchain.toml")).unwrap_or_default())
    }

    /// The channel name, as `rustup` spells it.
    #[must_use]
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// The components the checkout needs on top of the channel.
    #[must_use]
    pub fn components(&self) -> &[String] {
        &self.components
    }

    /// The `rustup` profile the channel is installed with, when the checkout
    /// names one.
    #[must_use]
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// The `rustup` arguments that install this toolchain.
    #[must_use]
    pub fn install_args(&self) -> Vec<String> {
        let mut args = vec![
            "toolchain".to_owned(),
            "install".to_owned(),
            self.channel.clone(),
        ];
        if let Some(profile) = &self.profile {
            args.push("--profile".to_owned());
            args.push(profile.clone());
        }
        for component in &self.components {
            args.push("--component".to_owned());
            args.push(component.clone());
        }
        args
    }

    /// Whether `rustup` reports this toolchain as installed.
    ///
    /// A machine without `rustup` cannot be asked, and reads as installed:
    /// there is nothing there to install with, and the toolchain already in
    /// use may well be the right one.
    #[must_use]
    pub fn is_installed(&self) -> bool {
        let mut command = Command::new("rustup");
        command
            .args(["toolchain", "list"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        scrub(&mut command);

        let Ok(output) = command.output() else {
            return true;
        };
        if !output.status.success() {
            return true;
        }
        listed(&String::from_utf8_lossy(&output.stdout), &self.channel)
    }

    /// Read a toolchain file's contents.
    ///
    /// Only the three keys that decide what to install are read, and a value
    /// is taken from the first line that carries its key. Anything after a `#`
    /// is a comment.
    fn parse(manifest: &str) -> Self {
        let mut channel = None;
        let mut profile = None;
        let mut components = Vec::new();
        // Set while a `components` array is still open, so the entries of a
        // list written over several lines are all collected.
        let mut open = false;

        for line in manifest.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if open {
                open = !line.contains(']');
                components.extend(entries(line));
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "channel" => channel = channel.or_else(|| unquote(value)),
                "profile" => profile = profile.or_else(|| unquote(value)),
                "components" if components.is_empty() => {
                    open = !value.contains(']');
                    components.extend(entries(value));
                }
                _ => {}
            }
        }

        Self {
            channel: channel.unwrap_or_else(|| DEFAULT_CHANNEL.to_owned()),
            components,
            profile,
        }
    }
}

/// The quoted string in `value`, or `None` when it holds nothing.
fn unquote(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"').trim_matches('\'');
    (!value.is_empty()).then(|| value.to_owned())
}

/// The quoted strings on one line of a TOML array.
fn entries(line: &str) -> Vec<String> {
    line.trim_start_matches(['=', '['])
        .trim_end_matches(']')
        .split(',')
        .filter_map(unquote)
        .collect()
}

/// Whether `rustup toolchain list` output names `channel`.
///
/// Each line is a channel with the host target appended, so a match is the
/// channel itself or the channel followed by a target. A target never starts
/// with a digit, which is what keeps a bare `nightly` from matching a dated
/// one.
fn listed(output: &str, channel: &str) -> bool {
    output.lines().any(|line| {
        let Some(name) = line.split_whitespace().next() else {
            return false;
        };
        if name == channel {
            return true;
        }
        name.strip_prefix(channel)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|target| !target.starts_with(|c: char| c.is_ascii_digit()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_toolchain_is_read_from_the_checkout() {
        let pinned = Pinned::parse(concat!(
            "[toolchain]\n",
            "channel = \"nightly-2026-07-27\"\n",
            "components = [\"rust-src\", \"rustc-dev\", \"llvm-tools\"]\n",
            "profile = \"minimal\"\n",
        ));

        assert_eq!(pinned.channel(), "nightly-2026-07-27");
        assert_eq!(pinned.components(), ["rust-src", "rustc-dev", "llvm-tools"]);
        assert_eq!(pinned.profile(), Some("minimal"));
    }

    #[test]
    fn a_checkout_with_no_toolchain_file_falls_back_to_nightly() {
        let pinned = Pinned::parse("");

        assert_eq!(pinned.channel(), DEFAULT_CHANNEL);
        assert!(pinned.components().is_empty());
        assert_eq!(pinned.profile(), None);
    }

    #[test]
    fn components_written_over_several_lines_are_all_read() {
        let pinned = Pinned::parse(concat!(
            "[toolchain]\n",
            "channel = \"nightly\"\n",
            "components = [\n",
            "  \"rust-src\",\n",
            "  \"rustc-dev\",\n",
            "]\n",
            "profile = \"minimal\"\n",
        ));

        assert_eq!(pinned.components(), ["rust-src", "rustc-dev"]);
        assert_eq!(pinned.profile(), Some("minimal"));
    }

    #[test]
    fn comments_are_not_values() {
        let pinned = Pinned::parse(concat!(
            "# channel = \"beta\"\n",
            "channel = \"nightly-2026-07-27\" # the pin\n",
        ));

        assert_eq!(pinned.channel(), "nightly-2026-07-27");
    }

    #[test]
    fn the_install_command_carries_the_profile_and_every_component() {
        let pinned = Pinned::parse(concat!(
            "channel = \"nightly-2026-07-27\"\n",
            "components = [\"rust-src\", \"rustc-dev\"]\n",
            "profile = \"minimal\"\n",
        ));

        assert_eq!(
            pinned.install_args(),
            [
                "toolchain",
                "install",
                "nightly-2026-07-27",
                "--profile",
                "minimal",
                "--component",
                "rust-src",
                "--component",
                "rustc-dev",
            ]
        );
    }

    #[test]
    fn a_channel_alone_installs_with_no_options() {
        let pinned = Pinned::parse("channel = \"nightly\"\n");

        assert_eq!(pinned.install_args(), ["toolchain", "install", "nightly"]);
    }

    #[test]
    fn a_listed_channel_is_found_with_its_target_appended() {
        let output = concat!(
            "stable-aarch64-apple-darwin (active, default)\n",
            "nightly-2026-07-27-aarch64-apple-darwin\n",
        );

        assert!(listed(output, "nightly-2026-07-27"));
        assert!(listed(output, "stable"));
    }

    #[test]
    fn a_channel_that_is_not_listed_is_not_found() {
        let output = "nightly-2026-07-26-aarch64-apple-darwin\n";

        assert!(!listed(output, "nightly-2026-07-27"));
        assert!(!listed("", "nightly-2026-07-27"));
    }

    #[test]
    fn a_dated_nightly_does_not_stand_in_for_the_plain_channel() {
        let output = "nightly-2026-07-27-aarch64-apple-darwin\n";

        assert!(!listed(output, "nightly"));
        assert!(listed("nightly-aarch64-apple-darwin\n", "nightly"));
    }
}

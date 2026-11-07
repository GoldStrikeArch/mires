use std::{
    fmt,
    path::{Path, PathBuf},
};

/// Which piece of the setup an [`Unmet`] precondition is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The nightly the codegen backend loads into.
    Toolchain,
    /// The codegen backend dylib.
    Backend,
    /// The sysroot holding the `core` the backend compiled.
    Sysroot,
    /// The `#[js_extern]` proc macro, built for the host that runs it.
    ExternMacro,
}

impl Step {
    /// Every step, in the order they have to be performed: each one is built
    /// with the ones before it.
    pub const ALL: [Self; 4] = [
        Self::Toolchain,
        Self::Backend,
        Self::Sysroot,
        Self::ExternMacro,
    ];

    /// A short name for the step, for progress output.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Toolchain => "toolchain",
            Self::Backend => "backend",
            Self::Sysroot => "sysroot",
            Self::ExternMacro => "js-extern macro",
        }
    }
}

/// A setup step that has to be performed before a client crate can be
/// compiled, with the reason it is needed and the command that performs it.
///
/// [`Toolchain::preconditions`](crate::Toolchain::preconditions) is the one
/// place these are decided, so a build script that reports them and a tool that
/// runs them agree on when a checkout is ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmet {
    reason: String,
    fix: Fix,
}

impl Unmet {
    /// A step that is missing for `reason`, satisfied by `fix`.
    pub(crate) fn new(reason: String, fix: Fix) -> Self {
        Self { reason, fix }
    }

    /// Which piece of the setup is missing.
    #[must_use]
    pub fn step(&self) -> Step {
        self.fix.step()
    }

    /// Why the step is needed, as a sentence naming the path that is missing
    /// or the options that disagree.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The command that performs the step.
    #[must_use]
    pub fn fix(&self) -> &Fix {
        &self.fix
    }
}

impl fmt::Display for Unmet {
    /// The reason, then the command that fixes it on its own line, so an error
    /// a build script prints holds something a reader can copy.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n{}", self.reason, self.fix)
    }
}

/// The command that satisfies an [`Unmet`] precondition.
///
/// It is both what gets run and what gets printed: the parts are kept apart so
/// running it needs no shell, and [`Display`](fmt::Display) writes the shell
/// line that does the same thing by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    step: Step,
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    dir: Option<PathBuf>,
}

impl Fix {
    /// The command performing `step` by running `program` with no arguments.
    pub(crate) fn new(step: Step, program: impl Into<String>) -> Self {
        Self {
            step,
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            dir: None,
        }
    }

    /// Append an argument.
    pub(crate) fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Append several arguments, in order.
    pub(crate) fn args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set an environment variable for the command.
    pub(crate) fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Run the command in `dir`.
    pub(crate) fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Which piece of the setup the command performs.
    #[must_use]
    pub fn step(&self) -> Step {
        self.step
    }

    /// The program to run, looked up on `PATH` when it is a bare name.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The arguments to run it with.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.args
    }

    /// The environment variables to set, on top of the ones inherited.
    #[must_use]
    pub fn environment(&self) -> &[(String, String)] {
        &self.env
    }

    /// The directory to run it in, when it only works from one.
    #[must_use]
    pub fn directory(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// `word` with the command's directory stripped off, so a path inside it
    /// reads the way someone standing there would type it.
    fn shorten(&self, word: &str) -> String {
        let Some(dir) = &self.dir else {
            return word.to_owned();
        };
        Path::new(word)
            .strip_prefix(dir)
            .map_or_else(|_| word.to_owned(), |rest| rest.display().to_string())
    }
}

impl fmt::Display for Fix {
    /// The command as a shell line: a `cd` into its directory, the environment
    /// it needs, then the command itself with paths inside that directory
    /// written relative to it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(dir) = &self.dir {
            write!(f, "cd {} && ", dir.display())?;
        }
        for (key, value) in &self.env {
            write!(f, "{key}='{value}' ")?;
        }
        f.write_str(&self.shorten(&self.program))?;
        for arg in &self.args {
            write!(f, " {}", self.shorten(arg))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_command_prints_as_it_runs() {
        let fix = Fix::new(Step::Toolchain, "rustup").args(["toolchain", "install", "nightly"]);

        assert_eq!(fix.to_string(), "rustup toolchain install nightly");
        assert_eq!(fix.step(), Step::Toolchain);
        assert_eq!(fix.program(), "rustup");
        assert_eq!(fix.arguments(), ["toolchain", "install", "nightly"]);
        assert_eq!(fix.directory(), None);
    }

    #[test]
    fn a_command_with_a_directory_prints_the_cd_that_reaches_it() {
        let fix = Fix::new(Step::ExternMacro, "cargo")
            .args(["build", "--release", "-p", "js-extern-macro"])
            .dir("/spike");

        assert_eq!(
            fix.to_string(),
            "cd /spike && cargo build --release -p js-extern-macro"
        );
        assert_eq!(fix.directory(), Some(Path::new("/spike")));
    }

    #[test]
    fn paths_inside_the_directory_print_relative_to_it() {
        let fix = Fix::new(Step::Backend, "bash")
            .arg("/spike/scripts/build.sh")
            .dir("/spike");

        assert_eq!(fix.to_string(), "cd /spike && bash scripts/build.sh");
        assert_eq!(fix.arguments(), ["/spike/scripts/build.sh"]);
    }

    #[test]
    fn the_environment_prints_ahead_of_the_command() {
        let fix = Fix::new(Step::Sysroot, "bash")
            .arg("/spike/scripts/build_sysroot.sh")
            .env("JS_EXTRA_ARGS", "js-minify=on")
            .dir("/spike");

        assert_eq!(
            fix.to_string(),
            "cd /spike && JS_EXTRA_ARGS='js-minify=on' bash scripts/build_sysroot.sh"
        );
        assert_eq!(
            fix.environment(),
            [("JS_EXTRA_ARGS".to_owned(), "js-minify=on".to_owned())]
        );
    }

    #[test]
    fn an_unmet_step_prints_its_reason_above_its_fix() {
        let unmet = Unmet::new(
            "the codegen backend is not built at /spike/target/release/lib.dylib".to_owned(),
            Fix::new(Step::Backend, "bash")
                .arg("/spike/scripts/build.sh")
                .dir("/spike"),
        );

        assert_eq!(unmet.step(), Step::Backend);
        assert_eq!(
            unmet.to_string(),
            concat!(
                "the codegen backend is not built at /spike/target/release/lib.dylib\n",
                "cd /spike && bash scripts/build.sh",
            )
        );
    }

    #[test]
    fn every_step_has_a_label() {
        assert_eq!(Step::Toolchain.label(), "toolchain");
        assert_eq!(Step::Backend.label(), "backend");
        assert_eq!(Step::Sysroot.label(), "sysroot");
        assert_eq!(Step::ExternMacro.label(), "js-extern macro");
    }
}

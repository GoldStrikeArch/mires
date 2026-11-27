//! `cargo-mires` — Cargo subcommand and `RUSTC_WRAPPER` for compiling Rust crates to JavaScript.

mod build;
mod cli;
mod metadata;
mod npm;
mod sysroot;
mod watch;
mod wrapper;

use std::env;
use std::process::ExitCode;

use cli::Command;

fn main() -> ExitCode {
    // Single-binary dual role: when Cargo invokes us as `RUSTC_WRAPPER`, bypass CLI parsing
    // immediately and forward/rewrite the `rustc` invocation cleanly.
    if env::var_os(wrapper::ENV_WRAPPER_ACTIVE).as_deref() == Some(std::ffi::OsStr::new("1")) {
        return wrapper::run();
    }

    let command = match cli::parse_args(env::args_os()) {
        Ok(cmd) => cmd,
        Err(err) => {
            eprintln!("error: {err}");
            eprintln!("run `cargo mires --help` for usage");
            return ExitCode::from(2);
        }
    };

    let result = match command {
        Command::Help(topic) => {
            print!("{}", cli::help_text(topic.as_deref()));
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("{}", cli::version_text());
            return ExitCode::SUCCESS;
        }
        Command::Build(opts) => build::run_build(&opts),
        Command::Check(opts) => build::run_check(&opts),
        Command::Watch(opts) => watch::run(&opts),
        Command::Sysroot(opts) => sysroot::run_sysroot_cmd(&opts),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

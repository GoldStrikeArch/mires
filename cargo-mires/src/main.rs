//! `cargo-mires` — Cargo subcommand, `RUSTC_WRAPPER`, and toolchain manager for Mires.

mod archive;
mod build;
mod cli;
mod metadata;
mod npm;
mod patch;
mod sysroot;
mod sysroot_build;
mod test_cmd;
mod toolchain;
mod watch;
mod wrapper;

use std::env;
use std::process::ExitCode;

use cli::Command;

fn main() -> ExitCode {
    // Dual/triple role binary:
    // 1. Link wrapper (`JSC_LINK_BIN`) for `cargo mires test` binaries to root & export `main()`.
    if env::var_os(wrapper::ENV_LINK_WRAPPER).as_deref() == Some(std::ffi::OsStr::new("1")) {
        return wrapper::run_link_wrapper();
    }
    // 2. `RUSTC_WRAPPER` invoked by Cargo during `build`, `check`, and `test`.
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
        Command::Test(opts) => test_cmd::run_test(&opts),
        Command::Watch(opts) => watch::run(&opts),
        Command::Sysroot(opts) => sysroot::run_sysroot_cmd(&opts),
        Command::Setup(opts) => toolchain::run_setup_cmd(&opts),
        Command::Toolchain(cmd) => toolchain::run_toolchain_cmd(&cmd),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

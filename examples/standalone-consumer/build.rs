fn main() {
    mires_build::BuildConfig::new("client/app.rs")
        .profile(mires_build::Profile::ReleaseBalanced)
        .output_name("client.js")
        .render()
        .expect("compile the framework-independent Rust client");
}

//! Standalone askpass helper with the same behaviour as the GlideGit binary's
//! askpass mode. Used by the integration tests (the app binary needs Tauri).
fn main() {
    std::process::exit(glide_core::askpass::client_main_if_requested().unwrap_or(2));
}

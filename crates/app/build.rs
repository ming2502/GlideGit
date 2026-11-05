fn main() {
    // The Windows bundle ships MinGit from target/mingit (filled by
    // scripts/fetch-mingit.mjs; see tauri.windows.conf.json). tauri-build
    // checks that resource paths exist on every build, so builds without it
    // (development, CI checks) get an empty folder: the app then simply has
    // no bundled git to fall back to.
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/mingit");
        let _ = std::fs::create_dir_all(dir);
    }
    tauri_build::build()
}

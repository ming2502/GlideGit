//! Check an update manifest the way the app does, without the app:
//! fetch it, pick this platform's package, download it and verify its
//! signature (and the version it was signed for) against the public key.
//!
//! ```text
//! cargo run -p glidegit --example updater_check -- <manifest-url> <pubkey> [current-version]
//! ```
//!
//! With the published release:
//! `… -- https://github.com/ming2502/GlideGit/releases/latest/download/latest.json "$(cat key.pub)" 0.0.0`.
//! A plain `http://` URL (a local test server) is allowed here, not in the app.

use tauri_plugin_updater::UpdaterExt;

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: updater_check <manifest-url> <pubkey> [current-version]";
    let url: tauri::Url = args.next().expect(usage).parse().expect("a manifest URL");
    let pubkey = args.next().expect(usage);
    let current = args.next().unwrap_or_else(|| "0.0.0".into());

    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.package_info_mut().version = current.parse().expect("a semver current version");
    context.config_mut().plugins.0.insert(
        "updater".into(),
        serde_json::json!({
            "pubkey": pubkey,
            "endpoints": [],
            "requireSignedVersion": true,
            "dangerousInsecureTransportProtocol": url.scheme() == "http",
        }),
    );
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .expect("test app");

    tauri::async_runtime::block_on(async move {
        let updater = app
            .handle()
            .updater_builder()
            .endpoints(vec![url])
            .expect("endpoint")
            .build()
            .expect("updater");
        match updater.check().await {
            Ok(Some(update)) => {
                println!("update: {} -> {}", update.current_version, update.version);
                println!("package: {}", update.download_url);
                match update.download(|_, _| {}, || {}).await {
                    Ok(bytes) => println!("downloaded {} bytes, signature OK", bytes.len()),
                    Err(e) => {
                        println!("REJECTED: {e}");
                        std::process::exit(1);
                    }
                }
            }
            Ok(None) => println!("no update (already at {current} or newer)"),
            Err(e) => {
                println!("check failed: {e}");
                std::process::exit(1);
            }
        }
    });
}

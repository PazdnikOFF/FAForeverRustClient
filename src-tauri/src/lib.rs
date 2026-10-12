//! Tauri shell: the thin glue between [`faf_app`] and the webview.
//!
//! Responsibilities (and nothing more, per ARCHITECTURE.md §2):
//! - expose the typed application bridge and narrowly scoped OS integrations
//!   ([`commands`]);
//! - forward every [`AppEvent`] from the core to the frontend on `app://event`
//!   ([`startup`]);
//! - own desktop-only concerns: the window and what may load in it
//!   ([`window`], [`navigation`]), the tray ([`tray`]), and diagnostic-log
//!   access ([`diagnostics`]).
//!
//! All logic lives in `faf-app`/`faf-domain`. [`run`] only assembles the
//! modules above in the order Tauri needs them; it must stay boring.

use std::sync::Arc;

use faf_app::App;
use faf_domain::{AppEvent, AppState};
use serde::Serialize;
use tauri::Manager;

mod commands;
mod diagnostics;
mod navigation;
mod startup;
mod tray;
mod window;

/// The channel the backend emits state deltas on. The frontend listens here.
const EVENT_CHANNEL: &str = "app://event";

/// One ordered shell-to-webview stream. Recovery snapshots deliberately use
/// the same channel as deltas so the webview cannot observe a post-recovery
/// event and then roll itself back to an older snapshot from another channel.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum FrontendMessage {
    Event { revision: u64, event: Box<AppEvent> },
    Snapshot { revision: u64, state: Box<AppState> },
}

/// Managed state: the shared application core.
struct Core(Arc<App>);

pub fn run() {
    // Before anything else: this process may be the Steam status helper the
    // launcher starts beside a game, which is not a client. It must not open a
    // window, and it must not reach the single-instance check below, which
    // would hand it to the running client as a second start.
    let arguments: Vec<String> = std::env::args().collect();
    if let Some(code) = faf_app::infra::steam_presence::run_helper_if_asked(&arguments) {
        std::process::exit(code);
    }

    #[cfg(windows)]
    if std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_err() {
        std::env::set_var(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "--enable-features=msLowMemoryMode --renderer-process-limit=1 --js-flags=\"--max-old-space-size=256 --scavenger_max_new_space_capacity_mb=8\" --disable-features=Translate,OptimizationHints,MediaRouter",
        );
    }

    tauri::Builder::default()
        // First, and the order is not cosmetic: this is what decides whether
        // this process is the client or a messenger for one that is already
        // running, and everything below assumes it is the client.
        //
        // A second start is not an error to report. Double-clicking a
        // `.fafreplay` is how most people will open one, and the shell starts
        // a whole new process for it: the useful answer is to hand the path to
        // the client that is already up, raise its window, and exit quietly.
        // Anything else means two clients fighting over one lobby connection.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                // Unminimise first: `set_focus` on a minimised window raises
                // nothing on Windows and the click appears to do nothing.
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            if let Some(path) = startup::replay_argument(&argv) {
                startup::open_replay_from_argument(app, path);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        // Where the window was, how big, on which monitor, and whether it was
        // maximised. Restored on the next start, and it belongs here rather
        // than in the settings store: it is a property of the shell, not of the
        // account, and this file is the only place that knows about windows.
        //
        // Three flags, not `all()`. `VISIBLE` would faithfully restore a client
        // that was hidden to the tray when it last exited, which is a client
        // that starts invisible; `DECORATIONS` and `FULLSCREEN` are not states
        // this client puts a window into.
        //
        // The plugin is also what makes the "if they're still valid" half of
        // this work: it only restores a position that some currently attached
        // monitor still covers, and otherwise leaves the placement to the OS.
        // A monitor that was unplugged therefore costs the position and nothing
        // else, rather than opening the window somewhere nobody can see.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .on_window_event(window::on_window_event)
        .setup(|app| {
            let log_dir = app.path().app_log_dir()?;
            app.manage(diagnostics::init(&log_dir)?);
            // Before anything resolves a path: the client used to store its
            // cache, data and settings under "forgeclient/forge-client", and
            // this moves them to the current name once. Best effort, and a
            // no-op after the first run. Placed after diagnostics so the
            // outcome is actually logged.
            faf_app::infra::migrate_legacy_directories();
            let ice_log_dir = log_dir.join("iceAdapterLogs");
            std::fs::create_dir_all(&ice_log_dir)?;
            std::env::set_var("FAF_ICE_LOG_DIR", &ice_log_dir);
            let backend_version = env!("CARGO_PKG_VERSION").to_string();
            startup::locate_bundled_helpers(app);
            let core = startup::start_core(app, backend_version);
            app.manage(Core(core));

            // The other half of the file association: this is the client being
            // started *by* a double-click rather than being told about one by
            // a second process. After `app.manage`, because the dispatch looks
            // the `Core` up out of Tauri's state and would otherwise find
            // nothing and log a warning about its own startup.
            let arguments: Vec<String> = std::env::args().collect();
            if let Some(path) = startup::replay_argument(&arguments) {
                startup::open_replay_from_argument(app.handle(), path);
            }

            window::build_main(app)?;
            tray::build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::dispatch,
            commands::dispatch_and_wait,
            commands::snapshot,
            commands::open_log_folder,
            commands::open_client_folder,
            commands::client_folder_path,
            commands::import_notification_sound,
            commands::list_notification_sounds,
            commands::read_notification_sound,
            commands::import_background_image,
            commands::read_background_image,
            commands::open_version_folder,
            commands::reveal_replay,
            commands::read_latest_log,
            commands::webview_engine,
            commands::system_date_pattern,
            commands::report_webview_stall,
            commands::report_event_cost,
            commands::exit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    fn tauri_config() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("valid tauri config")
    }

    fn windows_config() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.windows.conf.json"))
            .expect("valid windows tauri config")
    }

    fn linux_config() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.linux.conf.json"))
            .expect("valid linux tauri config")
    }

    fn capabilities() -> serde_json::Value {
        serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("valid capabilities")
    }

    #[test]
    fn webview_engine_reports_a_platform_the_frontend_knows() {
        let engine = crate::commands::webview_engine();
        assert!(matches!(
            engine.platform,
            "windows" | "macos" | "linux" | "other"
        ));
    }

    /// Also proves the FFI declaration links: a renamed or missing symbol fails
    /// this crate's build on the one platform where the call is compiled in.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_reports_a_plausible_webkitgtk_version() {
        let version = crate::commands::webkit_version().expect("linux must report a version");
        let major: u32 = version
            .split('.')
            .next()
            .and_then(|part| part.parse().ok())
            .expect("major version");
        assert!(major >= 2, "unexpected WebKitGTK version {version}");
    }

    #[test]
    fn package_versions_stay_in_sync() {
        let cargo_version = env!("CARGO_PKG_VERSION");
        let tauri = tauri_config();
        let package: serde_json::Value =
            serde_json::from_str(include_str!("../../package.json")).expect("valid package.json");

        assert_eq!(
            tauri["version"].as_str(),
            Some(cargo_version),
            "tauri.conf.json must match the Cargo workspace version"
        );
        assert_eq!(
            package["version"].as_str(),
            Some(cargo_version),
            "package.json must match the Cargo workspace version"
        );
    }

    #[test]
    fn package_identifier_is_stable_and_release_ready() {
        assert_eq!(
            tauri_config()["identifier"],
            "com.pazdnikoff.faforever-mac",
            "the macOS fork must not install over the official client \
             (`com.faforever.rustclient`), and changing it again needs a migration"
        );
    }

    /// The resource list may only name helpers `prepare:native` really
    /// produces on the platform being built.
    ///
    /// Tauri refuses to bundle when a declared resource path does not exist,
    /// and the list named the bundled JRE unconditionally while
    /// `ensure-java-runtime.mjs` downloads one for Windows alone. Every Linux
    /// release since this workflow was written therefore died at the bundling
    /// step with "resource path `../natives/jre` doesn't exist", which is why
    /// no release has ever carried a Linux asset.
    ///
    /// So the base config names only what both platforms have, and
    /// `tauri.windows.conf.json` adds the runtime. Tauri merges the per
    /// platform file over the base, and merging a map adds keys: a platform
    /// cannot take one away, which is why the base has to be the small one.
    #[test]
    fn only_windows_declares_the_bundled_java_runtime() {
        let base = tauri_config();
        let resources = base["bundle"]["resources"]
            .as_object()
            .expect("native resources must use explicit bundle destinations");

        assert_eq!(resources["../natives/faf-uid*"], "natives/");
        assert_eq!(resources["../natives/faf-pioneer*"], "natives/");
        assert_eq!(
            resources["../natives/java-ice-adapter/faf-ice-adapter.jar"],
            "natives/java-ice-adapter/faf-ice-adapter.jar"
        );
        assert!(
            !resources.contains_key("../natives/jre/"),
            "no JRE is downloaded outside Windows, so naming it here fails the Linux bundle"
        );

        let windows = windows_config();
        let windows = windows["bundle"]["resources"]
            .as_object()
            .expect("windows resource overrides");
        assert_eq!(
            windows["../natives/jre/"], "natives/jre/",
            "the packaged Java resolver expects this directory layout"
        );
    }

    /// Valve's library is a different file on each platform, and the steam
    /// status looks for it under `natives/steam/` by the name the SDK gives
    /// it there. So each platform file bundles its own, with the notice that
    /// says it is not MIT beside it, and the base bundles neither.
    #[test]
    fn each_platform_bundles_its_own_steamworks_library_and_its_notice() {
        let base = tauri_config();
        let base = base["bundle"]["resources"]
            .as_object()
            .expect("base resources");
        assert!(base.keys().all(|source| !source.contains("steamworks")));

        for (config, source, name) in [
            (windows_config(), "win64", "steam_api64.dll"),
            (linux_config(), "linux64", "libsteam_api.so"),
        ] {
            let resources = config["bundle"]["resources"]
                .as_object()
                .expect("platform resources")
                .clone();
            assert_eq!(
                resources[&format!("../vendor/steamworks/{source}/{name}")],
                format!("natives/steam/{name}")
            );
            assert_eq!(
                resources["../vendor/steamworks/README.md"],
                "natives/steam/README.md"
            );
        }
        assert_eq!(
            faf_app::infra::steam_presence::LIBRARY.map(|(name, _)| name),
            if cfg!(windows) {
                Some("steam_api64.dll")
            } else if cfg!(target_os = "linux") {
                Some("libsteam_api.so")
            } else {
                None
            }
        );
    }

    #[test]
    fn production_content_security_policy_keeps_tauri_ipc_and_embeds_scoped() {
        let tauri = tauri_config();
        let csp = tauri["app"]["security"]["csp"]
            .as_object()
            .expect("production CSP must stay enabled");

        assert_eq!(csp["default-src"], "'self'");
        assert_eq!(csp["connect-src"], "ipc: http://ipc.localhost");
        assert_eq!(csp["object-src"], "'none'");
        assert_eq!(csp["frame-ancestors"], "'none'");

        let frames = csp["frame-src"].as_str().expect("frame-src string");
        assert!(frames.contains("https://www.faforever.com"));
        assert!(frames.contains("https://faforever.github.io"));
        assert!(!frames.contains('*'));

        let dev_connect = tauri["app"]["security"]["devCsp"]["connect-src"]
            .as_str()
            .expect("development connect-src string");
        assert!(dev_connect.contains("ipc: http://ipc.localhost"));
        assert!(dev_connect.contains("ws://localhost:5173"));
    }

    fn url(raw: &str) -> tauri::Url {
        tauri::Url::parse(raw).expect("valid url")
    }

    /// The bug this guards: a packaged Windows build loads its own frontend from
    /// `http://tauri.localhost`, and an allow list that only knew the `https`
    /// spelling classified that first navigation as an outbound link. The window
    /// stayed empty and the user's browser opened on `http://tauri.localhost/`.
    #[test]
    fn every_packaged_app_origin_stays_inside_the_window() {
        for origin in [
            "http://tauri.localhost/",
            "http://tauri.localhost/assets/index.js",
            "tauri://localhost/",
            "tauri://localhost/index.html",
            "http://localhost:5173/",
            "http://ipc.localhost/",
            "http://asset.localhost/maps/preview.png",
        ] {
            assert!(
                crate::navigation::is_internal_navigation(&url(origin)),
                "{origin} is the app loading itself, not a link to hand to the browser"
            );
        }
    }

    #[test]
    fn embedded_site_roots_load_in_place_and_anything_else_leaves() {
        for embedded in [
            "https://www.faforever.com/newshub",
            "https://faforever.com/newshub",
            "https://www.faforever.com/dist/main.js",
            "https://faforever.github.io/spooky-db/",
        ] {
            assert!(crate::navigation::is_internal_navigation(&url(embedded)));
        }

        for external in [
            "https://www.youtube.com/watch?v=abc",
            "https://forum.faforever.com/topic/1",
            // A host that merely starts with an origin we trust is not that origin.
            "http://localhost.example.com/",
            "https://tauri.localhost.example.com/",
        ] {
            assert!(
                !crate::navigation::is_internal_navigation(&url(external)),
                "{external} must be opened in the OS browser"
            );
        }
    }

    #[test]
    fn news_hub_unwraps_video_links_before_handing_them_over() {
        assert_eq!(
            crate::navigation::external_target(&url(
                "https://www.faforever.com/newshub/youtube.com/watch?v=abc"
            ))
            .as_deref(),
            Some("https://www.youtube.com/watch?v=abc")
        );
        assert_eq!(
            crate::navigation::external_target(&url(
                "https://www.faforever.com/newshub/youtu.be/abc"
            ))
            .as_deref(),
            Some("https://youtu.be/abc")
        );
        assert_eq!(
            crate::navigation::external_target(&url("https://forum.faforever.com/topic/1"))
                .as_deref(),
            Some("https://forum.faforever.com/topic/1")
        );
    }

    #[test]
    fn only_web_links_are_ever_handed_to_the_operating_system() {
        // The opener starts whatever Windows has registered for a scheme, and
        // the embeds can drive a top-level navigation. A compromised newshub
        // must not be able to reach a protocol handler through this.
        for hostile in [
            "file:///C:/Windows/System32/calc.exe",
            "ms-msdt:/id%20PCWDiagnostic",
            "search-ms:query=passwords",
            "steam://run/9420",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
        ] {
            assert_eq!(
                crate::navigation::external_target(&url(hostile)),
                None,
                "{hostile} must never reach the OS opener"
            );
        }
    }

    #[test]
    fn native_url_opener_is_scoped_to_https() {
        let capabilities = capabilities();
        let permission = capabilities["permissions"]
            .as_array()
            .expect("permissions list")
            .iter()
            .find(|permission| permission["identifier"] == "opener:allow-open-url")
            .expect("scoped opener permission");

        assert_eq!(
            permission["allow"],
            serde_json::json!([{ "url": "https://*" }])
        );
    }
}

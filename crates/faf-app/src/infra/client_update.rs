//! Client self-update against the GitHub releases API.
//!
//! Java's `CheckForBetaUpdateTask` reads the same endpoint; its stable channel
//! instead reads FAF's own remote client configuration, which points at the
//! *Java* client's builds and is therefore no use to us. So both channels come
//! from this repository's releases here, and the prerelease flag is what
//! separates them.
//!
//! ## What is trusted, and what is not
//!
//! This downloads a file and then executes it, so it is worth being precise
//! about the trust chain, which is exactly as long as the Java client's and no
//! longer:
//!
//! - **TLS to GitHub.** Enforced by refusing any download URL that is not
//!   `https://`.
//! - **The configured repository.** A download URL must start with this repo's
//!   release-download prefix, so a compromised or mistaken API response cannot
//!   redirect the installer to an arbitrary host.
//! - **Nothing else.** There is *no signature check*. Anyone who can publish a
//!   release to the configured repository can run code on every client that
//!   accepts the update: which is true of the Java client too, and is the
//!   reason `tauri-plugin-updater` exists. Adopting it needs a signing key that
//!   only a release maintainer can generate, so it is left as a follow-up
//!   rather than faked here.
//!
//! The downloaded file's *name* is never taken from the API. It is built from
//! the release version (already shape-checked by
//! [`faf_domain::state::is_release_version`]) plus one of this module's own
//! [`ASSET_SUFFIXES`], so a hostile asset name cannot influence where the
//! client writes or what it runs.

use async_trait::async_trait;
use faf_domain::state::{
    compare_versions, is_release_version, strip_version_prefix, ClientRelease, ReleaseChannel,
};
use serde_json::Value;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use crate::infra::{cache_dir, env_or};
use crate::ports::{ClientUpdatePort, DownloadProgress};

/// Installer kinds this platform can run, best first.
///
/// Windows gets the NSIS installer ahead of the MSI because it can upgrade in
/// place; the MSI is kept as the fallback because `bundle.targets` is `all` and
/// a release may ship only one of them. macOS and Linux mirror what Tauri
/// bundles for each.
#[cfg(target_os = "windows")]
pub const ASSET_SUFFIXES: [&str; 2] = ["-setup.exe", ".msi"];
#[cfg(target_os = "macos")]
pub const ASSET_SUFFIXES: [&str; 2] = [".dmg", ".app.tar.gz"];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub const ASSET_SUFFIXES: [&str; 3] = [".AppImage", ".deb", ".rpm"];

/// The default source of releases: the macOS fork's own repository.
///
/// Not upstream's: the fork keeps its own identifier and version line, and
/// upstream publishes no macOS build, so its releases are never this client's.
///
/// It has to be the repository's real name, not a readable version of it. This
/// was `FAForeverRustClient/FAForever-Rust-Client`, which GitHub answers with a
/// 404, so every update check failed and no client would ever have been offered
/// a release however well one was published.
const DEFAULT_REPO: &str = "PazdnikOFF/FAForeverRustClient";

/// GitHub rejects unidentified API clients, so this is required, not polite.
/// Thirty releases of GitHub JSON, with room to spare. The API's own page of
/// thirty is well under a megabyte.
const MAX_RELEASE_LIST_BYTES: u64 = 4 * 1024 * 1024;

const USER_AGENT: &str = concat!("faforever-rust-client/", env!("CARGO_PKG_VERSION"));
/// Hard disk/memory safety ceiling for a self-update asset. Current platform
/// bundles are far smaller; a response beyond this is never a legitimate
/// installer for this client.
const MAX_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ClientUpdateConfig {
    /// GitHub API root.
    pub api_base: String,
    /// `owner/name` of the repository whose releases are offered.
    pub repo: String,
    /// A download URL must start with this, or it is refused. Derived from
    /// `repo` unless overridden, which is how a fork or a test points the
    /// download somewhere else *deliberately*.
    pub download_prefix: String,
}

impl ClientUpdateConfig {
    pub fn faf() -> Self {
        let repo = env_or("FAF_CLIENT_UPDATE_REPO", DEFAULT_REPO);
        // Where the client's own installer comes from is not an
        // environment variable in a shipped build: see `dev_env_or`.
        let download_prefix = crate::infra::dev_env_or(
            "FAF_CLIENT_UPDATE_DOWNLOAD_PREFIX",
            format!("https://github.com/{repo}/releases/download/"),
        );
        Self {
            api_base: env_or("FAF_CLIENT_UPDATE_API_BASE", "https://api.github.com"),
            repo,
            download_prefix,
        }
    }
}

pub struct GitHubUpdates {
    config: ClientUpdateConfig,
    http: reqwest::Client,
}

impl GitHubUpdates {
    pub fn new(config: ClientUpdateConfig) -> Self {
        Self {
            config,
            // The installer is executed, so the transport refuses a redirect
            // that leaves HTTPS. `is_trusted` pins the first URL to the
            // release prefix; this is what stops a later hop undoing that.
            http: super::http::https_only_download_client(),
        }
    }

    pub fn faf() -> Self {
        Self::new(ClientUpdateConfig::faf())
    }

    /// Reject a download URL that does not come from the configured repository
    /// over TLS. See the module doc: this is one of the two things standing
    /// between the API response and code execution.
    fn is_trusted(&self, url: &str) -> bool {
        url.starts_with("https://")
            && url.starts_with(&self.config.download_prefix)
            // `..` would be normalised away by the URL parser, escaping the
            // prefix this check just verified.
            && !url.contains("..")
    }

    /// Where an installer for `release` is written.
    ///
    /// Deliberately not derived from the asset name: see the module doc.
    fn installer_path(&self, release: &ClientRelease) -> Result<PathBuf, String> {
        let suffix = ASSET_SUFFIXES
            .iter()
            .find(|suffix| release.asset_name.ends_with(*suffix))
            .ok_or_else(|| {
                format!(
                    "release {} has no installer this platform can run",
                    release.version
                )
            })?;
        if !is_release_version(&release.version) {
            return Err(format!("{} is not a release version", release.version));
        }
        Ok(cache_dir()?
            .join("updates")
            .join(format!("faf-client-{}{suffix}", release.version)))
    }

    async fn fetch_releases(&self) -> Result<Vec<Value>, String> {
        let url = format!(
            "{}/repos/{}/releases?per_page=30",
            self.config.api_base, self.config.repo
        );
        let response = self
            .http
            .get(&url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| {
                format!(
                    "could not reach the release list: {}",
                    crate::infra::http::describe_transport_error(&e)
                )
            })?;

        let status = response.status();
        // Bounded like every other body in the client. This was the one that
        // was not, which is only a memory risk against a compromised GitHub
        // API, but an exception with no reason behind it is the kind that
        // gets copied.
        let body = crate::infra::vault_install::bounded_body(
            response,
            "the release list",
            MAX_RELEASE_LIST_BYTES,
        )
        .await
        .map_err(|e| format!("could not read the release list: {e}"))?;
        let body = String::from_utf8_lossy(&body).into_owned();
        if !status.is_success() {
            return Err(format!(
                "the release list returned {status}: {}",
                body.chars().take(200).collect::<String>()
            ));
        }

        match serde_json::from_str::<Value>(&body) {
            Ok(Value::Array(releases)) => Ok(releases),
            Ok(_) => Err("the release list was not a list".to_string()),
            Err(e) => Err(format!("the release list was not valid JSON: {e}")),
        }
    }

    async fn run_download(
        &self,
        release: &ClientRelease,
        progress: &mpsc::Sender<DownloadProgress>,
    ) -> Result<String, String> {
        if !self.is_trusted(&release.download_url) {
            // Not a "download failed": this is the check refusing to run
            // something that did not come from the configured repository.
            return Err(format!(
                "refusing to download {}: it is not a release asset of {}",
                release.download_url, self.config.repo
            ));
        }

        let target = self.installer_path(release)?;
        let directory = target
            .parent()
            .ok_or_else(|| "could not resolve the download directory".to_string())?;
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|e| format!("could not create the download directory: {e}"))?;
        let partial = target.with_extension("partial");
        // Clean up a file left by a process kill before starting another
        // attempt; completed installers use a different path.
        if tokio::fs::try_exists(&partial).await.unwrap_or(false) {
            tokio::fs::remove_file(&partial)
                .await
                .map_err(|e| format!("could not clear the partial installer: {e}"))?;
        }

        let response = self
            .http
            .get(&release.download_url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "could not reach the installer: {}",
                    crate::infra::http::describe_transport_error(&e)
                )
            })?;
        if !response.status().is_success() {
            return Err(format!(
                "the installer download returned {}",
                response.status()
            ));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_INSTALLER_BYTES)
        {
            return Err("the installer is larger than the allowed download size".into());
        }
        // Clamped for the IPC boundary, which has no 64-bit integer.
        let total_bytes = response
            .content_length()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
            .unwrap_or(0);

        // Written aside and renamed, so an interrupted download can never be
        // mistaken for a complete installer and executed.
        let mut file = tokio::fs::File::create(&partial)
            .await
            .map_err(|e| format!("could not write the installer: {e}"))?;

        let mut received: u64 = 0;
        let mut stream = response.bytes_stream();
        use futures_util::StreamExt as _;
        use tokio::io::AsyncWriteExt as _;
        let write_result: Result<(), String> = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| {
                    format!(
                        "the installer download failed: {}",
                        crate::infra::http::describe_transport_error(&e)
                    )
                })?;
                received = received
                    .checked_add(chunk.len() as u64)
                    .ok_or_else(|| "the installer is too large".to_string())?;
                if received > MAX_INSTALLER_BYTES {
                    return Err("the installer is larger than the allowed download size".into());
                }
                file.write_all(&chunk)
                    .await
                    .map_err(|e| format!("could not write the installer: {e}"))?;
                let _ = progress
                    .send(DownloadProgress::Received {
                        received_bytes: u32::try_from(received).unwrap_or(u32::MAX),
                        total_bytes,
                    })
                    .await;
            }
            file.flush()
                .await
                .map_err(|e| format!("could not finish writing the installer: {e}"))
        }
        .await;
        drop(file);
        if let Err(error) = write_result {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(error);
        }

        if let Err(error) = tokio::fs::rename(&partial, &target).await {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(format!("could not store the installer: {error}"));
        }
        Ok(target.to_string_lossy().into_owned())
    }
}

#[async_trait]
impl ClientUpdatePort for GitHubUpdates {
    async fn latest(&self, channel: ReleaseChannel) -> Result<Option<ClientRelease>, String> {
        let releases = self.fetch_releases().await?;
        Ok(pick_release(&releases, channel))
    }

    async fn download(&self, release: ClientRelease) -> mpsc::Receiver<DownloadProgress> {
        let (tx, rx) = mpsc::channel(32);
        let config = self.config.clone();
        let http = self.http.clone();
        tokio::spawn(async move {
            let client = GitHubUpdates { config, http };
            let outcome = client.run_download(&release, &tx).await;
            let _ = tx.send(DownloadProgress::Finished(outcome)).await;
        });
        rx
    }

    async fn install(&self, path: String) -> Result<(), String> {
        start_installer(Path::new(&path)).await
    }
}

/// Choose the newest usable release from a GitHub `/releases` response.
///
/// Pure so the interesting decisions: draft handling, channel filtering,
/// version ordering, asset selection: are testable without a network.
fn pick_release(releases: &[Value], channel: ReleaseChannel) -> Option<ClientRelease> {
    let mut best: Option<ClientRelease> = None;
    for release in releases {
        // Drafts are visible to authenticated maintainers and are not
        // published; offering one would hand out an unfinished build.
        if release
            .get("draft")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let pre_release = release
            .get("prerelease")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if pre_release && channel == ReleaseChannel::Stable {
            continue;
        }

        let tag = release.get("tag_name").and_then(Value::as_str)?;
        // A repository can carry tags that are not versions at all. Skipping
        // them is what keeps `latest` from becoming "whatever sorts last".
        if !is_release_version(tag) {
            continue;
        }
        let version = strip_version_prefix(tag).to_string();

        let asset = pick_asset(release.get("assets"));
        let candidate = ClientRelease {
            version,
            notes_url: release
                .get("html_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            download_url: asset
                .as_ref()
                .map(|asset| asset.url.clone())
                .unwrap_or_default(),
            asset_name: asset
                .as_ref()
                .map(|asset| asset.name.clone())
                .unwrap_or_default(),
            size_bytes: asset.as_ref().map(|asset| asset.size).unwrap_or(0),
            pre_release,
            published_at: release
                .get("published_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        };

        // Newest by version, not by list order: GitHub returns releases in
        // creation order, and a patch backported to an older line would
        // otherwise outrank the newest one.
        let better = match &best {
            None => true,
            Some(best) => compare_versions(&candidate.version, &best.version) == Ordering::Greater,
        };
        if better {
            best = Some(candidate);
        }
    }
    best
}

struct Asset {
    name: String,
    url: String,
    size: u32,
}

/// The installer for this platform, preferring earlier [`ASSET_SUFFIXES`].
fn pick_asset(assets: Option<&Value>) -> Option<Asset> {
    let assets = assets?.as_array()?;
    for suffix in ASSET_SUFFIXES {
        for asset in assets {
            let name = asset
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !name.ends_with(suffix) {
                continue;
            }
            let url = asset
                .get("browser_download_url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if url.is_empty() {
                continue;
            }
            return Some(Asset {
                name: name.to_string(),
                url: url.to_string(),
                size: asset
                    .get("size")
                    .and_then(Value::as_u64)
                    .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
                    .unwrap_or(0),
            });
        }
    }
    None
}

/// Launch the installer and return; it outlives this process.
///
/// An `.msi` is data, not a program, so it goes through `msiexec`. Everything
/// else is executed directly, matching Java's `ProcessBuilder(command)`.
async fn start_installer(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("{} is no longer there", path.display()));
    }

    #[cfg(unix)]
    {
        // Java does the same via `setUnixExecutableAndWritableBits`: an asset
        // downloaded over HTTP arrives without the executable bit.
        use std::os::unix::fs::PermissionsExt as _;
        let permissions = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(path, permissions)
            .map_err(|e| format!("could not make the installer executable: {e}"))?;
    }

    let mut command = if path.extension().is_some_and(|ext| ext == "msi") {
        let mut command = tokio::process::Command::new("msiexec");
        command.arg("/i").arg(path);
        command
    } else {
        tokio::process::Command::new(path)
    };

    command
        .spawn()
        .map(|_child| ())
        .map_err(|e| format!("could not start the installer: {e}"))
}

/// Reports that the client is current, without asking anyone.
///
/// Used offline and in tests. Deliberately `Ok(None)` rather than an error:
/// a development build should show no update banner at all, and an error would
/// put a failure into Settings that nothing is wrong with.
#[derive(Debug, Clone, Default)]
pub struct FakeClientUpdates;

#[async_trait]
impl ClientUpdatePort for FakeClientUpdates {
    async fn latest(&self, _channel: ReleaseChannel) -> Result<Option<ClientRelease>, String> {
        Ok(None)
    }

    async fn download(&self, _release: ClientRelease) -> mpsc::Receiver<DownloadProgress> {
        let (tx, rx) = mpsc::channel(1);
        let _ = tx
            .send(DownloadProgress::Finished(Err(
                "this build does not download updates".into(),
            )))
            .await;
        rx
    }

    async fn install(&self, _path: String) -> Result<(), String> {
        Err("this build does not install updates".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release whose asset name ends in whatever this platform accepts, so
    /// the tests below exercise the same path on every runner.
    fn asset_name(stem: &str) -> String {
        format!("{stem}{}", ASSET_SUFFIXES[0])
    }

    fn release_json(tag: &str, pre_release: bool, with_asset: bool) -> Value {
        let assets = if with_asset {
            serde_json::json!([{
                "name": asset_name("faf-client"),
                "browser_download_url":
                    format!("https://github.com/Org/Repo/releases/download/{tag}/{}", asset_name("faf-client")),
                "size": 4096,
            }])
        } else {
            serde_json::json!([])
        };
        serde_json::json!({
            "tag_name": tag,
            "prerelease": pre_release,
            "draft": false,
            "html_url": format!("https://github.com/Org/Repo/releases/tag/{tag}"),
            "published_at": "2026-02-01T00:00:00Z",
            "assets": assets,
        })
    }

    fn config() -> ClientUpdateConfig {
        ClientUpdateConfig {
            api_base: "https://api.github.invalid".into(),
            repo: "Org/Repo".into(),
            download_prefix: "https://github.com/Org/Repo/releases/download/".into(),
        }
    }

    #[test]
    fn the_newest_version_wins_regardless_of_list_order() {
        // GitHub returns releases newest-created first, which is not the same
        // as newest-version once a patch lands on an older line.
        let releases = vec![
            release_json("v1.9.1", false, true),
            release_json("v1.10.0", false, true),
            release_json("v1.2.0", false, true),
        ];
        let picked = pick_release(&releases, ReleaseChannel::Stable).expect("a release");
        assert_eq!(picked.version, "1.10.0");
    }

    #[test]
    fn the_stable_channel_never_sees_a_prerelease() {
        let releases = vec![
            release_json("v1.0.0", false, true),
            release_json("v2.0.0-rc1", true, true),
        ];
        assert_eq!(
            pick_release(&releases, ReleaseChannel::Stable)
                .expect("a release")
                .version,
            "1.0.0"
        );
        let beta = pick_release(&releases, ReleaseChannel::PreRelease).expect("a release");
        assert_eq!(beta.version, "2.0.0-rc1");
        assert!(beta.pre_release);
    }

    #[test]
    fn drafts_are_never_offered() {
        // Drafts are unpublished work visible to maintainers.
        let mut draft = release_json("v3.0.0", false, true);
        draft["draft"] = Value::Bool(true);
        let releases = vec![release_json("v1.0.0", false, true), draft];
        assert_eq!(
            pick_release(&releases, ReleaseChannel::PreRelease)
                .expect("a release")
                .version,
            "1.0.0"
        );
    }

    #[test]
    fn tags_that_are_not_versions_are_skipped_rather_than_sorted() {
        let releases = vec![
            release_json("nightly", false, true),
            release_json("latest", false, true),
            release_json("v0.3.0", false, true),
        ];
        assert_eq!(
            pick_release(&releases, ReleaseChannel::Stable)
                .expect("a release")
                .version,
            "0.3.0"
        );
    }

    #[test]
    fn a_release_without_an_asset_for_this_platform_is_still_announced() {
        // Java hides the install button and keeps the release-notes link; the
        // user is told a new version exists either way.
        let releases = vec![release_json("v0.4.0", false, false)];
        let picked = pick_release(&releases, ReleaseChannel::Stable).expect("a release");
        assert_eq!(picked.version, "0.4.0");
        assert!(!picked.is_installable());
        assert!(picked.notes_url.contains("0.4.0"));
    }

    #[test]
    fn an_empty_release_list_is_no_update_rather_than_an_error() {
        assert_eq!(pick_release(&[], ReleaseChannel::Stable), None);
    }

    #[test]
    fn only_release_assets_of_the_configured_repository_are_downloaded() {
        // The load-bearing check: whatever the API says, the installer has to
        // come from this repository over TLS.
        let client = GitHubUpdates::new(config());
        assert!(client
            .is_trusted("https://github.com/Org/Repo/releases/download/v1.0.0/faf-client.msi"));

        for hostile in [
            "http://github.com/Org/Repo/releases/download/v1.0.0/x.msi",
            "https://evil.invalid/Org/Repo/releases/download/v1.0.0/x.msi",
            "https://github.com/Other/Repo/releases/download/v1.0.0/x.msi",
            "https://github.com/Org/Repo/releases/download/../../../../Other/Repo/x.msi",
            "file:///C:/x.msi",
            "",
        ] {
            assert!(!client.is_trusted(hostile), "{hostile} must be refused");
        }
    }

    #[tokio::test]
    async fn an_untrusted_download_url_never_reaches_the_network() {
        // If this reached `reqwest` it would resolve `evil.invalid` and fail
        // with a DNS error instead of the refusal.
        let client = GitHubUpdates::new(config());
        let (tx, _rx) = mpsc::channel(4);
        let release = ClientRelease {
            version: "1.0.0".into(),
            download_url: "https://evil.invalid/installer.exe".into(),
            asset_name: asset_name("faf-client"),
            ..ClientRelease::default()
        };
        let error = client
            .run_download(&release, &tx)
            .await
            .expect_err("must refuse");
        assert!(error.contains("refusing"), "got: {error}");
    }

    #[test]
    fn the_installer_path_comes_from_the_version_not_the_asset_name() {
        // A hostile asset name must not choose where the client writes the
        // file it is about to execute.
        let client = GitHubUpdates::new(config());
        let release = ClientRelease {
            version: "1.2.3".into(),
            asset_name: format!("../../../../evil{}", ASSET_SUFFIXES[0]),
            ..ClientRelease::default()
        };
        let path = client.installer_path(&release).expect("a path");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, format!("faf-client-1.2.3{}", ASSET_SUFFIXES[0]));
        assert!(!path.to_string_lossy().contains(".."));
    }

    #[test]
    fn an_asset_this_platform_cannot_run_has_no_installer_path() {
        let client = GitHubUpdates::new(config());
        let release = ClientRelease {
            version: "1.2.3".into(),
            asset_name: "faf-client.sources.zip".into(),
            ..ClientRelease::default()
        };
        assert!(client.installer_path(&release).is_err());
    }

    #[tokio::test]
    async fn the_fake_reports_no_update_and_refuses_to_install() {
        let fake = FakeClientUpdates;
        assert_eq!(fake.latest(ReleaseChannel::Stable).await, Ok(None));
        assert!(fake.install("whatever".into()).await.is_err());
    }
}

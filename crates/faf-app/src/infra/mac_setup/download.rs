//! A pinned download: streamed to disk, hashed on the way, and only renamed
//! into place once the digest matches.
//!
//! Every file the Mac setup fetches is named here by URL *and* SHA-256, the
//! same rule `scripts/download-verified.mjs` applies at build time. A mirror
//! that starts serving something else fails the check instead of running.

use std::path::Path;
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::mpsc;

use crate::ports::mac_setup::StepUpdate;

/// One file to fetch.
#[derive(Debug, Clone, Copy)]
pub(super) struct Pinned {
    pub url: &'static str,
    pub sha256: &'static str,
    /// Shown in an error, so the player knows which part failed.
    pub label: &'static str,
}

/// How a download's bytes map onto the step's progress bar, so a step made of
/// several downloads fills one bar rather than restarting it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Span {
    pub from_permille: u32,
    pub to_permille: u32,
}

impl Span {
    pub const WHOLE: Span = Span {
        from_permille: 0,
        to_permille: 1000,
    };

    fn at(self, fraction: f64) -> u32 {
        let width = f64::from(self.to_permille.saturating_sub(self.from_permille));
        self.from_permille + (width * fraction.clamp(0.0, 1.0)) as u32
    }
}

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

fn megabytes(bytes: u64) -> u32 {
    u32::try_from(bytes / (1024 * 1024)).unwrap_or(u32::MAX)
}

/// Fetch `file` to `target`, reporting progress on `progress`.
///
/// Skips the network entirely when `target` already holds the right bytes,
/// so a step retried after a later failure does not download again.
pub(super) async fn fetch(
    http: &reqwest::Client,
    file: Pinned,
    target: &Path,
    span: Span,
    progress: &mpsc::Sender<StepUpdate>,
) -> Result<(), String> {
    if target.is_file() && digest_of(target).await.as_deref() == Some(file.sha256) {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let partial = target.with_extension("part");

    let response = http
        .get(file.url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("could not download {}: {error}", file.label))?;
    let total = response.content_length().unwrap_or(0);

    let mut out = tokio::fs::File::create(&partial)
        .await
        .map_err(|error| format!("could not write {}: {error}", partial.display()))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut last_report = Instant::now() - PROGRESS_INTERVAL;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk
            .map_err(|error| format!("the download of {} was interrupted: {error}", file.label))?;
        hasher.update(&chunk);
        out.write_all(&chunk)
            .await
            .map_err(|error| format!("could not write {}: {error}", partial.display()))?;
        received += chunk.len() as u64;
        if last_report.elapsed() >= PROGRESS_INTERVAL {
            last_report = Instant::now();
            let permille = (total > 0).then(|| span.at(received as f64 / total as f64));
            let _ = progress
                .send(StepUpdate::Progress {
                    permille,
                    done_mb: megabytes(received),
                    total_mb: megabytes(total),
                })
                .await;
        }
    }
    out.flush()
        .await
        .map_err(|error| format!("could not write {}: {error}", partial.display()))?;
    drop(out);

    let actual = format!("{:x}", hasher.finalize());
    if actual != file.sha256 {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(format!(
            "{} did not match its published checksum, so it was not used",
            file.label
        ));
    }
    tokio::fs::rename(&partial, target)
        .await
        .map_err(|error| format!("could not move {} into place: {error}", file.label))?;
    Ok(())
}

/// SHA-256 of a file on disk, or `None` when it cannot be read.
pub(super) async fn digest_of(path: &Path) -> Option<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(path).ok()?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).ok()?;
        Some(format!("{:x}", hasher.finalize()))
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_span_maps_a_fraction_into_its_share_of_the_bar() {
        let span = Span {
            from_permille: 200,
            to_permille: 600,
        };
        assert_eq!(span.at(0.0), 200);
        assert_eq!(span.at(0.5), 400);
        assert_eq!(span.at(1.0), 600);
        assert_eq!(span.at(7.0), 600, "never past the end of its share");
    }
}

//! The connectivity runtime: a private Java runtime and FAF's ICE adapter.
//!
//! Every FAF client connects players through the Java `faf-ice-adapter`, and
//! the Go adapter cannot talk to it, so a Mac that wants to play ordinary
//! games and the matchmaker needs the Java one. It is installed here, into the
//! client's own folder, rather than asking anybody to install Java: nothing
//! outside the client sees this runtime, and nothing outside it is used.
//!
//! The adapter's published platform jars carry JavaFX for Windows and Linux
//! only. The `nojfx` jar is the portable one, but it still references JavaFX
//! for its log window and fails at the first log line without it, so the
//! JavaFX modules for Apple silicon come from Maven Central beside it and the
//! launcher puts them on the class path (see `infra::ice_java`).

use tokio::sync::mpsc;

use super::download::{self, Pinned, Span};
use super::{mark, Layout};
use crate::ports::mac_setup::StepUpdate;

/// The marker contents for a finished runtime. Bump with any pin below.
pub(super) const RUNTIME_VERSION: &str = "temurin-25.0.3+9+ice-3.3.14+javafx-21.0.8";

/// Java 25 because the client's map generator needs it too.
const JRE: Pinned = Pinned {
    url: "https://github.com/adoptium/temurin25-binaries/releases/download/jdk-25.0.3%2B9/OpenJDK25U-jre_aarch64_mac_hotspot_25.0.3_9.tar.gz",
    sha256: "287cc80077dc2ffd0e5733ba238f92206a84c26bef33e6881a23c213e4c35af4",
    label: "the Java runtime",
};

const ICE_ADAPTER: Pinned = Pinned {
    url: "https://github.com/FAForever/java-ice-adapter/releases/download/3.3.14/faf-ice-adapter-3.3.14-nojfx.jar",
    sha256: "5d1348f57d29e51c92e5a80380e4cf0dec85f867bdc6f58d8c4e5b5fc01d8281",
    label: "the ICE adapter",
};

const JAVAFX: [(&str, Pinned); 3] = [
    (
        "javafx-base.jar",
        Pinned {
            url: "https://repo1.maven.org/maven2/org/openjfx/javafx-base/21.0.8/javafx-base-21.0.8-mac-aarch64.jar",
            sha256: "79314a670289b8a61303fff1de85b085b74bbd69ab1a4a2c1316ff280522633c",
            label: "JavaFX base",
        },
    ),
    (
        "javafx-graphics.jar",
        Pinned {
            url: "https://repo1.maven.org/maven2/org/openjfx/javafx-graphics/21.0.8/javafx-graphics-21.0.8-mac-aarch64.jar",
            sha256: "08f5a193f885c4375ba33bf1a2f55b481726a87bf2a62c117b63c026ef3e2ef6",
            label: "JavaFX graphics",
        },
    ),
    (
        "javafx-controls.jar",
        Pinned {
            url: "https://repo1.maven.org/maven2/org/openjfx/javafx-controls/21.0.8/javafx-controls-21.0.8-mac-aarch64.jar",
            sha256: "a4f0c0fd4a2d0d19d30b568d0cda4e81a6a007d4ade69c00761075fe0a75c79d",
            label: "JavaFX controls",
        },
    ),
];

pub(super) async fn install(
    layout: &Layout,
    http: &reqwest::Client,
    progress: &mpsc::Sender<StepUpdate>,
) -> Result<(), String> {
    let runtime = layout.runtime_dir();
    let archive = layout.downloads().join("temurin-jre.tar.gz");
    download::fetch(
        http,
        JRE,
        &archive,
        Span {
            from_permille: 0,
            to_permille: 700,
        },
        progress,
    )
    .await?;

    // Downloaded into a sibling and swapped in at the end, so a failure part
    // way leaves either the old runtime or none, never half of one.
    let next = layout.root().join("runtime.next");
    let _ = tokio::fs::remove_dir_all(&next).await;
    download::fetch(
        http,
        ICE_ADAPTER,
        &next.join("ice/faf-ice-adapter.jar"),
        Span {
            from_permille: 700,
            to_permille: 850,
        },
        progress,
    )
    .await?;
    for (index, (name, file)) in JAVAFX.iter().enumerate() {
        let start = 850 + 40 * index as u32;
        download::fetch(
            http,
            *file,
            &next.join("ice/javafx").join(name),
            Span {
                from_permille: start,
                to_permille: start + 40,
            },
            progress,
        )
        .await?;
    }

    // The Temurin archive is a macOS bundle (`jdk-…-jre/Contents/Home`); the
    // runtime proper is the `Home` folder, which is laid out like a JRE on any
    // other platform and is what the launcher looks for.
    let staging = layout.root().join("jre.unpacking");
    super::wine::unpack(&archive, &staging, "the Java runtime").await?;
    let home = std::fs::read_dir(&staging)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("Contents/Home"))
        .find(|home| home.join("bin/java").is_file())
        .ok_or("the Java runtime archive is not laid out as expected")?;
    tokio::fs::rename(&home, next.join("jre"))
        .await
        .map_err(|error| format!("could not move the Java runtime into place: {error}"))?;
    let _ = tokio::fs::remove_dir_all(&staging).await;

    let _ = tokio::fs::remove_dir_all(&runtime).await;
    tokio::fs::rename(&next, &runtime)
        .await
        .map_err(|error| format!("could not move the runtime into place: {error}"))?;
    mark(&runtime, RUNTIME_VERSION)?;
    let _ = tokio::fs::remove_file(&archive).await;
    Ok(())
}

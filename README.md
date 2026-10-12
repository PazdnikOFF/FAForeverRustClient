# FAF for Mac (unofficial)

Play [Forged Alliance Forever](https://www.faforever.com/) on an Apple-silicon Mac.

This is an **unofficial** fork of the
[FAForever Rust client](https://github.com/FAForeverRustClient/FAForeverRustClient)
that adds macOS support: on first launch it installs everything Forged Alliance
needs to run under Wine, then starts games from the lobby like the client does
on Windows.

> **Not supported by the FAForever or FAForeverRustClient teams.** They have not
> reviewed, tested or endorsed this build. Report problems
> [here](https://github.com/PazdnikOFF/FAForeverRustClient/issues), not upstream.
> On Windows or Linux, use the
> [official client](https://github.com/FAForeverRustClient/FAForeverRustClient/releases)
> instead: this fork publishes only the macOS build.

## Requirements

- A Mac with Apple silicon (M1 or later). Intel Macs are not supported.
- macOS 13 Ventura or later.
- Forged Alliance in your Steam library, or an existing copy of the game folder.
- A FAF account.
- Several gigabytes of free disk space for Wine, the game and the runtime.

## Install

1. Download `FAForever.Client_<version>_aarch64.dmg` from
   [Releases](https://github.com/PazdnikOFF/FAForeverRustClient/releases).
2. Open it and drag **FAForever Client** to **Applications**.
3. Open the app. The build is not notarized by Apple, so macOS blocks it the
   first time. Open **System Settings → Privacy & Security**, scroll down and
   click **Open Anyway** next to the message about FAForever Client.

   Only do this for a file downloaded from the Releases page above. GitHub shows
   each file's SHA-256 next to it, and you can compare it with
   `shasum -a 256 <file>`.

## First launch

A setup assistant opens and installs, with progress for each step:

- **Rosetta 2**, through macOS's own administrator prompt, if it is missing.
- **Wine** (wine-crossover 23.7.1-1, the build FAF's own macOS scripts were
  tested on).
- **A Wine prefix** with the DirectX 9 and XACT libraries from Microsoft's June
  2010 DirectX redistributable.
- **The online runtime**: a private Java runtime, FAF's ICE adapter and JavaFX,
  so you can play with people on every other client.
- **The game**, downloaded from Steam with your own account (Steam Guard codes
  and mobile approval work), or taken from a folder already on the Mac.

Every download is pinned to a fixed URL and SHA-256 checksum. Your Steam password
is passed straight to the downloader (SteamRE's DepotDownloader) and is never
saved. If you would rather not enter it, point the assistant at a game folder
you already have.

Everything is installed into one folder:

```
~/Library/Application Support/com.FAForever.FAForever-Client/macOS
```

**Settings → Game & launch → Forged Alliance on This Mac** can show it in
Finder, rebuild the game environment or remove everything.

On the first start the client also applies Mac-friendly game settings once:
shadows, vertical sync, antialiasing and bloom off, the costliest options under
Wine. A small helper brings the game window to the front when a match starts and
keeps the cursor inside it, so the screen edge scrolls the map. **Optimize
Graphics for Mac** in the same settings panel applies these defaults again.

## Updates

The client checks this fork's releases for updates. Releases are currently
published as pre-releases, so turn on **Settings → Updates → Include
pre-releases** to be offered them.

**Upgrading from 0.8.1:** that build used the official client's app identifier
and does not see newer fork releases. Install the new version from Releases,
then delete the old app. Wine, the game and your FAF settings are kept.

## Uninstall

1. In the client, **Settings → Game & launch → Remove Everything…** deletes
   Wine, the prefix, the runtime and the game it installed. Or delete the
   folder above by hand.
2. Delete **FAForever Client** from **Applications**.
3. Optionally delete `~/Library/Application Support/com.FAForever.FAForever-Client`
   for the client's settings, cache and logs.

## What this fork changes

- The macOS setup assistant and game launch through Wine (`crates/faf-app/src/infra/mac_setup`).
- `fa-window.exe` (`natives-src/fa-window`), which hands the focus to the game
  and keeps the cursor in it.
- Its own app identifier (`com.pazdnikoff.faforever-mac`) and version line
  (`2026.10.x`), so it never installs over the official client.
- Updates come from this repository, and only the macOS build is released.
- CI builds and tests on macOS.

Everything else follows upstream and is merged from it regularly.

## Build from source

You need Rust (stable), Node 20+ and pnpm (`corepack enable pnpm`).

```bash
pnpm install
pnpm run tauri dev                                   # run in development
pnpm run tauri build --target aarch64-apple-darwin   # build the .app and .dmg
cargo test                                           # Rust tests
pnpm test                                            # frontend tests
```

Architecture, conventions and the development environment variables are
described upstream: see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md),
[`docs/PROJECT_GUIDE.md`](docs/PROJECT_GUIDE.md) and the
[upstream README](https://github.com/FAForeverRustClient/FAForeverRustClient#readme).

## License

MIT, as upstream. See [LICENSE](LICENSE) and
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

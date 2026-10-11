# FAForever Client

> **macOS fork.** This is a fork of
> [FAForeverRustClient/FAForeverRustClient](https://github.com/FAForeverRustClient/FAForeverRustClient)
> that makes the client a complete, out-of-the-box way to play Forged Alliance
> Forever on an Apple-silicon Mac. On first launch a setup assistant installs
> Rosetta 2, Wine (wine-crossover 23.7.1-1), a game prefix with the DirectX
> runtime, a private Java runtime for FAF's ICE adapter, and Forged Alliance
> itself from your Steam library (Steam Guard supported), all with progress,
> into one client-owned folder. Games then start from the lobby through that
> Wine, with the cursor kept in the game and Mac graphics defaults applied. See [macOS (Apple silicon)](#macos-apple-silicon)
> below. Everything else is upstream, unchanged.

A modular FAF (Supreme Commander: Forged Alliance Forever) client in **Rust + Tauri**,
with a **React** frontend reusing the ForgeMapToolkit design system.

Architecture is the contract: see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
One source of truth in Rust, a fully state-driven UI, no per-tab logic.

## Status

The unidirectional state loop is wired end-to-end and proven:

```
UI dispatch ─▶ Tauri command ─▶ service ─▶ event ─▶ reduce(AppState) ─▶ emit ─▶ store ─▶ UI
```

The vertical slices that established the loop, each still the reference for its pattern:

- **session**: connection-status handshake.
- **auth**: login/logout via the **Port pattern** (`AuthPort`). Real provider `OAuthAuth`
  (FAF Ory Hydra, Authorization Code + PKCE); `FakeAuth` for tests/offline. State-driven
  login ↔ app routing. The login screen can remember the session using an OS-keyring
  refresh token and restore it silently on the next launch.
- **nav**: multi-tab shell; the active tab lives in `AppState` so the backend can drive
  navigation too.
- **settings**: persisted preferences (first: UI `theme`), loaded from disk on startup
  and saved on change via `SettingsPort` → `FileSettings`. The theme is type-safe across
  the boundary (a `Theme` enum) and applied as `<html data-theme>`. Four themes ship:
  `forgeDark` (default), `forgeLight`, `javaClient`, `pythonClient`: each a token set in
  `tokens.css`, so adding a theme touches no component.
- **lobby**: a live open-games list driven by a **streaming port** (`LobbyPort::connect`
  → snapshot stream → `GamesUpdated` events), with explicit `connect`/`disconnect`.
  Real provider `LobbyClient` (FAF lobby WebSocket protocol) is used for normal account
  sessions; `FakeLobby` remains available for offline mode. The connection flow is
  `GET user.faforever.com/lobby/access` → verified `wss://…/?verify=…` URL →
  `ask_session` → `auth` (token + `faf-uid` fingerprint) → `game_info`. The OAuth access
  token reaches the lobby via an in-memory `TokenStore`, never through `AppState`.

Everything since is the same shape, one slice plus service plus port each. 28 slices and 14
tabs today; `ls ui/src/features` and `crates/faf-domain/src/state/mod.rs` are the current
answer, and `docs/PROJECT_GUIDE.md` maps them:

- **play**: custom game browser and hosting, matchmaker with party and map pool vetoes,
  co-op, and Galactic War (a separate application this client only installs and starts).
- **chat**: IRC over a binary WebSocket, with reconnect after a suspend.
- **vault**: maps, mods, replays (vault, local and live spectate), uploads, and the Neroxis
  map generator.
- **community**: tournaments against FAF's own `faf-tournaments` service, leaderboards and
  player cards, clan management, moderation reports and reviews.
- **content**: a training hub over a community catalogue kept in a Git repository, an events
  calendar built the same way, and FAForever/fa's patch notes read in the client.
- **client**: settings with configurable game paths, notifications, diagnostics, and client
  self-update from GitHub releases.
- **six languages** (`en`, `de`, `fr`, `ru`, `es`, `pl`): a missing translation falls back to
  English, and a missing key is a compile error. See `ui/src/i18n/`.

**CI** (`.github/workflows/ci.yml`) runs tests, clippy, frontend linting,
typecheck, build, a bindings-drift check (fails if `ui/src/ipc/bindings.ts` is
stale), and a tokens-only guardrail (fails if any component CSS hardcodes a
hex color instead of a token).

Env toggles for local dev:
- `FAF_FAKE_AUTH=1`: skip the browser login (offline fake auth)
- `FAF_FAKE_LOBBY=1` / `FAF_FAKE_CHAT=1`: keep either live service local while testing
- `FAF_FAKE_ROLES=TOURNAMENT_DIRECTOR`: comma-separated permission roles for the session,
  so role-gated UI can be built and screenshotted without holding the role. It reveals
  controls and authorises nothing: FAF still refuses every privileged call from an account
  that lacks the role. Nothing gates on it today; the tournament service answers who the
  caller is per event instead.
- `TOURNEY_API_BASE=…`: point the tournament tab at a different faf-tournaments
  deployment, such as a local `node server.js`. Defaults to the live service.
- `FAF_REAL_LOBBY=1` / `FAF_REAL_CHAT=1`: legacy flags retained for existing launch scripts;
  live services are now the default for a real account session
- `FAF_ICE_ADAPTER_KIND`: overrides the Settings choice: `java` selects the
  production `faf-ice-adapter` (the default); `go` selects the experimental
  bundled `faf-pioneer` backend.
- `FAF_UID_PATH`: optional path override for the `faf-uid` executable. `pnpm run tauri …`
  prepares the official helper in `natives/` automatically (<https://github.com/FAForever/uid/releases>);
  `FAF_UID_VERSION` can select a different release.
- `FAF_ICE_ADAPTER_JAR`: override the checksum-verified Java ICE adapter bundled by `pnpm tauri`
- `FAF_JAVA_PATH`: override the Java 21+ executable. The client otherwise checks a bundled runtime, the official FAF Client installation, `JAVA_HOME`, then `PATH`.
- `FAF_ICE_ADAPTER_VERSION` and `FAF_ICE_ADAPTER_SHA256`: override the pinned official adapter release together
- `FAF_ICE_ADAPTER_PATH`: optional path override for the bundled `faf-pioneer`
- `FAF_GAME_PATH`: path to `ForgedAlliance.exe` (its folder must hold `init_<mod>.lua`)
- `FAF_CLIENT_VERSION`: client version reported to the lobby (defaults to the crate version)
- `FAF_LOG`: optional `tracing` filter for client diagnostics (for example
  `faf_app=debug,faforever_rust_client_lib=info,warn`); the default records client info and all warnings
- `FAF_USER_API_BASE` / `FAF_API_BASE` / `FAF_HYDRA_BASE` / `FAF_LOBBY_URL`: endpoint overrides (e.g. staging)

Packaged and development builds write daily `faforever-client.*.log` files to
the operating system's application log directory. At most seven files are
retained. Debug builds also mirror the structured log to the terminal; access
tokens, raw adapter output, local paths and network candidates are intentionally
excluded from these records.

## Layout

```
crates/
  faf-domain/   pure state + events + commands + reducer (no IO, no async)
  faf-app/      runtime loop, services, ports, infra
  faf-ipc/      generates ui/src/ipc/bindings.ts from the Rust types
src-tauri/      thin Tauri shell (commands + event forwarding)
ui/             React frontend (ipc bridge, Zustand store, features)
docs/           ARCHITECTURE.md (the contract), PROJECT_GUIDE.md (the map),
                notes/ (dated research, not maintained). See docs/README.md.
```

## macOS (Apple silicon)

Forged Alliance is a 32-bit Windows game, so on a Mac it runs under Wine and
Rosetta 2. The client sets all of that up itself: at first launch a setup
assistant opens and installs, with progress for each part,

- **Rosetta 2**, through macOS's own administrator prompt, when it is missing;
- **Wine**: wine-crossover 23.7.1-1, the build the official FAF client's macOS
  scripts were soak-tested on (Game Porting Toolkit and WineHQ 11 are known
  not to hold a match);
- **a Wine prefix** with the `d3dx9` and XACT libraries from Microsoft's June 2010
  DirectX redistributable, unpacked with the system `tar` (no winetricks, no
  Homebrew);
- **the online runtime**: a private Temurin 25 JRE, FAF's Java ICE adapter and
  JavaFX for Apple silicon, so games connect to players on every other client;
- **the game**, downloaded from Steam with the player's own account (Steam Guard
  codes and mobile approval included) through SteamRE's DepotDownloader, or
  adopted from a folder already on the Mac.

Every download is pinned by URL and SHA-256. Everything lives in one folder,
`~/Library/Application Support/com.FAForever.FAForever-Client/macOS`, which
Settings, Game can reveal in Finder, rebuild or remove; nothing is installed
elsewhere. The Steam password is written to the downloader's standard input
and never stored. After setup the launcher runs the game through that Wine and
prefix by itself; the Linux-style launch wrapper setting still overrides it.

On the first start the client also gives the game Mac defaults, once: shadows,
vertical sync, antialiasing and bloom off, the settings that measured
costliest under Wine on Apple silicon. `fa-window.exe`, a tiny helper built
from `natives-src/fa-window` with `rustc` alone, runs beside the game: it
brings the game's window forward when it starts (macOS no longer lets a
background application take the focus) and keeps the cursor inside the game
while it is active, so the screen edge scrolls the map instead of leading to
another display. Wine runs with MSync. Settings, Game can apply the defaults again.

```bash
pnpm install
pnpm run tauri build --target aarch64-apple-darwin
```

## Develop

Prerequisites: Rust (stable), Node 20+, pnpm (via `corepack enable pnpm`), and the
Tauri prerequisites for your OS (on Windows: WebView2, already present on Win 10/11).
Dependency installation with npm or Yarn is intentionally rejected; use the pnpm
version pinned by `package.json`.

```bash
pnpm install                # frontend deps
pnpm run bindings           # regenerate ui/src/ipc/bindings.ts from Rust
pnpm run tauri dev          # run the app (Vite + Tauri)
```

The Tauri command prepares the platform-specific helpers in the ignored `natives/`
directory, each pinned to a version and checked against a SHA-256 digest before it
is used: `faf-uid` for the anti-smurf fingerprint and `faf-pioneer`, FAF's Go ICE
adapter. Neither is committed.

- `FAF_SKIP_UID_DOWNLOAD=1` / `FAF_SKIP_PIONEER_DOWNLOAD=1` skip a download for an
  offline or test run.
- `FAF_UID_PATH` / `FAF_ICE_ADAPTER_PATH` point at a locally managed binary instead.
- Both are skipped automatically when `FAF_FAKE_AUTH` is set without
  `FAF_REAL_LOBBY`, since neither is reachable in that mode anyway.

Other commands:

```bash
cargo test                  # run Rust tests (reducer + loop)
pnpm run lint               # ESLint, including React Hooks rules
pnpm run typecheck          # tsc over the frontend
pnpm test                   # run frontend tests
pnpm run build              # build the frontend to ui/dist
```

## Conventions

When you add anything, follow [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §8:

- New state → a **slice** + reducer + tests in `faf-domain/state/`.
- New capability → a **command + event(s)** + a **service** in `faf-app/services/`.
- New external system → a **`Port` trait** + an **`infra` impl** + a mock.
- New screen → a **`features/<tab>/`** folder; container selects + dispatches, view uses primitives.
- After changing cross-boundary types, run `pnpm run bindings`.

Never: mutate state outside a reducer; do IO outside `infra`; put logic in a component;
hand-write a cross-boundary type.

CI also enforces a few repository rules that no linter catches, including a ban on
em dashes across the whole tree. They are listed with the commands that reproduce
them in [`docs/PROJECT_GUIDE.md`](docs/PROJECT_GUIDE.md#8-repository-guardrails).

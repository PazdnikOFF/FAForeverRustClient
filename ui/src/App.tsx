// App shell. Owns the single event subscription and startup handshake, then
// routes purely from state: logged in → the active tab, otherwise → Login. No router logic
// beyond selecting a slice (ARCHITECTURE.md §4).

import { useAppBackground } from "./shared/appBackground";
import { useEffect, useRef, useState } from "react";
import { ipc } from "./ipc/client";
import { native } from "./ipc/native";
import { RevisionedMirror } from "./ipc/revisionedMirror";
import { measureEventApply } from "./ipc/eventCost";
import { useAppStore } from "./store/store";
import { t } from "./i18n";
import { LoginView } from "./features/auth/LoginView";
import { AppShell } from "./features/shell/AppShell";
import { CommandErrorBanner } from "./features/shell/CommandErrorBanner";
import { ExitGuard } from "./features/shell/ExitGuard";
import { MacSetupAssistant } from "./features/mac-setup/MacSetupAssistant";
import { StartupView } from "./features/shell/StartupView";
import {
  clearLegacyBrowsingPreferences,
  migrateLegacyBrowsingPreferences,
} from "./shared/browsingPreferences";

/** Never let a zoom failure take the shell down with it; the UI is still usable
 *  at 100%, and there is nothing the user could do about it here anyway. */
async function applyInterfaceScale(scale: number): Promise<void> {
  try {
    await native.setZoom(scale / 100);
  } catch (error) {
    console.warn("could not apply the interface scale", error);
  }
}

function browserStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function App() {
  const auth = useAppStore((s) => s.state.auth);
  const hydrated = useAppStore((s) => s.hydrated);
  const authStatus = auth.status;
  const sessionStatus = useAppStore((s) => s.state.session.status);
  const playerName = auth.player?.name;
  const theme = useAppStore((s) => s.state.settings.theme);
  const appearance = useAppStore((s) => s.state.settings.appearance);
  const browsing = useAppStore((s) => s.state.settings.browsing);
  const friendColor = useAppStore((s) => s.state.settings.chat.nameColors.friends);
  const foeColor = useAppStore((s) => s.state.settings.chat.nameColors.foes);
  const browsingMigrationStarted = useRef(false);
  const [startupError, setStartupError] = useState<string | null>(null);
  const [commandError, setCommandError] = useState<string | null>(null);

  useEffect(() => ipc.onCommandError(setCommandError), []);

  // Project backend-owned appearance preferences once at the document root;
  // feature components remain token-driven and need no preference branches.
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.density = appearance.density;
    document.documentElement.dataset.reducedMotion = String(appearance.reduceMotion);
  }, [appearance.density, appearance.reduceMotion, theme]);

  // The player's own background picture, if one is set (#439).
  useAppBackground(appearance.backgroundImage, appearance.backgroundDim);

  // Project friend and foe color preferences at the document root so token-driven
  // indicators (game tile borders, badges, chat elements) align.
  useEffect(() => {
    if (friendColor && friendColor.trim() !== "") {
      document.documentElement.style.setProperty("--color-friend", friendColor.trim());
    } else {
      document.documentElement.style.removeProperty("--color-friend");
    }
  }, [friendColor]);

  useEffect(() => {
    if (foeColor && foeColor.trim() !== "") {
      document.documentElement.style.setProperty("--color-foe", foeColor.trim());
    } else {
      document.documentElement.style.removeProperty("--color-foe");
    }
  }, [foeColor]);

  // Whole-interface zoom, applied at the webview again (#391). A CSS `zoom` on
  // the root, which this was for a while, scales layout but leaves pointer
  // coordinates, `getBoundingClientRect` and `window.innerWidth` in unscaled
  // pixels, while every length written back and every `vh` is scaled. At 125%
  // that put the map zoom off target, every popover a quarter too far right
  // and down, and the large map preview past both edges of the screen. Webview
  // zoom keeps one coordinate space. Java has no whole-interface scale; it
  // follows the Windows display scale, which the webview does by itself.
  const uiScale = useRef(appearance.uiScale);
  useEffect(() => {
    uiScale.current = appearance.uiScale;
    void applyInterfaceScale(appearance.uiScale);
  }, [appearance.uiScale]);

  // Keep webview layout and DOM container dimensions synchronized with native
  // window resizing. The zoom is applied again once a resize settles: the CSS
  // zoom above was introduced against WebView2 clipping its content after the
  // window changed size, and setting the factor again makes the webview lay
  // itself out against its new bounds.
  useEffect(() => {
    let settle: number | undefined;
    const unlistenPromise = native.onWindowResized(() => {
      window.dispatchEvent(new Event("resize"));
      window.clearTimeout(settle);
      settle = window.setTimeout(() => {
        if (uiScale.current !== 100) void applyInterfaceScale(uiScale.current);
      }, 200);
    });
    return () => {
      window.clearTimeout(settle);
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    // The shell only announces the backend as connected after persisted
    // settings have loaded, so migration never writes over Rust defaults from
    // an early webview snapshot.
    if (!hydrated || sessionStatus !== "connected") return;
    if (browsing.legacyStorageMigrated) {
      // Only clear on a startup where the marker was already present in the
      // hydrated settings. When this session just dispatched the migration,
      // retaining the keys lets a failed disk write retry safely next launch.
      if (!browsingMigrationStarted.current) {
        const storage = browserStorage();
        if (storage) clearLegacyBrowsingPreferences(storage);
      }
      return;
    }
    if (browsingMigrationStarted.current) return;
    browsingMigrationStarted.current = true;
    const storage = browserStorage();
    const patch = storage
      ? migrateLegacyBrowsingPreferences(browsing, storage)
      : { legacyStorageMigrated: true };
    void ipc
      .dispatch({ kind: "Settings", command: { type: "patchBrowsing", payload: { patch } } })
      .catch(() => {
        browsingMigrationStarted.current = false;
      });
  }, [browsing, hydrated, sessionStatus]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    // Held here so the cleanup can stop a recovery retry it has scheduled:
    // without that, StrictMode's discarded first run kept retrying snapshots
    // into a store the second run now owns.
    let mirror: RevisionedMirror | undefined;

    const bootstrap = async () => {
      mirror = new RevisionedMirror(
        (state) => useAppStore.getState().hydrate(state),
        // Timed, so a slow page leaves numbers in the client log: see `eventCost`.
        (event) => measureEventApply(event, (applied) => useAppStore.getState().apply(applied)),
        () => ipc.snapshot(),
        (error) => {
          if (active) {
            setCommandError(t("app.stateSyncFailed", {
              reason: error instanceof Error ? error.message : String(error),
            }));
          }
        },
      );
      // Register before requesting the snapshot. Deltas that race the IPC
      // response are buffered by revision, and lag-recovery snapshots travel
      // on this same ordered channel.
      const current = mirror;
      const stopListening = await ipc.onMessage((message) => current.receive(message));
      // StrictMode's double-invoke runs this effect's cleanup synchronously
      // before this `await` resolves, so `active` can already be false here.
      // Without this check the listener registered above would leak: never
      // assigned to `unlisten`, so the cleanup below can't remove it: and a
      // second one from the effect's real run would double-apply every event.
      if (!active) {
        stopListening();
        return;
      }
      unlisten = stopListening;
      const snapshot = await ipc.snapshot();
      if (!active) return;
      current.replace(snapshot);
    };

    void bootstrap().catch((error: unknown) => {
      if (active) setStartupError(error instanceof Error ? error.message : String(error));
    });

    return () => {
      active = false;
      unlisten?.();
      mirror?.dispose();
    };
  }, []);

  // The reference clients establish lobby and chat sessions as part of account
  // login, rather than waiting for the user to open those tabs. Test mode stays
  // local and keeps its existing on-demand fake connections.
  useEffect(() => {
    if (!hydrated || auth.status !== "loggedIn" || auth.mode !== "account" || !playerName) return;
    ipc.send({ kind: "Lobby", command: { type: "connect" } });
    ipc.send({ kind: "Chat", command: { type: "connect", payload: { username: playerName } } });
  }, [auth.mode, auth.status, hydrated, playerName]);

  useEffect(() => {
    if (!hydrated || auth.status !== "loggedOut") return;
    ipc.send({ kind: "Lobby", command: { type: "disconnect" } });
    ipc.send({ kind: "Chat", command: { type: "disconnect" } });
  }, [auth.status, hydrated]);

  if (startupError) return <StartupView error={startupError} />;
  const content = !hydrated
    ? <StartupView />
    : authStatus === "loggedIn" ? <AppShell /> : <LoginView />;

  return (
    <>
      {content}
      {/* Over the login screen as well as the shell: a Mac starts getting the
          game ready the moment the client opens, signed in or not. */}
      <MacSetupAssistant />
      <ExitGuard />
      {commandError && (
        <CommandErrorBanner message={commandError} onDismiss={() => setCommandError(null)} />
      )}
    </>
  );
}

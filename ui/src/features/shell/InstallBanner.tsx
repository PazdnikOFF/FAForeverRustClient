// Missing-install banner: slides in at the top of the workspace when the
// client cannot find Forged Alliance.
//
// Both reference clients refuse to get this far without a known install (the
// Python client's `validate_game_path`, the Java client's first-run wizard).
// This client is usable without one: chat, the vault, the leaderboard and the
// map/mod browsers are all pure API features: so instead of blocking startup
// it says what is missing and offers the one-click fix.
//
// The state is derived, not configured: the backend stats the paths on startup
// and after every change (see `faf-domain`'s install slice), so the banner also
// catches an install that was moved or uninstalled since it was configured.

import { useState } from "react";
import { ipc } from "../../ipc/client";
import { native } from "../../ipc/native";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { useAppStore } from "../../store/store";
import { setupComplete } from "../../store/reducers/macSetup";
import { t } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";

const openSettings = () =>
  ipc.send({ kind: "Nav", command: { type: "select", payload: { tab: "settings" } } });

const checkAgain = () =>
  ipc.send({ kind: "Settings", command: { type: "checkInstalls" } });

/**
 * Set both installs at once: the common case is one FA folder for both.
 *
 * Not a component, so it uses the standalone `t` rather than the hook: the
 * dialog title is produced once, at click time, and needs the language that is
 * current then.
 */
async function pickInstall() {
  const path = await native.selectFile({
    title: t("shell.install.pickTitle"),
    filters: [{ name: "ForgedAlliance.exe", extensions: ["exe"] }],
  });
  if (!path) return;
  await ipc.settle({ kind: "Settings", command: { type: "setGamePath", payload: { path } } });
  await ipc.dispatch({
    kind: "Settings",
    command: { type: "setReplayGamePath", payload: { path } },
  });
}

const openMacSetup = () =>
  ipc.send({ kind: "MacSetup", command: { type: "showAssistant" } });

export function InstallBanner() {
  const { t } = useTranslation();
  const install = useAppStore((s) => s.state.install);
  const macSetup = useAppStore((s) => s.state.macSetup);
  const [dismissed, setDismissed] = useState(false);

  // On a Mac the client installs the game's whole environment itself, and
  // "locate ForgedAlliance.exe" is the wrong question until it has: the one
  // thing to offer is the assistant that finishes the job.
  const macUnfinished = macSetup.supported && macSetup.checked && !setupComplete(macSetup);
  if (macUnfinished && !dismissed && !macSetup.assistantOpen) {
    return (
      <div className="install-banner" role="status">
        <span className="install-banner-icon" aria-hidden="true">
          <Icon name="download" size={16} />
        </span>
        <div className="install-banner-copy">
          <strong>{t("shell.install.macTitle")}</strong>
          <span className="muted">{t("shell.install.macHint")}</span>
        </div>
        <div className="install-banner-actions">
          <Button variant="primary" onClick={() => void openMacSetup()}>
            {t("shell.install.macOpen")}
          </Button>
          <button
            type="button"
            className="install-banner-close"
            aria-label={t("shell.install.dismiss")}
            title={t("shell.install.dismissTitle")}
            onClick={() => setDismissed(true)}
          >
            <Icon name="close" size={14} />
          </button>
        </div>
      </div>
    );
  }

  // `checked` guards the startup window: before the first stat completes we
  // know nothing, and flashing a warning at every launch would train the user
  // to ignore it.
  const missing = install.checked && !install.gameReady;
  if (!missing || dismissed) return null;

  // A machine that has never run FAF is not missing an install; it has not
  // downloaded one yet, and pressing Play is what downloads it. Saying "not
  // found, go and locate it" there is what sent the reporter of #261 to the
  // Java client and back: the only executable he had to point at was Steam's,
  // which this client cannot use, and the folder it was asking for did not
  // exist until something created it.
  const pending = install.gamePending;

  // Replays can play from a separate install, so a half-configured client gets
  // a narrower message than a completely unconfigured one.
  const replayOnly = install.replayReady;

  return (
    <div className="install-banner" role="status">
      <span className="install-banner-icon" aria-hidden="true">
        <Icon name={pending ? "download" : "lock"} size={16} />
      </span>
      <div className="install-banner-copy">
        <strong>
          {pending
            ? t("shell.install.notDownloadedYet")
            : replayOnly
              ? t("shell.install.noGameConfigured")
              : t("shell.install.notFound")}
        </strong>
        <span className="muted">
          {pending
            ? macSetup.supported
              ? t("shell.install.macPendingHint")
              : t("shell.install.notDownloadedYetHint")
            : replayOnly
              ? t("shell.install.replayOnlyHint")
              : t("shell.install.missingHint")}
        </span>
      </div>
      <div className="install-banner-actions">
        {/* Not the primary action any more when the client is about to do
            this itself: the button is there for somebody who keeps their
            install somewhere else, not for somebody who has to find one. */}
        <Button variant={pending ? "ghost" : "primary"} onClick={() => void pickInstall()}>
          {t("shell.install.locate")}
        </Button>
        <Button onClick={() => void openSettings()}>{t("shell.install.settings")}</Button>
        <Button onClick={() => void checkAgain()} title={t("shell.install.checkAgainTitle")}>
          {t("shell.install.checkAgain")}
        </Button>
        <button
          type="button"
          className="install-banner-close"
          aria-label={t("shell.install.dismiss")}
          title={t("shell.install.dismissTitle")}
          onClick={() => setDismissed(true)}
        >
          <Icon name="close" size={14} />
        </button>
      </div>
    </div>
  );
}

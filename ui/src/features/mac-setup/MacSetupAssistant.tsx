// The Mac setup assistant: one window that takes a Mac from nothing to a game
// that starts, the way a macOS setup assistant does.
//
// Everything here is a projection of the `macSetup` slice. The window opens
// itself at launch when something is missing (the backend decides that and
// starts installing at once), lists each part with its own status and
// progress, and leaves the player exactly one decision: where the game comes
// from. Closing it never stops anything; the work carries on and the install
// banner brings the window back.

import { useState, type FormEvent } from "react";

import type { MacSetupState, SetupStep, StepStatus } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { native } from "../../ipc/native";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { Modal } from "../../design-system/Modal";
import { t as translate } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";
import type { MessageKey } from "../../i18n/catalog/en";
import { useAppStore } from "../../store/store";
import {
  SETUP_STEPS,
  componentsReady,
  setupBusy,
  setupComplete,
  stepStatus,
} from "../../store/reducers/macSetup";
import "./mac-setup.css";

/** What the game needs on disk, in megabytes: the warning threshold. */
const GAME_SPACE_MB = 8 * 1024;

const STEP_TEXT: Record<SetupStep, { title: MessageKey; detail: MessageKey }> = {
  rosetta: { title: "macSetup.step.rosetta.title", detail: "macSetup.step.rosetta.detail" },
  wine: { title: "macSetup.step.wine.title", detail: "macSetup.step.wine.detail" },
  prefix: { title: "macSetup.step.prefix.title", detail: "macSetup.step.prefix.detail" },
  connectivity: {
    title: "macSetup.step.connectivity.title",
    detail: "macSetup.step.connectivity.detail",
  },
  game: { title: "macSetup.step.game.title", detail: "macSetup.step.game.detail" },
};

const send = (command: Parameters<typeof ipc.send>[0]) => void ipc.send(command);
const hide = () => send({ kind: "MacSetup", command: { type: "hideAssistant" } });
const prepare = () => send({ kind: "MacSetup", command: { type: "prepare" } });

async function chooseGameFolder() {
  const path = await native.selectDirectory();
  if (!path) return;
  send({ kind: "MacSetup", command: { type: "useGameFolder", payload: { path } } });
}

function StatusGlyph({ status }: { status: StepStatus }) {
  switch (status.type) {
    case "ready":
      return (
        <span className="mac-setup-glyph mac-setup-glyph-ready" aria-hidden="true">
          <Icon name="check" size={13} />
        </span>
      );
    case "running":
      return <span className="mac-setup-glyph mac-setup-glyph-running" aria-hidden="true" />;
    case "failed":
      return (
        <span className="mac-setup-glyph mac-setup-glyph-failed" aria-hidden="true">
          <Icon name="alert" size={13} />
        </span>
      );
    case "missing":
      return <span className="mac-setup-glyph" aria-hidden="true" />;
  }
}

function statusLabel(status: StepStatus, step: SetupStep): string {
  switch (status.type) {
    case "ready":
      return translate("macSetup.status.ready");
    case "running":
      return step === "game"
        ? translate("macSetup.status.downloading")
        : translate("macSetup.status.running");
    case "failed":
      return translate("macSetup.status.failed");
    case "missing":
      return translate("macSetup.status.waiting");
  }
}

/** The bar and the line under it for the step that is running. */
function Progress({ setup }: { setup: MacSetupState }) {
  const { t } = useTranslation();
  const { permille, doneMb, totalMb } = setup.progress;
  const percent = permille === null ? null : Math.min(100, Math.floor(permille / 10));
  const label =
    totalMb > 0
      ? t("macSetup.progress.mb", { done: doneMb, total: totalMb })
      : percent !== null
        ? t("macSetup.progress.percent", { percent })
        : t("macSetup.progress.working");
  return (
    <div className="mac-setup-progress">
      <div
        className={percent === null ? "mac-setup-bar mac-setup-bar-indeterminate" : "mac-setup-bar"}
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        aria-valuetext={label}
      >
        <span style={percent === null ? undefined : { width: `${percent}%` }} />
      </div>
      <span className="mac-setup-progress-label">{label}</span>
    </div>
  );
}

function StepRow({ setup, step }: { setup: MacSetupState; step: SetupStep }) {
  const { t } = useTranslation();
  const status = stepStatus(setup, step);
  const text = STEP_TEXT[step];
  const running = setup.active === step;
  return (
    <li className={`mac-setup-step mac-setup-step-${status.type}`}>
      <StatusGlyph status={status} />
      <div className="mac-setup-step-copy">
        <div className="mac-setup-step-head">
          <span className="mac-setup-step-title">{t(text.title)}</span>
          <span className="mac-setup-step-status">{statusLabel(status, step)}</span>
        </div>
        <span className="mac-setup-step-detail">{t(text.detail)}</span>
        {running && <Progress setup={setup} />}
        {status.type === "failed" && (
          <p className="mac-setup-step-error" role="alert">
            {status.payload.reason}
          </p>
        )}
      </div>
    </li>
  );
}

/** Where the game comes from: a Steam sign-in, or a folder already here. */
function GameSource({ setup }: { setup: MacSetupState }) {
  const { t } = useTranslation();
  const [username, setUsername] = useState(setup.steamUser);
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const steam = setup.steam;

  const signIn = (event: FormEvent) => {
    event.preventDefault();
    send({
      kind: "MacSetup",
      command: { type: "steamDownload", payload: { username: username.trim(), password } },
    });
    // Never kept longer than the one submission it was typed for.
    setPassword("");
  };

  const submitCode = (event: FormEvent) => {
    event.preventDefault();
    send({ kind: "MacSetup", command: { type: "steamCode", payload: { code: code.trim() } } });
    setCode("");
  };

  const cancel = () => send({ kind: "MacSetup", command: { type: "cancelSteam" } });

  if (steam.type === "needsCode") {
    const kind = steam.payload.kind;
    const prompt =
      kind.type === "authenticator"
        ? t("macSetup.steam.codeAuthenticator")
        : kind.payload.hint !== ""
          ? t("macSetup.steam.codeEmail", { hint: kind.payload.hint })
          : t("macSetup.steam.codeEmailNoHint");
    return (
      <form className="mac-setup-source" onSubmit={submitCode}>
        <p className="mac-setup-source-intro">{prompt}</p>
        {steam.payload.retry && (
          <p className="mac-setup-step-error" role="alert">
            {t("macSetup.steam.codeRetry")}
          </p>
        )}
        <label className="mac-setup-field">
          <span>{t("macSetup.steam.code")}</span>
          <input
            autoFocus
            value={code}
            onChange={(event) => setCode(event.target.value)}
            autoComplete="one-time-code"
            autoCapitalize="characters"
            spellCheck={false}
            maxLength={8}
          />
        </label>
        <div className="mac-setup-source-actions">
          <Button onClick={cancel}>{t("common.cancel")}</Button>
          <Button variant="primary" type="submit" disabled={code.trim() === ""}>
            {t("macSetup.steam.submitCode")}
          </Button>
        </div>
      </form>
    );
  }

  if (steam.type === "awaitingApproval" || steam.type === "signingIn") {
    return (
      <div className="mac-setup-source">
        <p className="mac-setup-source-intro mac-setup-waiting" role="status">
          <span className="mac-setup-glyph mac-setup-glyph-running" aria-hidden="true" />
          {steam.type === "awaitingApproval"
            ? t("macSetup.steam.approve")
            : t("macSetup.steam.signingIn")}
        </p>
        {steam.type === "awaitingApproval" && (
          <p className="mac-setup-source-intro">{t("macSetup.steam.approveHint")}</p>
        )}
        <div className="mac-setup-source-actions">
          {steam.type === "awaitingApproval" && (
            <button
              type="button"
              className="mac-setup-link mac-setup-or"
              onClick={() => send({ kind: "MacSetup", command: { type: "steamUseCode" } })}
            >
              {t("macSetup.steam.useCode")}
            </button>
          )}
          <Button onClick={cancel}>{t("common.cancel")}</Button>
        </div>
      </div>
    );
  }

  if (steam.type === "downloading") {
    return (
      <div className="mac-setup-source">
        <p className="mac-setup-source-intro" role="status">
          {t("macSetup.steam.downloading")}
        </p>
        <div className="mac-setup-source-actions">
          <Button onClick={cancel}>{t("macSetup.steam.cancel")}</Button>
        </div>
      </div>
    );
  }

  const lowSpace = setup.freeMb > 0 && setup.freeMb < GAME_SPACE_MB;
  return (
    <form className="mac-setup-source" onSubmit={signIn}>
      <h3 className="mac-setup-source-title">{t("macSetup.steam.heading")}</h3>
      <p className="mac-setup-source-intro">{t("macSetup.steam.intro")}</p>
      <div className="mac-setup-fields">
        <label className="mac-setup-field">
          <span>{t("macSetup.steam.account")}</span>
          <input
            value={username}
            onChange={(event) => setUsername(event.target.value)}
            autoComplete="username"
            autoCapitalize="off"
            spellCheck={false}
          />
        </label>
        <label className="mac-setup-field">
          <span>{t("macSetup.steam.password")}</span>
          <input
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete="current-password"
          />
        </label>
      </div>
      <p className="mac-setup-note">
        <Icon name="lock" size={12} /> {t("macSetup.steam.privacy")}
      </p>
      {lowSpace && (
        <p className="mac-setup-step-error" role="alert">
          {t("macSetup.steam.lowSpace", { free: Math.floor(setup.freeMb / 1024) })}
        </p>
      )}
      <div className="mac-setup-source-actions">
        <span className="mac-setup-or">
          {t("macSetup.steam.orFolder")}{" "}
          <button type="button" className="mac-setup-link" onClick={() => void chooseGameFolder()}>
            {t("macSetup.steam.chooseFolder")}
          </button>
        </span>
        <Button variant="primary" type="submit" disabled={username.trim() === ""}>
          {t("macSetup.steam.signIn")}
        </Button>
      </div>
    </form>
  );
}

export function MacSetupAssistant() {
  const { t } = useTranslation();
  const setup = useAppStore((s) => s.state.macSetup);

  if (!setup.supported || !setup.assistantOpen) return null;

  const complete = setupComplete(setup);
  const busy = setupBusy(setup);
  const preparing = setup.active !== null && setup.active !== "game";
  const componentFailed = SETUP_STEPS.some(
    (step) => step !== "game" && stepStatus(setup, step).type === "failed",
  );
  const needsComponents = !componentsReady(setup);
  const gameMissing = setup.game.type !== "ready";

  return (
    <Modal className="mac-setup-modal" onClose={hide} ariaLabel={t("macSetup.title")}>
      <header className="mac-setup-head">
        <span className="mac-setup-icon" aria-hidden="true">
          <Icon name={complete ? "check" : "download"} size={22} />
        </span>
        <div>
          <h2 className="mac-setup-title">
            {complete ? t("macSetup.readyTitle") : t("macSetup.title")}
          </h2>
          <p className="mac-setup-subtitle">
            {complete ? t("macSetup.readyBody") : t("macSetup.subtitle")}
          </p>
        </div>
      </header>

      <ol className="mac-setup-steps">
        {SETUP_STEPS.map((step) => (
          <StepRow key={step} setup={setup} step={step} />
        ))}
      </ol>

      {gameMissing && <GameSource setup={setup} />}

      {busy && <p className="mac-setup-note">{t("macSetup.background")}</p>}

      <footer className="mac-setup-footer">
        <Button onClick={() => void native.openClientFolder("macSetup")}>
          <Icon name="folder" size={14} /> {t("macSetup.showInFinder")}
        </Button>
        <div className="mac-setup-footer-actions">
          {complete ? (
            <Button variant="primary" onClick={hide}>
              {t("macSetup.done")}
            </Button>
          ) : (
            <>
              <Button onClick={hide}>{t("macSetup.later")}</Button>
              {/* Not drawn while a part is installing: the bar already says
                  so, and a disabled default button only reads as broken. */}
              {needsComponents && !preparing && (
                <Button variant="primary" onClick={prepare}>
                  {componentFailed ? t("macSetup.retry") : t("macSetup.install")}
                </Button>
              )}
            </>
          )}
        </div>
      </footer>
    </Modal>
  );
}

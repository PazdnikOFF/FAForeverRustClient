import { useState } from "react";
import { Button } from "../../design-system/Button";
import { ConfirmDialog } from "../../design-system/ConfirmDialog";
import { ipc } from "../../ipc/client";
import { native } from "../../ipc/native";
import { useTranslation } from "../../i18n/useTranslation";
import { setupBusy, setupComplete } from "../../store/reducers/macSetup";
import { useAppStore } from "../../store/store";
import { SettingRow } from "./SettingControls";

const command = (
  type: "showAssistant" | "resetPrefix" | "removeEverything" | "optimizeGraphics",
) =>
  ipc.send({ kind: "MacSetup", command: { type } });

/**
 * What the Mac setup installed, and the three things worth doing with it:
 * reopen the assistant, rebuild a prefix that broke, or take it all away.
 *
 * Only on a Mac, which the backend says rather than the user agent: the
 * setup is the backend's, and it reports `supported` once it has looked.
 */
export function MacSetupSettingsSection() {
  const { t } = useTranslation();
  const setup = useAppStore((state) => state.state.macSetup);
  const [confirming, setConfirming] = useState(false);

  if (!setup.supported) return null;
  const busy = setupBusy(setup);

  return (
    <>
      <SettingRow
        label={t("macSetup.manage.title")}
        hint={setupComplete(setup) ? t("macSetup.manage.ready") : t("macSetup.manage.missing")}
      >
        <Button onClick={() => void command("showAssistant")}>{t("macSetup.manage.open")}</Button>
      </SettingRow>
      <SettingRow label={t("macSetup.showInFinder")} hint={setup.rootDir}>
        <Button onClick={() => void native.openClientFolder("macSetup")}>
          {t("macSetup.showInFinder")}
        </Button>
      </SettingRow>
      <SettingRow label={t("macSetup.manage.optimize")} hint={t("macSetup.manage.optimizeHint")}>
        <Button disabled={!setupComplete(setup)} onClick={() => void command("optimizeGraphics")}>
          {t("macSetup.manage.optimize")}
        </Button>
      </SettingRow>
      <SettingRow label={t("macSetup.manage.rebuild")} hint={t("macSetup.manage.rebuildHint")}>
        <Button disabled={busy || setup.wine.type !== "ready"} onClick={() => void command("resetPrefix")}>
          {t("macSetup.manage.rebuild")}
        </Button>
      </SettingRow>
      <SettingRow label={t("macSetup.manage.remove")} hint={t("macSetup.manage.removeHint")}>
        <Button variant="danger" disabled={busy} onClick={() => setConfirming(true)}>
          {t("macSetup.manage.remove")}
        </Button>
      </SettingRow>
      {confirming && (
        <ConfirmDialog
          title={t("macSetup.manage.removeTitle")}
          body={t("macSetup.manage.removeBody")}
          warning={t("macSetup.manage.removeWarning")}
          confirmLabel={t("macSetup.manage.removeConfirm")}
          danger
          onConfirm={() => {
            setConfirming(false);
            void command("removeEverything");
          }}
          onCancel={() => setConfirming(false)}
        />
      )}
    </>
  );
}

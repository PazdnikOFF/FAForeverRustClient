// The settings sidebar, declared once.
//
// One entry per section, in sidebar order. A section is a *kind* of setting,
// not a feature of the client: a path is filed under Paths whichever feature
// put it there, disk under Cache, anything about how the client looks under
// Appearance, a log or a debug window under Diagnostics. A feature earns a
// section of its own only when it is a subsystem with settings that are not
// any other kind (chat, notifications, connectivity, the account, launching
// the game). Without that rule every new setting landed in whichever section
// the person adding it happened to have open.
//
// Deliberately no `labels` array here. The search reads what the rows actually
// rendered (see `settingsSearch`), which cannot drift because there is nothing
// to keep in step; `keywords` only adds the words people type that no label
// on the page uses.

import type { ComponentType } from "react";

import type { IconName } from "../../design-system/Icon";
import type { MessageKey } from "../../i18n";
import type { SettingsSection } from "../../ipc/bindings";
import { AccountSupportSettingsSection } from "./AccountSupportSettingsSection";
import { AppearanceSettingsSection } from "./AppearanceSettingsSection";
import { ChatSettingsSection } from "./ChatSettingsSection";
import { ConnectivitySettingsSection } from "./ConnectivitySettingsSection";
import { DiagnosticsSettingsSection } from "./DiagnosticsSettingsSection";
import { IceDebugWindowsSection, MapGeneratorWindowSection } from "./DebugWindowsSettingsSection";
import { DiscordSettingsSection } from "./DiscordSettingsSection";
import { FoldersSettingsSection } from "./FoldersSettingsSection";
import { GameCacheSettingsSection } from "./GameCacheSettingsSection";
import { GameSettingsSection } from "./GameSettingsSection";
import { MacSetupSettingsSection } from "./MacSetupSettingsSection";
import { GeneralSettingsSection } from "./GeneralSettingsSection";
import { NotificationsSettingsSection } from "./NotificationsSettingsSection";
import { PathsSettingsSection } from "./PathsSettingsSection";
import { SteamSettingsSection } from "./SteamSettingsSection";
import { UpdatesSettingsSection } from "./UpdatesSettingsSection";

/**
 * The section keys are the backend's `SettingsSection`, because the open
 * section is navigation state and lives there. A section added here without
 * a variant on the Rust side fails to typecheck rather than failing to open.
 */
export type SectionKey = SettingsSection;

export interface SectionDef {
  readonly title: MessageKey;
  /** One plain line under the page title saying what is on the page. */
  readonly description: MessageKey;
  /** Extra words the search should match, for terms the labels never use. */
  readonly keywords: MessageKey;
  readonly icon: IconName;
  /**
   * The panels that make up the section, in order. Several sections are two
   * existing panels stacked: one section does not imply one component, and
   * splitting them keeps each panel reading the slice of settings it already
   * reads.
   */
  readonly panels: readonly ComponentType[];
}

export const SECTIONS: Record<SectionKey, SectionDef> = {
  general: {
    title: "settings.page.general.title",
    description: "settings.page.general.description",
    keywords: "settings.section.general.keywords",
    icon: "settings",
    panels: [GeneralSettingsSection, UpdatesSettingsSection],
  },
  appearance: {
    title: "settings.page.appearance.title",
    description: "settings.page.appearance.description",
    keywords: "settings.section.appearance.keywords",
    icon: "eye",
    panels: [AppearanceSettingsSection],
  },
  chat: {
    title: "settings.page.chat.title",
    description: "settings.page.chat.description",
    keywords: "settings.section.chat.keywords",
    icon: "chat",
    panels: [ChatSettingsSection],
  },
  notifications: {
    title: "settings.page.notifications.title",
    description: "settings.page.notifications.description",
    keywords: "settings.section.notifications.keywords",
    icon: "bell",
    panels: [NotificationsSettingsSection],
  },
  account: {
    title: "settings.page.account.title",
    description: "settings.page.account.description",
    keywords: "settings.section.account.keywords",
    icon: "user",
    panels: [AccountSupportSettingsSection, DiscordSettingsSection, SteamSettingsSection],
  },
  game: {
    title: "settings.page.game.title",
    description: "settings.page.game.description",
    keywords: "settings.section.game.keywords",
    icon: "play",
    panels: [MacSetupSettingsSection, GameSettingsSection],
  },
  paths: {
    title: "settings.page.paths.title",
    description: "settings.page.paths.description",
    keywords: "settings.section.paths.keywords",
    icon: "folder",
    panels: [PathsSettingsSection, FoldersSettingsSection],
  },
  cache: {
    title: "settings.page.cache.title",
    description: "settings.page.cache.description",
    keywords: "settings.section.gameCache.keywords",
    icon: "drive",
    panels: [GameCacheSettingsSection],
  },
  connectivity: {
    title: "settings.page.connectivity.title",
    description: "settings.page.connectivity.description",
    keywords: "settings.section.connectivity.keywords",
    icon: "globe",
    panels: [ConnectivitySettingsSection, IceDebugWindowsSection],
  },
  diagnostics: {
    title: "settings.page.diagnostics.title",
    description: "settings.page.diagnostics.description",
    keywords: "settings.section.diagnostics.keywords",
    icon: "activity",
    panels: [DiagnosticsSettingsSection, MapGeneratorWindowSection],
  },
};

export const SECTION_ORDER: readonly SectionKey[] = [
  "general",
  "appearance",
  "chat",
  "notifications",
  "account",
  "game",
  "paths",
  "cache",
  "connectivity",
  "diagnostics",
];

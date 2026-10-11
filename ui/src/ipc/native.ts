// The single bridge for desktop-shell capabilities that are not domain commands.
// Feature code stays unaware of Tauri packages, which keeps it browser-testable
// and gives architecture checks one explicit native boundary to enforce.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, type OpenDialogOptions } from "@tauri-apps/plugin-dialog";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { openUrl } from "@tauri-apps/plugin-opener";

export type LogKind = "game" | "client";

/** Mirrors the arms of `faf_app::infra::client_folder`. */
export type ClientFolder = "maps" | "mods" | "replays" | "vault" | "gameCache" | "gamePrefs" | "macSetup";

/** Mirrors `faf_domain::protocol::log_analysis::LogIssue`. */
export type LogIssue = "gameMinimized" | "soundDriver";

export interface LogPreview {
  fileName: string;
  content: string;
  /**
   * Known problems found in the log. Detected in the backend over the *whole*
   * file; `content` is only the newest 512 KiB, so an issue can be reported
   * that the visible excerpt does not contain.
   */
  issues: LogIssue[];
}

/** Mirrors `WebviewEngine` in the Tauri shell. */
export interface WebviewEngine {
  /** `windows`, `macos`, `linux`, or `other`. */
  platform: string;
  /** WebKitGTK `major.minor.micro`; only Linux has a version worth reporting. */
  webkitVersion: string | null;
}

export const native = {
  openLogFolder(kind: LogKind): Promise<void> {
    return invoke("open_log_folder", { kind });
  },

  /** Reveal one of the client's own directories (or `game.prefs`). */
  openClientFolder(kind: ClientFolder): Promise<void> {
    return invoke("open_client_folder", { kind });
  },

  /**
   * Copy a picked sound file into the client's own sounds directory.
   *
   * Resolves to the name it was stored under, which is what the settings hold.
   * Not always the name that was picked: a collision gets a numbered suffix
   * rather than replacing a sound somebody had already chosen.
   */
  importNotificationSound(path: string): Promise<string> {
    return invoke<string>("import_notification_sound", { path });
  },

  /** The sounds the player has added, by name. */
  listNotificationSounds(): Promise<string[]> {
    return invoke<string[]>("list_notification_sounds");
  },

  /** Copy a picked picture in as the background; resolves to its stored name (#439). */
  importBackgroundImage(path: string): Promise<string> {
    return invoke<string>("import_background_image", { path });
  },

  /** The stored background's bytes, for the page to show. */
  readBackgroundImage(name: string): Promise<number[]> {
    return invoke<number[]>("read_background_image", { name });
  },

  /** One stored sound's bytes, for the page to decode. */
  readNotificationSound(name: string): Promise<number[]> {
    return invoke<number[]>("read_notification_sound", { name });
  },

  // No "remove a sound" here: removal is the backend's
  // `RemoveNotificationSound`, which clears every setting that plays the sound
  // and saves before the file goes. A native delete would skip that.

  /**
   * Where a client folder is, for a file dialog to start in.
   *
   * Resolves the same way `openClientFolder` does. Rejects when the folder
   * cannot be created, which callers treat as "no preference" rather than an
   * error worth showing: a picker in the wrong place still works.
   */
  clientFolderPath(kind: ClientFolder): Promise<string> {
    return invoke<string>("client_folder_path", { kind });
  },

  /** Reveal a specific cached game version folder. */
  openVersionFolder(name: string): Promise<void> {
    return invoke("open_version_folder", { name });
  },

  readLatestLog(kind: LogKind): Promise<LogPreview | null> {
    return invoke<LogPreview | null>("read_latest_log", { kind });
  },

  revealReplay(path: string): Promise<void> {
    return invoke("reveal_replay", { path });
  },

  async selectFile(options: OpenDialogOptions): Promise<string | null> {
    const selected = await open({ ...options, multiple: false });
    return typeof selected === "string" ? selected : null;
  },

  /** Pick a folder rather than a file. Same dialog, `directory` mode. */
  async selectDirectory(defaultPath?: string): Promise<string | null> {
    const selected = await open({ directory: true, multiple: false, defaultPath });
    return typeof selected === "string" ? selected : null;
  },

  openUrl(url: string): Promise<void> {
    return openUrl(url);
  },

  /**
   * The short date pattern of the user's regional format (`dd.MM.yyyy`), or
   * null where the system has none. See `shared/format/systemDate.ts`.
   */
  systemDatePattern(): Promise<string | null> {
    return invoke<string | null>("system_date_pattern");
  },

  /** Which engine the interface is being rendered by. */
  webviewEngine(): Promise<WebviewEngine> {
    return invoke<WebviewEngine>("webview_engine");
  },

  isWindowFocused(): Promise<boolean> {
    return getCurrentWindow().isFocused();
  },

  async ensureNotificationPermission(): Promise<boolean> {
    if (await isPermissionGranted()) return true;
    return (await requestPermission()) === "granted";
  },

  sendNotification(title: string, body: string): void {
    sendNotification({ title, body });
  },

  /**
   * Scale the whole interface, as the browser's own zoom would.
   *
   * Deliberately the webview's zoom rather than a CSS transform: it keeps
   * layout, pointer coordinates and `window.innerWidth` in one coordinate
   * space, so code that mixes `getBoundingClientRect` with viewport dimensions
   * keeps working. Failures are swallowed by the caller, because a webview that
   * refuses to zoom is not a reason to fail startup.
   */
  setZoom(factor: number): Promise<void> {
    return getCurrentWebview().setZoom(factor);
  },

  /**
   * Listen for window close confirmation requests when Forged Alliance is running.
   */
  onRequestExitConfirm(handler: () => void): Promise<() => void> {
    return listen("app://request-exit-confirm", () => handler());
  },

  /**
   * Listen for native window resize events.
   */
  onWindowResized(handler: (size: { width: number; height: number }) => void): Promise<() => void> {
    return getCurrentWindow().onResized((event) => handler(event.payload));
  },

  /** Terminate the application process cleanly. */
  exitApp(): Promise<void> {
    return invoke("exit_app");
  },

  closeWindow(): Promise<void> {
    return invoke("exit_app");
  },
};

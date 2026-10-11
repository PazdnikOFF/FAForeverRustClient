// Frontend mirror of `faf-domain`'s reducer. Pure, immutable, and structurally
// identical to crates/faf-domain/src/reducer.rs: same events, same transitions.
// If you change a slice reducer in Rust, change its twin here (ARCHITECTURE.md §3.6).

import type { AppEvent, AppState } from "../ipc/bindings";
import { reduceChangelog } from "./reducers/changelog";
import { reduceChat } from "./reducers/chat";
import { reduceClan } from "./reducers/clan";
import { reduceClientUpdate } from "./reducers/clientUpdate";
import { reduceCoop } from "./reducers/coop";
import { reduceAuth, reduceInstall, reduceNav, reduceSession, reduceSettings } from "./reducers/core";
import { reduceEvents } from "./reducers/events";
import { reduceGalacticWar } from "./reducers/galacticWar";
import { reduceGuides } from "./reducers/guides";
import { reduceLeaderboard } from "./reducers/leaderboard";
import { reduceLobby } from "./reducers/lobby";
import { reduceMacSetup } from "./reducers/macSetup";
import { reduceMapGenerator } from "./reducers/mapGenerator";
import { reduceMaps } from "./reducers/maps";
import { reduceMods } from "./reducers/mods";
import { reduceNotifications } from "./reducers/notifications";
import { reducePlayerCard } from "./reducers/playerCard";
import { reduceReplays } from "./reducers/replays";
import { reduceReporting } from "./reducers/reporting";
import { reduceReviews } from "./reducers/reviews";
import { reduceSocial } from "./reducers/social";
import { reduceStreams } from "./reducers/streams";
import { reduceTraining } from "./reducers/training";
import { reduceTutorials } from "./reducers/tutorials";
import { reduceUploads } from "./reducers/uploads";
import { reduceTourney } from "./reducers/tourney";

export { isModerator, isPrivateChannel, mentions } from "./reducers/chat";
export { findPlayer, playersByNickname } from "./reducers/social";

export function applyEvent(state: AppState, event: AppEvent): AppState {
  switch (event.kind) {
    case "Session":
      return { ...state, session: reduceSession(state.session, event.event) };
    case "Auth":
      return { ...state, auth: reduceAuth(state.auth, event.event) };
    case "Nav":
      return { ...state, nav: reduceNav(state.nav, event.event) };
    case "Notifications":
      return { ...state, notifications: reduceNotifications(state.notifications, event.event) };
    case "Chat":
      return { ...state, chat: reduceChat(state.chat, event.event) };
    case "Clan":
      return { ...state, clan: reduceClan(state.clan, event.event) };
    case "Changelog":
      return { ...state, changelog: reduceChangelog(state.changelog, event.event) };
    case "Coop":
      return { ...state, coop: reduceCoop(state.coop, event.event) };
    case "Events":
      return { ...state, events: reduceEvents(state.events, event.event) };
    case "MacSetup":
      return { ...state, macSetup: reduceMacSetup(state.macSetup, event.event) };
    case "Install":
      return { ...state, install: reduceInstall(state.install, event.event) };
    case "Social":
      return { ...state, social: reduceSocial(state.social, event.event) };
    case "Streams":
      return { ...state, streams: reduceStreams(state.streams, event.event) };
    case "Lobby":
      return { ...state, lobby: reduceLobby(state.lobby, event.event) };
    case "Replays":
      return { ...state, replays: reduceReplays(state.replays, event.event) };
    case "Maps":
      return { ...state, maps: reduceMaps(state.maps, event.event) };
    case "MapGenerator":
      return { ...state, mapGenerator: reduceMapGenerator(state.mapGenerator, event.event) };
    case "Mods":
      return { ...state, mods: reduceMods(state.mods, event.event) };
    case "Leaderboard":
      return { ...state, leaderboard: reduceLeaderboard(state.leaderboard, event.event) };
    case "PlayerCard":
      return { ...state, playerCard: reducePlayerCard(state.playerCard, event.event) };
    case "Reporting":
      return { ...state, reporting: reduceReporting(state.reporting, event.event) };
    case "Reviews":
      return { ...state, reviews: reduceReviews(state.reviews, event.event) };
    case "Tourney":
      return { ...state, tourney: reduceTourney(state.tourney, event.event) };
    case "Guides":
      return { ...state, guides: reduceGuides(state.guides, event.event) };
    case "Training":
      return { ...state, training: reduceTraining(state.training, event.event) };
    case "Tutorials":
      return { ...state, tutorials: reduceTutorials(state.tutorials, event.event) };
    case "Uploads":
      return { ...state, uploads: reduceUploads(state.uploads, event.event) };
    case "GalacticWar":
      return { ...state, galacticWar: reduceGalacticWar(state.galacticWar, event.event) };
    case "ClientUpdate":
      return { ...state, clientUpdate: reduceClientUpdate(state.clientUpdate, event.event) };
    case "Settings":
      return { ...state, settings: reduceSettings(state.settings, event.event) };
  }
}

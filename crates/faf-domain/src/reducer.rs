//! The reducer: the entire mutation surface of the application.
//!
//! Pure, total, no IO, no async. Dispatches each [`AppEvent`] to the owning
//! slice reducer. To add a slice, add one match arm here (ARCHITECTURE.md §3.3).

use crate::state::{
    auth, changelog, chat, clan, client_update, coop, events, galactic_war, guides, install,
    leaderboard, lobby, mac_setup, map_generator, maps, mods, nav, notifications, player_card,
    replays, reporting, reviews, session, settings, social, streams, tourney, training, tutorials,
    uploads,
};
use crate::{AppEvent, AppState};

pub fn reduce(state: &mut AppState, event: &AppEvent) {
    match event {
        AppEvent::Session(e) => session::reduce(&mut state.session, e),
        AppEvent::Auth(e) => auth::reduce(&mut state.auth, e),
        AppEvent::Nav(e) => nav::reduce(&mut state.nav, e),
        AppEvent::Notifications(e) => notifications::reduce(&mut state.notifications, e),
        AppEvent::Chat(e) => chat::reduce(&mut state.chat, e),
        AppEvent::Clan(e) => clan::reduce(&mut state.clan, e),
        AppEvent::Coop(e) => coop::reduce(&mut state.coop, e),
        AppEvent::Events(e) => events::reduce(&mut state.events, e),
        AppEvent::Lobby(e) => lobby::reduce(&mut state.lobby, e),
        AppEvent::Replays(e) => replays::reduce(&mut state.replays, e),
        AppEvent::Maps(e) => maps::reduce(&mut state.maps, e),
        AppEvent::MapGenerator(e) => map_generator::reduce(&mut state.map_generator, e),
        AppEvent::Mods(e) => mods::reduce(&mut state.mods, e),
        AppEvent::Leaderboard(e) => leaderboard::reduce(&mut state.leaderboard, e),
        AppEvent::PlayerCard(e) => player_card::reduce(&mut state.player_card, e),
        AppEvent::Reporting(e) => reporting::reduce(&mut state.reporting, e),
        AppEvent::Reviews(e) => reviews::reduce(&mut state.reviews, e),
        AppEvent::Social(e) => social::reduce(&mut state.social, e),
        AppEvent::Streams(e) => streams::reduce(&mut state.streams, e),
        AppEvent::Tourney(e) => tourney::reduce(&mut state.tourney, e),
        AppEvent::Training(e) => training::reduce(&mut state.training, e),
        AppEvent::Tutorials(e) => tutorials::reduce(&mut state.tutorials, e),
        AppEvent::Changelog(e) => changelog::reduce(&mut state.changelog, e),
        AppEvent::Uploads(e) => uploads::reduce(&mut state.uploads, e),
        AppEvent::GalacticWar(e) => galactic_war::reduce(&mut state.galactic_war, e),
        AppEvent::Guides(e) => guides::reduce(&mut state.guides, e),
        AppEvent::ClientUpdate(e) => client_update::reduce(&mut state.client_update, e),
        AppEvent::MacSetup(e) => mac_setup::reduce(&mut state.mac_setup, e),
        AppEvent::Install(e) => install::reduce(&mut state.install, e),
        AppEvent::Settings(e) => settings::reduce(&mut state.settings, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ConnectionStatus, SessionEvent};

    #[test]
    fn routes_session_event_to_session_slice() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            &SessionEvent::BackendReady {
                version: "9.9.9".into(),
                offline_auth: false,
            }
            .into(),
        );
        assert_eq!(state.session.status, ConnectionStatus::Connected);
        assert_eq!(state.session.backend_version, "9.9.9");
    }
}

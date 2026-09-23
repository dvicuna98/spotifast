//! SpotSurf: playing the song that is on as a game level.
//!
//! While a song plays, its whole file is fetched through the engine's
//! session, decrypted, and held in memory. Once it is there the player bar's
//! game button lights up; pressing it starts SpotSurf inside the window,
//! handing the song across on the game's standard input. SpotSurf builds the
//! level from the audio itself, and plays it; the app's own player follows it
//! silenced, and hands it the songs that come next.
//!
//! The decrypted audio exists only in memory, here and then in the game's
//! process. Nothing decrypted is written to disk. Only a few songs are held at
//! a time: the one playing and those around it.
//!
//! Finding and starting the game, talking to it, and keeping it on the
//! playing song are SpotSurf's connector's work, shared by any player; what
//! is here is what only this one does: getting the song out of librespot.

use std::io::Read;

use librespot_audio::{AudioDecrypt, AudioFile};
use librespot_core::{Session, SpotifyId, SpotifyUri};
use librespot_metadata::audio::{AudioFileFormat, AudioItem, UniqueFields};
use spotsurf_core::{EncodedTrack, TrackMetadata};

pub use spotsurf_connector::{Follow, Game, GameSession, Playing};

/// Spotify's Ogg files start with a header of their own, which an ordinary
/// Ogg demuxer cannot read past. librespot's player skips the same amount.
const SPOTIFY_OGG_HEADER_END: usize = 0xa7;

/// Ogg Vorbis only, which is what SpotSurf decodes. Best quality first, with
/// each format's data rate in kilobytes a second, which librespot paces its
/// download by.
const FORMATS: [(AudioFileFormat, usize); 3] = [
    (AudioFileFormat::OGG_VORBIS_320, 40),
    (AudioFileFormat::OGG_VORBIS_160, 20),
    (AudioFileFormat::OGG_VORBIS_96, 12),
];

/// Where the game's level for the playing song stands, as the player bar
/// shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// Nothing to play: no song, a podcast, or no playback session.
    #[default]
    Idle,
    /// Fetching the song for `uri`.
    Loading { uri: String },
    /// The song for `uri` is in memory; the game can start at once.
    Ready { uri: String },
    /// The song for `uri` could not be fetched.
    Failed { uri: String, reason: String },
}

impl Stage {
    /// The song this stage is about, if any.
    pub fn uri(&self) -> Option<&str> {
        match self {
            Stage::Idle => None,
            Stage::Loading { uri } | Stage::Ready { uri } | Stage::Failed { uri, .. } => Some(uri),
        }
    }
}

/// Whether a uri names something SpotSurf can play: songs, not podcasts.
pub fn playable(uri: &str) -> bool {
    uri.starts_with("spotify:track:")
}

/// The whole of a song, decrypted, in memory, ready to hand to the game.
pub async fn fetch(session: &Session, uri: &str) -> Result<EncodedTrack, String> {
    let parsed = SpotifyUri::from_uri(uri).map_err(|error| format!("{error}"))?;
    let track_id = SpotifyId::try_from(&parsed).map_err(|error| format!("{error}"))?;
    let item = AudioItem::get_file(session, parsed)
        .await
        .map_err(|error| format!("{error}"))?;

    let (format, rate_kb) = FORMATS
        .into_iter()
        .find(|(format, _)| item.files.contains_key(format))
        .ok_or_else(|| "no Ogg Vorbis version of this song is available".to_string())?;
    let file_id = item.files[&format];

    let file = AudioFile::open(session, file_id, rate_kb * 1024)
        .await
        .map_err(|error| format!("{error}"))?;
    let key = session
        .audio_key()
        .request(track_id, file_id)
        .await
        .map_err(|error| format!("Spotify refused the audio key: {error}"))?;
    // Reading the file through is what makes librespot fetch all of it; the
    // reads block, so they are kept off the runtime's threads.
    let bytes = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
        let mut decrypted = AudioDecrypt::new(Some(key), file);
        let mut all = Vec::new();
        decrypted.read_to_end(&mut all)?;
        Ok(all)
    })
    .await
    .map_err(|error| format!("{error}"))?
    .map_err(|error| format!("{error}"))?;
    let audio = bytes
        .get(SPOTIFY_OGG_HEADER_END..)
        .filter(|audio| !audio.is_empty())
        .ok_or_else(|| "the song's file is empty".to_string())?
        .to_vec();

    let artist = match &item.unique_fields {
        UniqueFields::Track { album_artists, .. } => album_artists.join(", "),
        _ => String::new(),
    };
    Ok(EncodedTrack {
        metadata: TrackMetadata {
            uri: Some(uri.to_string()),
            title: item.name,
            artist,
            duration_ms: Some(u64::from(item.duration_ms)),
        },
        bytes: audio,
        hint: Some("ogg".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_songs_are_playable() {
        assert!(playable("spotify:track:1iNeZGJsoC0D7ZyJTdIbDS"));
        assert!(!playable("spotify:episode:512ojhOuo1ktJprKbVcKyQ"));
        assert!(!playable(""));
    }

    #[test]
    fn a_stage_knows_its_song() {
        assert_eq!(Stage::Idle.uri(), None);
        let uri = "spotify:track:1iNeZGJsoC0D7ZyJTdIbDS".to_string();
        assert_eq!(
            Stage::Loading { uri: uri.clone() }.uri(),
            Some(uri.as_str())
        );
        assert_eq!(
            Stage::Failed {
                uri: uri.clone(),
                reason: "no".into()
            }
            .uri(),
            Some(uri.as_str())
        );
    }
}

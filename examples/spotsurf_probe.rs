//! Diagnostic: fetch one Spotify track in full, decode it in memory and run
//! SpotSurf's analysis on it, to see whether a game map can be built from what
//! librespot hands over — and how long that takes.
//!
//! Uses the account Spotifast already signed in with; sign in and enable
//! playback in Spotifast first. Nothing decrypted is written to disk: the
//! audio exists only in this process's memory, and is dropped on exit.
//!
//!   cargo run --example spotsurf_probe -- spotify:track:<id>

use std::io::Read;
use std::time::Instant;

use anyhow::{Context, bail};
use librespot_audio::{AudioDecrypt, AudioFile};
use librespot_core::{Session, SessionConfig, SpotifyId, SpotifyUri};
use librespot_metadata::audio::{AudioFileFormat, AudioItem, UniqueFields};

use spotifast::credentials::{Grant, Slot, Store};
use spotifast::paths::AppDirs;

use spotsurf_analysis::AnalysisConfig;
use spotsurf_core::progress::ProgressSink;
use spotsurf_core::track::{EncodedTrack, TrackMetadata};
use spotsurf_game::{CubeKind, GameConfig, World};

/// Spotify's Ogg files start with a header of its own, which has to be
/// skipped before an ordinary Ogg demuxer can read them. Same value
/// librespot's player uses.
const SPOTIFY_OGG_HEADER_END: usize = 0xa7;

/// Ogg Vorbis only: those are what SpotSurf decodes. Best quality first, with
/// each format's data rate in kilobytes a second, which librespot wants to
/// pace its download.
const FORMATS: [(AudioFileFormat, usize); 3] = [
    (AudioFileFormat::OGG_VORBIS_320, 40),
    (AudioFileFormat::OGG_VORBIS_160, 20),
    (AudioFileFormat::OGG_VORBIS_96, 12),
];

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let Some(uri) = std::env::args().nth(1) else {
        bail!("usage: cargo run --example spotsurf_probe -- spotify:track:<id>");
    };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(probe(&uri))
}

async fn probe(uri: &str) -> anyhow::Result<()> {
    let store = Store::new(AppDirs::discover());
    let loaded = store.lease(Slot::Playback).load().await?;
    if let Some(warning) = loaded.warning {
        eprintln!("{warning}");
    }
    let Some(Grant::Playback(credentials)) = loaded.grant else {
        bail!("sign in to Spotifast and enable playback first");
    };

    let started = Instant::now();
    // No cache: this probe must not touch the files the app itself keeps.
    let session = Session::new(SessionConfig::default(), None);
    session
        .connect(credentials, false)
        .await
        .context("could not connect to Spotify")?;
    println!(
        "connected                     {:>6} ms",
        started.elapsed().as_millis()
    );

    let uri = SpotifyUri::from_uri(uri).context("not a Spotify track uri")?;
    let track_id = SpotifyId::try_from(&uri)?;
    let item = AudioItem::get_file(&session, uri).await?;
    let artist = match &item.unique_fields {
        UniqueFields::Track { album_artists, .. } => album_artists.join(", "),
        _ => String::new(),
    };
    println!("track                         {} — {}", artist, item.name);

    let Some((format, rate_kb)) = FORMATS
        .into_iter()
        .find(|(format, _)| item.files.contains_key(format))
    else {
        bail!("no Ogg Vorbis version of this track is available");
    };
    let file_id = item.files[&format];
    println!("format                        {format:?}");

    // The whole file, decrypted, in memory. Reading it through is what makes
    // librespot fetch all of it; blocking reads, so off the async threads.
    let fetch_started = Instant::now();
    let file = AudioFile::open(&session, file_id, rate_kb * 1024).await?;
    let key = session
        .audio_key()
        .request(track_id, file_id)
        .await
        .context("Spotify refused the audio key")?;
    let bytes = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
        let mut decrypted = AudioDecrypt::new(Some(key), file);
        let mut all = Vec::new();
        decrypted.read_to_end(&mut all)?;
        Ok(all)
    })
    .await??;
    println!(
        "fetched {:>5.1} MB            {:>6} ms",
        bytes.len() as f64 / 1_048_576.0,
        fetch_started.elapsed().as_millis()
    );
    if bytes.len() <= SPOTIFY_OGG_HEADER_END {
        bail!("the file is too short to hold any audio");
    }

    let analysis_started = Instant::now();
    let encoded = EncodedTrack {
        metadata: TrackMetadata {
            uri: Some(item.uri.clone()),
            title: item.name.clone(),
            artist,
            duration_ms: Some(u64::from(item.duration_ms)),
        },
        bytes: bytes[SPOTIFY_OGG_HEADER_END..].to_vec(),
        hint: Some("ogg".into()),
    };
    drop(bytes);
    let decoded = spotsurf_core::decode(encoded, &ProgressSink::silent())?;
    println!(
        "decoded {:>5.1} s at {} Hz      {:>6} ms",
        decoded.duration().as_secs_f32(),
        decoded.sample_rate,
        analysis_started.elapsed().as_millis()
    );

    let map = spotsurf_analysis::analyze(
        &decoded,
        &AnalysisConfig::default(),
        &ProgressSink::silent(),
    );
    map.validate().context("the beat map came out malformed")?;
    let world = World::from_beatmap(&map, GameConfig::default());
    let hazards = world
        .cubes()
        .iter()
        .filter(|cube| cube.kind == CubeKind::Hazard)
        .count();
    println!(
        "analysed                      {:>6} ms",
        analysis_started.elapsed().as_millis()
    );
    println!();
    println!("onsets                        {}", map.onsets.len());
    match map.bpm {
        Some(bpm) => println!("tempo                         {bpm:.1} bpm"),
        None => println!("tempo                         (no clear beat)"),
    }
    println!(
        "items                         {} ({} grey)",
        world.cubes().len(),
        hazards
    );
    let heights = world.path().centre_line().iter().map(|p| p.y);
    let (low, high) = heights.fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
    println!("height range                  {:.0} units", high - low);
    println!();
    println!(
        "total                         {:>6} ms",
        started.elapsed().as_millis()
    );
    Ok(())
}

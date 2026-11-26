//! Playing a stream to its end: a local file once, a live stream opened
//! again whenever an opening is interrupted, as a TV tunes in again.

mod policy;
mod renewal;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use policy::{Budget, Next, charge, next};
pub use policy::MAX_REOPENINGS;
pub(super) use renewal::PAUSES;
use renewal::Renewal;

use super::Program;
use super::stats::{Failure, Shared};
use crate::live::hls::{self, Live, Patience};
use crate::live::playable::{self, Playable};
use crate::live::{Cause, Interruption, QualityChoice};
use crate::log::log;
use crate::playback::demux::Demux;
use crate::playback::feeder::{self, Ended};
use crate::playback::ffmpeg::Media;
use crate::playback::reader::Reader;
use crate::net::{self, Client};
use crate::playback::{self, OpenBackend, Settings};
use crate::source::Selection;
use crate::source::run::{self, Candidate, Engine, MAX_ANSWER_BYTES, Outcome, Request, Resolved};

/// How long opening a candidate the viewer chose among several insists
/// before playback ends, so that they can choose another: an unreachable
/// host must not hold the screen. 3 network failures one second apart, as
/// Media3's playlists (3 retries; hls.js gives up a manifest after 1 retry,
/// FFmpeg's hls.c does not retry it), or 10 s from the first attempt, the
/// time to first byte hls.js allows a manifest: a server that never answers
/// costs one read timeout, not three. With nothing else to choose, as for a
/// stream the list gives, opening goes on until the network is back, as a
/// TV tunes in again.
const CANDIDATE_PATIENCE: Patience = Patience { attempts: 3, within: Duration::from_secs(10) };

pub(super) fn run(
    program: &Program,
    settings: &Settings,
    open: &OpenBackend,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
) -> Result<(), Failure> {
    let loaded;
    let playable = match program {
        Program::Live(playable) => playable,
        Program::Source(selection) => return play_source(selection, settings, open, stop, shared, quality),
        // A `.stream` file describes a live HLS stream.
        Program::Path(path) if path.extension().is_some_and(|e| e == playable::EXTENSION) => {
            loaded = Playable::load(path).map_err(Failure::Playable)?;
            &loaded
        }
        // Anything else is a local file, which can loop.
        Program::Path(path) => return Ok(play_file(path, settings, open, stop, &shared.playback)?),
    };
    log!("live {}", playable.describe());
    play_stream(playable.clone(), &mut Renewal::again(), None, settings, open, stop, shared, quality)
}

/// Plays an item of a source: the candidate the viewer chose, else the
/// item's first, resolved and played as long as it can be. Mana does not
/// pass to another candidate by itself: the viewer chooses again.
fn play_source(
    selection: &Selection,
    settings: &Settings,
    open: &OpenBackend,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
) -> Result<(), Failure> {
    let (source, item) = (&selection.source, &selection.item);
    log!("source {}: {}", source.id, item.fields.get("title").map_or("", String::as_str));
    let client = Client::new().map_err(|e| Failure::Live(Interruption::Failed(Cause::Client(e))))?;
    let mut engine = engine(selection, &client, stop);
    let candidate = match &selection.candidate {
        Some(candidate) => candidate.clone(),
        None => {
            let Some(found) = unless_cancelled(engine.sources(item))? else { return Ok(()) };
            found.into_iter().next().expect("sources gives at least one candidate")
        }
    };
    let Some(resolved) = unless_cancelled(engine.resolve(&candidate, item))? else { return Ok(()) };
    log!("candidate {} resolved: {}", candidate.id().unwrap_or(""), net::loggable(&resolved.url));
    // A candidate chosen among several gives way to the viewer's choice.
    let patience = selection.candidate.is_some().then_some(CANDIDATE_PATIENCE);
    let reread = source.sources.is_some();
    let mut renew = || renewed(&mut engine, &candidate, item, reread);
    play_stream(playable(resolved), &mut Renewal::resolve(&mut renew), patience, settings, open, stop, shared, quality)
}

/// What a run of the engine gave, None when the viewing stopped meanwhile.
fn unless_cancelled<T>(result: Result<T, run::Failure>) -> Result<Option<T>, Failure> {
    match result {
        Err(failure) if failure.outcome == Outcome::Cancelled => Ok(None),
        other => other.map(Some).map_err(Failure::Source),
    }
}

/// An engine for `selection`'s source, on `client`, given up once `stop`.
pub(super) fn engine<'a>(selection: &'a Selection, client: &'a Client, stop: &'a AtomicBool) -> Engine<'a> {
    let fetch = |request: &Request| client.send(request.method, &request.url, &request.headers, request.body.as_deref(), MAX_ANSWER_BYTES);
    Engine::new(&selection.source, &selection.device, fetch, || stop.load(Ordering::Relaxed))
}

/// `candidate` resolved again, its stream having been refused: as the
/// source gives it now, its item's candidates read again first when the
/// source has them (`reread`), since an event's signals may change, and a
/// signal whose stream the resolution only takes from them would otherwise
/// come back the same.
fn renewed(engine: &mut Engine, candidate: &Candidate, item: &crate::source::Item, reread: bool) -> Result<Playable, Failure> {
    let current;
    let candidate = if reread {
        current = still_offered(engine.sources(item).map_err(Failure::Source)?, candidate)?;
        &current
    } else {
        candidate
    };
    let resolved = engine.resolve(candidate, item).map_err(Failure::Source)?;
    log!("stream resolved again: {}", net::loggable(&resolved.url));
    Ok(playable(resolved))
}

/// `candidate` as `found`, the item's candidates read again, has it: the
/// item no longer offering it ends playback, and the viewer chooses again.
fn still_offered(found: Vec<Candidate>, candidate: &Candidate) -> Result<Candidate, Failure> {
    found.into_iter().find(|c| c.id() == candidate.id()).ok_or(Failure::Withdrawn)
}

fn playable(resolved: Resolved) -> Playable {
    Playable::direct(resolved.url, resolved.headers)
}

/// Opens the live `playable` again and again, as `charge` allows, until it
/// ends or the viewing stops; `renewal` gives it anew when it is refused;
/// each opening gives up within `patience`, if given.
fn play_stream(
    playable: Playable,
    renewal: &mut Renewal,
    patience: Option<Patience>,
    settings: &Settings,
    open: &OpenBackend,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
) -> Result<(), Failure> {
    let mut budget = Budget::default();
    let mut playable = playable;
    let mut live = resolve(&mut playable, renewal, patience, stop, shared, quality, &mut budget)?;
    while let Some(opened) = &live {
        let before = presented(shared);
        let outcome = play_opening(opened, open, settings, stop, shared);
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if presented(shared) != before {
            renewal.played();
        }
        let cause = match charge(outcome, &mut budget) {
            Next::End(result) => return result,
            Next::Reopen(_) => continue,
            Next::Resolve(cause) => cause,
        };
        let Some(renewed) = renewal.renew(&playable, cause, &mut budget, stop)? else { return Ok(()) };
        playable = renewed;
        live = resolve(&mut playable, renewal, patience, stop, shared, quality, &mut budget)?;
    }
    Ok(())
}

/// Plays one opening of a live stream until it ends, and tells what to do
/// next.
fn play_opening(opened: &Live, open: &OpenBackend, settings: &Settings, stop: &Arc<AtomicBool>, shared: &Shared) -> Next {
    let stream_stop = hls::Stop::new(stop);
    let ended = play_live(opened, &stream_stop, open, settings, stop, &shared.playback);
    // The downloaders of this opening end with it, also when it failed to
    // open.
    stream_stop.set();
    next(stream_stop.interruption(), ended)
}

/// Tells what `playable` serves: None once stopped meanwhile. A refusal,
/// as when its token expired already, has it given anew by `renewal`.
fn resolve(
    playable: &mut Playable,
    renewal: &mut Renewal,
    patience: Option<Patience>,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
    budget: &mut Budget,
) -> Result<Option<Live>, Failure> {
    loop {
        let cause = match Live::resolve(playable, &hls::Stop::resolving(stop), shared.live.clone(), quality, patience) {
            Ok(live) => return Ok(live),
            Err(Interruption::Refused(cause)) => cause,
            Err(other) => return Err(Failure::Live(other)),
        };
        let Some(renewed) = renewal.renew(playable, cause, budget, stop)? else { return Ok(None) };
        *playable = renewed;
    }
}

/// Where the decoder has presented, to tell whether an opening played.
fn presented(shared: &Shared) -> Option<i64> {
    shared.playback.lock().unwrap().presented_ms
}

/// Plays a local file to its end, again and again when the settings loop it.
fn play_file(
    path: &Path,
    settings: &Settings,
    open: &OpenBackend,
    stop: &AtomicBool,
    stats: &Mutex<playback::Stats>,
) -> Result<(), playback::Error> {
    let media = Media::open(path).map_err(playback::Error::Demuxer)?;
    let reader = Reader::start(Demux::new(media, None), settings.loop_playback)?;
    let ended = feeder::play(&reader, false, open, settings, stop, stats);
    reader.finish();
    ended?;
    Ok(())
}

fn play_live(
    live: &Live,
    stream_stop: &hls::Stop,
    open: &OpenBackend,
    settings: &Settings,
    stop: &AtomicBool,
    stats: &Mutex<playback::Stats>,
) -> Result<Ended, playback::Error> {
    let streams = live.open(stream_stop).map_err(|e| playback::Error::Thread(e.to_string()))?;
    let main = Media::open_stream(Box::new(streams.main)).map_err(playback::Error::Demuxer)?;
    let audio = streams.audio.map(|a| Media::open_stream(Box::new(a))).transpose().map_err(playback::Error::Demuxer)?;
    let reader = Reader::start(Demux::new(main, audio), false)?;
    let ended = feeder::play(&reader, true, open, settings, stop, stats);
    // The reader may be waiting on the downloads for data: they stop first.
    stream_stop.set();
    reader.finish();
    ended
}

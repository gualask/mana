//! End-to-end: the app's path, from the channel list to the decoder, against
//! the synthetic lab (dev/lab) running as a process: the list read as the
//! app reads it, a source's folder, the signals of an event, the session
//! playing on a fake backend. A test binary of its own, so that the lab's
//! encoders do not slow down the core's timed tests, which cargo runs before.

mod decoder;
mod harness;

use std::time::{Duration, Instant};

use harness::{Lab, candidates, channel, ended, folder, selection, within};
use mana_core::catalog::Play;
use mana_core::playback::{AudioCodec, Settings, VideoCodec};
use mana_core::session::{Program, Session, State, chosen_at_once};
use mana_core::source::Selection;
use mana_core::source::run::Outcome;

/// The lab's clip lasts 4 s: once played, ended long before this.
const CLIP_LIMIT: Duration = Duration::from_secs(30);

#[test]
fn an_event_s_signals_play_as_the_viewer_chooses_them() {
    let lab = Lab::start(&[]);
    let entries = lab.list(&format!("{}\n[[include]]\nsource = \"lab-events\"\n", lab.example("lab-events")));
    let events = folder(&lab, &entries);
    let names: Vec<_> = events.iter().map(|e| e.name.as_deref()).collect();
    assert_eq!(names, [Some("Synthetic bars and tone"), Some("Empty event")]);

    // The event has two signals: the viewer chooses.
    let demo = selection(&events[0]);
    let signals = candidates(&demo).unwrap();
    let labels: Vec<_> = signals.iter().enumerate().map(|(i, s)| s.label(i)).collect();
    assert_eq!(labels, ["Deliberate access denial", "Synthetic encrypted link"]);
    assert!(chosen_at_once(&signals).is_none());
    let chosen = |index: usize| Program::Source(Selection { candidate: Some(signals[index].clone()), ..demo.clone() });

    // The first one's media is refused: playback ends saying so, the
    // decoder never loaded, and Mana passes to no other signal by itself.
    let (open, fed) = decoder::open();
    let mut session = Session::new(open);
    session.play("Synthetic bars and tone".into(), chosen(0), Settings::default()).unwrap();
    let state = ended(&session, CLIP_LIMIT).to_string();
    assert!(state.contains("HTTP 403 for /denied.m3u8"), "{state}");
    assert!(fed.lock().unwrap().loads.is_empty());

    // The viewer chooses the other: its sealed envelope leads to the signed
    // clip, which plays to its end with video and audio.
    session.play("Synthetic bars and tone".into(), chosen(1), Settings::default()).unwrap();
    assert_eq!(ended(&session, CLIP_LIMIT), State::Stopped);
    {
        let fed = fed.lock().unwrap();
        assert_eq!(fed.loads.len(), 1, "{:?}", fed.loads);
        let formats = fed.loads[0];
        assert_eq!((formats.video, formats.width, formats.height, formats.audio), (VideoCodec::H264, 320, 180, Some(AudioCodec::Aac)));
        let span = fed.video.iter().max().unwrap() - fed.video.iter().min().unwrap();
        assert!(span >= 3_800_000_000, "{span} ns of video");
        assert!(!fed.audio.is_empty());
    }

    // An event without signals says so, before and while playing.
    let empty = selection(&events[1]);
    assert_eq!(candidates(&empty).unwrap_err().outcome, Outcome::NoSources);
    session.play("Empty event".into(), Program::Source(empty), Settings::default()).unwrap();
    assert_eq!(ended(&session, CLIP_LIMIT).to_string(), "Error: sources: no streams for now");
    assert_eq!(fed.lock().unwrap().loads.len(), 1);
}

/// A channel whose resolution answers 2 s late, and the lab's clip.
const SLOW_AND_CLIP: &str = r#"
[sources.slow]
[[sources.slow.resolve.steps]]
id = "answer"
op = "http"
url = "http://127.0.0.1:8781/faults/slow?seconds=2"
[[sources.slow.resolve.steps]]
id = "stream"
op = "stream"
url = "{answer.json.url}"

[[channels]]
name = "Slow"
source = "slow"
id = "slow"

[[channels]]
name = "Clip"
url = "http://127.0.0.1:8783/public/master.m3u8"
"#;

#[test]
fn changing_channel_gives_up_a_resolution_that_comes_late() {
    let lab = Lab::start(&[]);
    let entries = lab.list(SLOW_AND_CLIP);
    let (open, fed) = decoder::open();
    let mut session = Session::new(open);
    session.play("Slow".into(), Program::Source(selection(channel(&entries, "Slow"))), Settings::default()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let Ok(Play::Stream(clip)) = channel(&entries, "Clip").play.clone() else { panic!("the clip is a stream") };
    let changed = Instant::now();
    session.play("Clip".into(), Program::Live(clip), Settings::default()).unwrap();
    assert!(changed.elapsed() < Duration::from_millis(100), "{:?}", changed.elapsed());
    assert_eq!(ended(&session, CLIP_LIMIT), State::Stopped);
    // The slow answer came meanwhile, to a viewing stopped: it opened no
    // decoder, before the clip's or after it.
    within(Duration::from_secs(10), || (!session.is_stopping()).then_some(()));
    assert_eq!(session.stats().unwrap().name, "Clip");
    assert_eq!(fed.lock().unwrap().loads.len(), 1);
}

#[test]
fn a_token_refused_while_playing_is_resolved_again_and_playback_goes_on() {
    let lab = Lab::start(&["live"]);
    let list = format!("{}\n[[channels]]\nname = \"Live\"\nsource = \"lab-live\"\nid = \"live\"\n", lab.example("lab-live"));
    let entries = lab.list(&list);
    let (open, _) = decoder::open();
    let mut session = Session::new(open);
    session.play("Live".into(), Program::Source(selection(channel(&entries, "Live"))), Settings::default()).unwrap();
    // Where the decoder has presented, while the viewing goes on.
    let presented = || {
        let stats = session.stats().unwrap();
        assert!(!stats.state.ended(), "{}", stats.state);
        (stats.state == State::Playing).then_some(stats.playback.presented_ms).flatten()
    };
    within(Duration::from_secs(30), presented);
    let resolved = || lab.api("/control/live-stats", None)["resolved"].as_u64().unwrap();
    assert_eq!(resolved(), 1);

    // The CDN's token changes, as when it expires: the next playlist is
    // refused, and the source gives the stream again with the new one.
    lab.api("/control/live-token", Some("{}"));
    within(Duration::from_secs(20), || (resolved() == 2).then_some(()));
    // The timeline goes on from there, in real time.
    let renewed = within(Duration::from_secs(10), presented);
    within(Duration::from_secs(30), || presented().filter(|&at| at >= renewed + 6000));
    assert_eq!(resolved(), 2);
    session.stop();
}

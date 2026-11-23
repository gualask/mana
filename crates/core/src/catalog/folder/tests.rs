use std::time::{Duration, Instant};

use super::*;
use crate::catalog::list::{self, Entry};
use crate::catalog::update::tests::{answer, data_dir, serve, with_catalogue};
use crate::source::run::Outcome;

const EVENTS: &str = r#"[
  {"id": "late", "title": "Late", "time": "2026-11-23 22:00:00", "signals": [1]},
  {"id": "none", "title": "No signals", "time": "2026-11-23 18:00:00", "signals": []},
  {"id": "early", "title": "Early", "time": "2026-11-23 20:30:00", "signals": [1, 2]}
]"#;

/// The folder of the events source served at `address`.
fn folder(address: &str) -> Folder {
    let list = list::parse(&with_catalogue(address, &[]), || Some("d-1".into())).unwrap();
    let Some(Entry::Folder(folder)) = list.entries.into_iter().next() else { panic!("no folder") };
    folder
}

/// The next answer of a source, once it came.
fn answered(folders: &mut Folders) -> Update {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(update) = folders.poll() {
            return update;
        }
        assert!(Instant::now() < deadline, "no answer");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn names(channels: &[Channel]) -> Vec<&str> {
    channels.iter().map(|c| c.name.as_deref().unwrap_or("")).collect()
}

#[test]
fn opening_a_folder_reads_its_catalogue_events_with_signals_by_time() {
    let (address, answers) = serve();
    answer(&answers, "/events.json", 200, EVENTS);
    let (dir, folder) = (data_dir("folder"), folder(&address));
    let mut folders = Folders::new(&dir);
    assert_eq!(folders.open(&folder), None, "no copy yet");
    assert!(folders.asking("events"));
    let channels = answered(&mut folders).outcome.unwrap();
    assert!(!folders.asking("events"));
    assert_eq!(names(&channels), ["Early", "Late"]);
    assert_eq!(channels[0].starts_at, crate::source::date::instant("2026-11-23T12:30:00Z"));
    let Ok(Play::Source(selection)) = &channels[0].play else { panic!("{:?}", channels[0].play) };
    assert_eq!((selection.item.fields["id"].as_str(), selection.device.as_str()), ("early", "d-1"));
    // Opened again within AGE: the copy, without asking.
    assert_eq!(names(&folders.open(&folder).unwrap()), ["Early", "Late"]);
    assert!(!folders.asking("events"));
}

#[test]
fn a_copy_shows_at_once_and_is_read_again_once_old_or_from_an_earlier_start() {
    let (address, answers) = serve();
    answer(&answers, "/events.json", 200, EVENTS);
    let (dir, folder) = (data_dir("folder-age"), folder(&address));
    let mut folders = Folders::new(&dir);
    folders.open(&folder);
    answered(&mut folders).outcome.unwrap();
    // The next start shows the copy, and asks the source.
    let mut folders = Folders::new(&dir);
    assert_eq!(names(&folders.open(&folder).unwrap()), ["Early", "Late"]);
    assert!(folders.asking("events"));
    assert!(!folders.refresh(&folder), "asked once at a time");
    answered(&mut folders).outcome.unwrap();
    assert!(!folders.refresh(&folder), "fresh");
    folders.age = Duration::ZERO;
    assert!(folders.refresh(&folder), "old");
}

#[test]
fn a_source_that_fails_says_why_and_leaves_the_copy() {
    let (address, answers) = serve();
    answer(&answers, "/events.json", 200, EVENTS);
    let (dir, folder) = (data_dir("folder-failure"), folder(&address));
    let mut folders = Folders::new(&dir);
    folders.open(&folder);
    answered(&mut folders).outcome.unwrap();
    answer(&answers, "/events.json", 503, "");
    folders.age = Duration::ZERO;
    assert!(folders.refresh(&folder));
    let failure = answered(&mut folders).outcome.unwrap_err();
    assert_eq!((failure.outcome, failure.at.as_str()), (Outcome::NetworkError, "catalogue.page"));
    assert_eq!(names(&folders.open(&folder).unwrap()), ["Early", "Late"]);
}

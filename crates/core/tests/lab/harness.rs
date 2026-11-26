//! The synthetic lab (dev/lab) as a process of its own, on ports the system
//! assigns, and the channel list the tests give the app: the sources of
//! config/examples with the lab's addresses in place of the fixed ones.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mana_core::catalog::folder::Folders;
use mana_core::catalog::list::{self, Entry};
use mana_core::catalog::{Channel, Play};
use mana_core::session::{Candidates, Session, State};
use mana_core::source::Selection;
use mana_core::source::run::{Candidate, Failure};

/// The addresses the examples are written with (docs/local-lab.md).
const FIXED: [(&str, &str); 3] =
    [("api", "http://127.0.0.1:8781"), ("resolver", "http://127.0.0.1:8782"), ("media", "http://127.0.0.1:8783")];
/// Starting takes a few seconds, more with live channels, which wait for
/// their first segments.
const START_LIMIT: Duration = Duration::from_secs(90);

/// The lab while it runs; dropping it stops it.
pub struct Lab {
    child: Child,
    /// Role (`api`, `resolver`, `media`) and origin.
    pub origins: Vec<(String, String)>,
    /// Where the lab writes its fixture manifest, and the app its data.
    pub dir: PathBuf,
}

impl Lab {
    /// Starts the lab, with the live channels `live` (none: no live).
    pub fn start(live: &[&str]) -> Self {
        static STARTED: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::temp_dir().join(format!("mana-e2e-{}-{}", std::process::id(), STARTED.fetch_add(1, Ordering::Relaxed)));
        let mut command = Command::new("python3");
        command.args(["-B", "-m", "dev.lab", "--until-stdin-closes", "--write-config"]).arg(dir.join("lab"));
        if !live.is_empty() {
            command.arg("--live").args(live);
        }
        command.current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        let mut child = command.spawn().expect("python3 runs the lab (docs/local-lab.md: requirements)");
        let stdout = child.stdout.take().unwrap();
        let (sender, summary) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = sender.send(line);
        });
        let line = summary.recv_timeout(START_LIMIT).expect("the lab starts");
        assert!(!line.is_empty(), "the lab ended without starting: its error is above");
        let summary: serde_json::Value = serde_json::from_str(&line).unwrap();
        let origins = FIXED.iter().map(|(role, _)| (role.to_string(), summary["origins"][role].as_str().unwrap().to_string())).collect();
        Self { child, origins, dir }
    }

    pub fn origin(&self, role: &str) -> &str {
        &self.origins.iter().find(|(r, _)| r == role).unwrap().1
    }

    /// `text` with the lab's origins in place of the fixed addresses.
    pub fn mapped(&self, text: &str) -> String {
        FIXED.iter().fold(text.to_string(), |text, (role, fixed)| text.replace(fixed, self.origin(role)))
    }

    /// The example source `name` as a list writes it, under
    /// `[sources.NAME]`, with the lab's origins.
    pub fn example(&self, name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/examples").join(format!("{name}.toml"));
        let text = std::fs::read_to_string(path).unwrap();
        self.mapped(&nested(name, &text))
    }

    /// The entries of the channel list `text`, as the app reads it.
    pub fn list(&self, text: &str) -> Vec<Entry> {
        let list = list::parse(&self.mapped(text), || Some("e2e-device".into())).unwrap();
        assert!(list.skipped.is_empty(), "{:?}", list.skipped);
        list.entries
    }

    /// The lab's answer to `path` of its API, posting `body` when given.
    pub fn api(&self, path: &str, body: Option<&str>) -> serde_json::Value {
        let url = format!("{}{path}", self.origin("api"));
        let answer = match body {
            Some(body) => ureq::post(&url).set("Content-Type", "application/json").send_string(body),
            None => ureq::get(&url).call(),
        };
        serde_json::from_str(&answer.unwrap().into_string().unwrap()).unwrap()
    }
}

impl Drop for Lab {
    /// Closes the lab's input, which stops it, and its encoders with it.
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(15);
        while self.child.try_wait().ok().flatten().is_none() {
            if Instant::now() > deadline {
                let _ = self.child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A source file's text under `[sources.NAME]`: its keys before the first
/// table under that header, each table's header prefixed.
fn nested(name: &str, text: &str) -> String {
    let mut nested = format!("[sources.{name}]\n");
    for line in text.lines() {
        let line = match line.strip_prefix("[[") {
            Some(rest) => format!("[[sources.{name}.{rest}"),
            None => match line.strip_prefix('[') {
                Some(rest) => format!("[sources.{name}.{rest}"),
                None => line.to_string(),
            },
        };
        nested.push_str(&line);
        nested.push('\n');
    }
    nested
}

/// The channels of the folder in `entries`, once its source answered, as
/// the app shows them on opening it.
pub fn folder(lab: &Lab, entries: &[Entry]) -> Vec<Channel> {
    let folder = entries.iter().find_map(|e| if let Entry::Folder(f) = e { Some(f) } else { None }).expect("a folder");
    let mut folders = Folders::new(&lab.dir);
    assert_eq!(folders.open(folder), None, "no copy before the first reading");
    let update = within(Duration::from_secs(10), || folders.poll());
    assert_eq!(update.name, folder.name);
    update.outcome.unwrap()
}

/// The channel named `name` in `entries`.
pub fn channel<'a>(entries: &'a [Entry], name: &str) -> &'a Channel {
    entries
        .iter()
        .find_map(|e| match e {
            Entry::Channel(c) if c.name.as_deref() == Some(name) => Some(c),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no channel {name}"))
}

/// The item of a source that `channel` plays.
pub fn selection(channel: &Channel) -> Selection {
    match &channel.play {
        Ok(Play::Source(selection)) => selection.clone(),
        other => panic!("{:?} plays {other:?}", channel.name),
    }
}

/// The candidates of `selection`, once they came.
pub fn candidates(selection: &Selection) -> Result<Vec<Candidate>, Failure> {
    let asking = Candidates::ask(selection);
    within(Duration::from_secs(10), || asking.poll())
}

/// The viewing's final state, once it ended.
pub fn ended(session: &Session, limit: Duration) -> State {
    within(limit, || session.stats().map(|s| s.state).filter(State::ended))
}

/// What `poll` gives, once it gives something within `limit`.
pub fn within<T>(limit: Duration, mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(found) = poll() {
            return found;
        }
        assert!(Instant::now() < deadline, "nothing within {limit:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;

/// What the test server answers for each path: status and body.
type Answers = Arc<Mutex<HashMap<String, (u16, String)>>>;

/// Serves the answers on a local port, 404 for other paths.
fn serve() -> (String, Answers) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let answers = Answers::default();
    let served = answers.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                continue;
            }
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
            let (status, body) = served.lock().unwrap().get(&path).cloned().unwrap_or((404, String::new()));
            let head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let mut stream = reader.into_inner();
            let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body.as_bytes()));
        }
    });
    (address, answers)
}

fn answer(answers: &Answers, path: &str, status: u16, body: &str) {
    answers.lock().unwrap().insert(path.into(), (status, body.into()));
}

/// A data directory of its own for each test.
fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mana-update-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The next download's outcome, once it ends.
fn outcome(updater: &mut Updater) -> Update {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(update) = updater.poll() {
            return update;
        }
        assert!(Instant::now() < deadline, "aggiornamento non finito");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn names(channels: &[Channel]) -> Vec<&str> {
    channels.iter().map(|c| c.name.as_str()).collect()
}

fn updated(update: Update) -> Vec<Channel> {
    kept(update).0
}

/// The channels of the update, and whether they are kept.
fn kept(update: Update) -> (Vec<Channel>, bool) {
    match update.outcome {
        Outcome::Channels { channels, kept } => (channels, kept),
        Outcome::Failed(reason) => panic!("aggiornamento fallito: {reason}"),
    }
}

fn failed(update: Update) -> String {
    match update.outcome {
        Outcome::Channels { channels, .. } => panic!("aggiornata con {:?}", names(&channels)),
        Outcome::Failed(reason) => reason,
    }
}

/// A list of one channel, `name`.
fn one(name: &str) -> String {
    format!("[[canali]]\nnome = \"{name}\"\nurl = \"https://a.invalid/{name}.m3u8\"\n")
}

fn list_with(address: &str, names: &[&str]) -> String {
    let channels: String = names.iter().map(|n| format!("[[canali]]\nnome = \"{n}\"\nurl = \"https://a.invalid/{n}.m3u8\"\n")).collect();
    format!("{channels}[[includi]]\nm3u = \"{address}/it.m3u\"\n")
}

#[test]
fn the_saved_copy_shows_at_once_and_a_valid_list_replaces_it() {
    let (address, answers) = serve();
    let dir = data_dir("replaced");
    let url = Url::parse(&format!("{address}/lista.toml")).unwrap();
    answer(&answers, "/lista.toml", 200, &list_with(&address, &["Uno"]));
    answer(&answers, "/it.m3u", 200, "#EXTM3U\n#EXTINF:-1,Inclusa\nhttps://a.invalid/i.m3u8\n");

    // The first start has no copy yet.
    let (saved, mut updater) = Updater::start(&dir, Some(url.clone()));
    assert_eq!(saved, None);
    assert_eq!(names(&updated(outcome(&mut updater))), ["Uno", "Inclusa"]);

    answer(&answers, "/lista.toml", 200, &list_with(&address, &["Uno", "Due"]));
    let (saved, mut updater) = Updater::start(&dir, Some(url));
    assert_eq!(names(&saved.unwrap()), ["Uno", "Inclusa"]);
    assert_eq!(names(&updated(outcome(&mut updater))), ["Uno", "Due", "Inclusa"]);
}

#[test]
fn the_copy_stays_when_the_download_fails_or_is_not_toml() {
    let (address, answers) = serve();
    let dir = data_dir("kept");
    let url = Url::parse(&format!("{address}/lista.toml")).unwrap();
    answer(&answers, "/lista.toml", 200, &list_with(&address, &["Uno"]));
    answer(&answers, "/it.m3u", 200, "#EXTINF:-1,Inclusa\nhttps://a.invalid/i.m3u8\n");
    updated(outcome(&mut Updater::start(&dir, Some(url.clone())).1));

    answer(&answers, "/lista.toml", 503, "");
    let (saved, mut updater) = Updater::start(&dir, Some(url.clone()));
    assert!(failed(outcome(&mut updater)).contains("503"));
    assert_eq!(names(&saved.unwrap()), ["Uno", "Inclusa"]);

    answer(&answers, "/lista.toml", 200, "<html>errore</html>");
    let (_, mut updater) = Updater::start(&dir, Some(url.clone()));
    assert!(failed(outcome(&mut updater)).starts_with("non è TOML valido"));
    // The copy is still the last valid one.
    assert_eq!(names(&Updater::start(&dir, Some(url)).0.unwrap()), ["Uno", "Inclusa"]);
}

#[test]
fn an_included_list_that_fails_uses_its_copy_else_is_left_out() {
    let (address, answers) = serve();
    let dir = data_dir("included");
    let url = Url::parse(&format!("{address}/lista.toml")).unwrap();
    answer(&answers, "/lista.toml", 200, &list_with(&address, &["Uno"]));
    answer(&answers, "/it.m3u", 200, "#EXTINF:-1,Inclusa\nhttps://a.invalid/i.m3u8\n");
    updated(outcome(&mut Updater::start(&dir, Some(url.clone())).1));

    answer(&answers, "/it.m3u", 500, "");
    assert_eq!(names(&updated(outcome(&mut Updater::start(&dir, Some(url.clone())).1))), ["Uno", "Inclusa"]);

    // Another inclusion, never downloaded: left out.
    let list = format!("{}[[includi]]\nm3u = \"{address}/altra.m3u\"\n", list_with(&address, &["Uno"]));
    answer(&answers, "/lista.toml", 200, &list);
    assert_eq!(names(&updated(outcome(&mut Updater::start(&dir, Some(url)).1))), ["Uno", "Inclusa"]);
}

#[test]
fn a_copy_from_another_url_is_not_shown() {
    let (address, answers) = serve();
    let dir = data_dir("other");
    answer(&answers, "/lista.toml", 200, "[[canali]]\nnome = \"Uno\"\nurl = \"https://a.invalid/1.m3u8\"\n");
    let url = Url::parse(&format!("{address}/lista.toml")).unwrap();
    updated(outcome(&mut Updater::start(&dir, Some(url)).1));
    let other = Url::parse(&format!("{address}/altra.toml")).unwrap();
    assert_eq!(Updater::start(&dir, Some(other)).0, None);
    // Without a URL, the copy there is, and nothing to download.
    let (saved, updater) = Updater::start(&dir, None);
    assert_eq!(names(&saved.unwrap()), ["Uno"]);
    assert_eq!(updater.url(), None);
    assert!(!updater.downloading());
}

#[test]
fn without_a_data_directory_the_list_works_for_the_session() {
    let (address, answers) = serve();
    answer(&answers, "/lista.toml", 200, "[[canali]]\nnome = \"Uno\"\nurl = \"https://a.invalid/1.m3u8\"\n");
    let url = Url::parse(&format!("{address}/lista.toml")).unwrap();
    let (saved, mut updater) = Updater::start(Path::new("/dev/null/mana"), Some(url));
    assert_eq!(saved, None);
    assert_eq!(names(&updated(outcome(&mut updater))), ["Uno"]);
}

#[test]
fn a_list_url_is_http_or_https_without_control_characters_within_a_length() {
    assert_eq!(list_url(" https://a.example/lista.toml\n").unwrap().as_str(), "https://a.example/lista.toml");
    assert!(list_url("http://a.example/l?x=1").is_ok());
    assert!(list_url("").unwrap_err().contains("vuoto"));
    assert!(list_url("ftp://a.example/l").unwrap_err().contains("ftp"));
    assert!(list_url("file:///etc/passwd").is_err());
    assert!(list_url("https://a.example/l\u{7}x").unwrap_err().contains("controllo"));
    assert!(list_url("https://a.example/\tl").is_err());
    assert!(list_url("non un indirizzo").is_err());
    let long = format!("https://a.example/{}", "a".repeat(MAX_URL));
    assert!(list_url(&long).unwrap_err().contains("oltre"));
}

#[test]
fn a_url_chosen_whose_list_is_toml_is_used_at_the_next_start() {
    let (address, answers) = serve();
    let dir = data_dir("chosen");
    let first = Url::parse(&format!("{address}/prima.toml")).unwrap();
    let chosen = Url::parse(&format!("{address}/scelta.toml")).unwrap();
    answer(&answers, "/prima.toml", 200, &one("Prima"));
    answer(&answers, "/scelta.toml", 200, &one("Scelta"));
    let (_, mut updater) = Updater::start(&dir, Some(first));
    updated(outcome(&mut updater));

    let ticket = updater.change(chosen.clone());
    assert!(updater.downloading());
    let update = outcome(&mut updater);
    assert_eq!((update.ticket, &update.url), (ticket, &chosen));
    let (channels, kept) = kept(update);
    assert_eq!((names(&channels), kept), (vec!["Scelta"], true));
    assert!(!updater.downloading());
    assert_eq!(updater.url(), Some(&chosen));

    let (saved, updater) = Updater::start(&dir, None);
    assert_eq!(names(&saved.unwrap()), ["Scelta"]);
    assert_eq!(updater.url(), Some(&chosen));
}

#[test]
fn a_url_chosen_whose_list_is_not_toml_changes_nothing() {
    let (address, answers) = serve();
    let dir = data_dir("refused");
    let chosen = Url::parse(&format!("{address}/scelta.toml")).unwrap();
    let wrong = Url::parse(&format!("{address}/pagina.html")).unwrap();
    answer(&answers, "/scelta.toml", 200, &one("Scelta"));
    answer(&answers, "/pagina.html", 200, "<html>errore</html>");
    let (_, mut updater) = Updater::start(&dir, None);
    updater.change(chosen.clone());
    updated(outcome(&mut updater));

    updater.change(wrong.clone());
    let update = outcome(&mut updater);
    assert_eq!(update.url, wrong);
    assert!(failed(update).starts_with("non è TOML valido"));
    assert_eq!(updater.url(), Some(&chosen));

    let (saved, updater) = Updater::start(&dir, None);
    assert_eq!(names(&saved.unwrap()), ["Scelta"]);
    assert_eq!(updater.url(), Some(&chosen));
}

#[test]
fn the_settings_url_wins_over_the_one_chosen_without_replacing_it() {
    let (address, answers) = serve();
    let dir = data_dir("override");
    let chosen = Url::parse(&format!("{address}/scelta.toml")).unwrap();
    let configured = Url::parse(&format!("{address}/prove.toml")).unwrap();
    answer(&answers, "/scelta.toml", 200, &one("Scelta"));
    answer(&answers, "/prove.toml", 200, &one("Prove"));
    let (_, mut updater) = Updater::start(&dir, None);
    updater.change(chosen.clone());
    updated(outcome(&mut updater));

    let (_, mut updater) = Updater::start(&dir, Some(configured.clone()));
    assert_eq!(updater.url(), Some(&configured));
    assert_eq!(names(&updated(outcome(&mut updater))), ["Prove"]);

    assert_eq!(Updater::start(&dir, None).1.url(), Some(&chosen));
}

/// Downloading again the list of the settings' URL brings its new channels
/// and leaves the URL chosen from the phone for the next start.
#[test]
fn a_refresh_downloads_the_list_in_use_again_and_keeps_the_url_chosen() {
    let (address, answers) = serve();
    let dir = data_dir("refresh");
    let chosen = Url::parse(&format!("{address}/scelta.toml")).unwrap();
    let configured = Url::parse(&format!("{address}/prove.toml")).unwrap();
    answer(&answers, "/scelta.toml", 200, &one("Scelta"));
    answer(&answers, "/prove.toml", 200, &one("Prove"));
    let (_, mut updater) = Updater::start(&dir, None);
    assert_eq!(updater.refresh(), None);
    updater.change(chosen.clone());
    updated(outcome(&mut updater));

    let (_, mut updater) = Updater::start(&dir, Some(configured.clone()));
    updated(outcome(&mut updater));
    answer(&answers, "/prove.toml", 200, &list_with(&address, &["Prove", "Nuovo"]));
    let ticket = updater.refresh().unwrap();
    let update = outcome(&mut updater);
    assert_eq!((update.ticket, &update.url), (ticket, &configured));
    assert_eq!(names(&updated(update)), ["Prove", "Nuovo"]);

    assert_eq!(Updater::start(&dir, None).1.url(), Some(&chosen));
}

#[test]
fn downloads_run_in_the_order_asked_and_the_last_one_stays() {
    let (address, answers) = serve();
    let dir = data_dir("order");
    let urls: Vec<Url> = ["a", "b", "c"].iter().map(|n| Url::parse(&format!("{address}/{n}.toml")).unwrap()).collect();
    for name in ["a", "b", "c"] {
        answer(&answers, &format!("/{name}.toml"), 200, &one(name));
    }
    let (_, mut updater) = Updater::start(&dir, Some(urls[0].clone()));
    let tickets: Vec<Ticket> = urls.iter().map(|url| updater.change(url.clone())).collect();
    let got: Vec<(Ticket, Url)> = (0..4).map(|_| outcome(&mut updater)).map(|u| (u.ticket, u.url)).collect();
    // The download at start first, then those asked, each with its ticket.
    assert_eq!(got[0].1, urls[0]);
    let asked: Vec<(Ticket, Url)> = tickets.into_iter().zip(urls.iter().cloned()).collect();
    assert_eq!(got[1..], asked[..]);
    assert!(got[0].0 != got[1].0);

    let (saved, updater) = Updater::start(&dir, None);
    assert_eq!(names(&saved.unwrap()), ["c"]);
    assert_eq!(updater.url(), Some(&urls[2]));
}

#[test]
fn a_url_chosen_that_cannot_be_saved_is_not_kept() {
    let (address, answers) = serve();
    answer(&answers, "/lista.toml", 200, &one("Uno"));
    let url = Url::parse(&format!("{address}/lista.toml")).unwrap();
    let (_, mut updater) = Updater::start(Path::new("/dev/null/mana"), None);
    updater.change(url);
    let (channels, kept) = kept(outcome(&mut updater));
    assert_eq!((names(&channels), kept), (vec!["Uno"], false));
}

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::*;

#[test]
fn a_locale_is_italian_only_when_its_language_is() {
    for code in ["it-IT", "it", "it_CH", "IT-it", "it_IT.UTF-8"] {
        assert_eq!(Language::of_locale(code), Language::Italian, "{code}");
    }
    for code in ["en-GB", "de-DE", "", "C", "C.UTF-8", "its", "fr_IT"] {
        assert_eq!(Language::of_locale(code), Language::English, "{code}");
    }
}

#[test]
fn the_menu_comes_before_the_tv_and_the_tv_before_english() {
    let italian_tv = Ok("it-IT".to_string());
    assert_eq!(resolve(Choice::English, Some(&italian_tv)), (Language::English, Origin::Menu));
    assert_eq!(resolve(Choice::Italian, None), (Language::Italian, Origin::Menu));
    assert_eq!(resolve(Choice::Automatic, Some(&italian_tv)), (Language::Italian, Origin::Tv("it-IT".into())));
    let german_tv = Ok("de-DE".to_string());
    assert_eq!(resolve(Choice::Automatic, Some(&german_tv)).0, Language::English);
    let failed = Err("timeout".to_string());
    assert_eq!(resolve(Choice::Automatic, Some(&failed)), (Language::English, Origin::Default("timeout".into())));
    assert_eq!(resolve(Choice::Automatic, None).0, Language::English);
}

#[test]
fn the_choice_is_kept_and_an_unknown_one_is_automatic() {
    let dir = std::env::temp_dir().join(format!("mana-language-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(Choice::load(&dir), Choice::Automatic);
    Choice::English.save(&dir).unwrap();
    assert_eq!(Choice::load(&dir), Choice::English);
    Choice::Italian.save(&dir).unwrap();
    assert_eq!(Choice::load(&dir), Choice::Italian);
    std::fs::write(dir.join(choice::FILE), "klingon").unwrap();
    assert_eq!(Choice::load(&dir), Choice::Automatic);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_choices_cycle_in_the_settings_order() {
    assert_eq!(Choice::Automatic.next(), Choice::Italian);
    assert_eq!(Choice::Italian.next(), Choice::English);
    assert_eq!(Choice::English.next(), Choice::Automatic);
    assert_eq!(Choice::Automatic.previous(), Choice::English);
    assert_eq!(Choice::Automatic.label(Language::Italian), "Automatica");
    assert_eq!(Choice::English.label(Language::Italian), "English");
    assert_eq!(Choice::Italian.label(Language::English), "Italiano");
}

#[test]
fn variables_fill_in_without_marks_and_counts_select_the_plural() {
    assert_eq!(crate::text!(Language::English, "settings-channels", count = 1), "1 channel");
    assert_eq!(crate::text!(Language::English, "settings-channels", count = 10_usize), "10 channels");
    assert_eq!(crate::text!(Language::Italian, "settings-channels", count = 1), "1 canale");
    assert_eq!(crate::text!(Language::Italian, "settings-channels", count = 0), "0 canali");
    assert_eq!(crate::text!(Language::Italian, "notice-test-list", file = "/tmp/a.m3u"), "La TV usa la lista di prova /tmp/a.m3u");
    assert_eq!(crate::text!(Language::English, "home-position", focus = 5000, count = 12_000), "5000 of 12000");
    assert_eq!(crate::text!(Language::English, "net-empty", path = "/a.m3u8"), "/a.m3u8: empty answer");
    assert_eq!(message(Language::Italian, "no-such-message", &[]), "no-such-message");
}

#[test]
fn the_catalogs_read_without_errors() {
    for language in Language::ALL {
        let resource = FluentResource::try_new(language.source().to_string());
        assert!(resource.is_ok(), "{language}: {:?}", resource.err().map(|(_, errors)| errors));
        let mut catalog = FluentBundle::new_concurrent(vec![language.tag().parse().unwrap()]);
        assert!(catalog.add_resource(resource.unwrap()).is_ok(), "{language}: a message twice");
    }
}

#[test]
fn both_catalogs_have_the_same_messages_with_the_same_variables() {
    let english = messages(Language::English.source());
    let italian = messages(Language::Italian.source());
    assert!(english.len() > 100, "{} messages read", english.len());
    for (id, variables) in &english {
        assert_eq!(italian.get(id), Some(variables), "{id} in it.ftl");
    }
    for id in italian.keys() {
        assert!(english.contains_key(id), "{id} only in it.ftl");
    }
}

#[test]
fn every_text_names_a_message_with_its_variables_and_every_message_is_used() {
    let catalog = messages(Language::English.source());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut used = BTreeSet::new();
    let mut sources = Vec::new();
    for dir in ["crates", "apps"] {
        rust_files(&root.join(dir), &mut sources);
    }
    for file in &sources {
        let source = std::fs::read_to_string(file).unwrap();
        for (id, variables) in text_calls(&source) {
            let place = format!("{}: {id}", file.display());
            assert_eq!(catalog.get(&id), Some(&variables), "{place}: message missing or with other variables");
            used.insert(id);
        }
    }
    let unused: Vec<_> = catalog.keys().filter(|id| !used.contains(*id)).collect();
    assert!(unused.is_empty(), "messages no `text!` uses: {unused:?}");
}

/// The messages of a catalog and the variables each one uses, read from
/// its lines: a message starts at an identifier at the start of a line and
/// goes on over the indented lines after it.
fn messages(source: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut messages = BTreeMap::new();
    let mut current = None;
    for line in source.lines() {
        if line.starts_with(|c: char| c.is_ascii_alphabetic()) {
            let (id, value) = line.split_once('=').unwrap_or_else(|| panic!("no = in {line:?}"));
            let id = id.trim().to_string();
            assert!(messages.insert(id.clone(), variables(value)).is_none(), "{id} twice");
            current = Some(id);
        } else if line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
            let id = current.as_ref().unwrap_or_else(|| panic!("{line:?} outside a message"));
            messages.get_mut(id).unwrap().extend(variables(line));
        } else {
            current = None;
        }
    }
    messages
}

/// The `$variables` of a line of a catalog.
fn variables(line: &str) -> BTreeSet<String> {
    line.split('$')
        .skip(1)
        .map(|after| after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect())
        .collect()
}

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() && path.file_name().is_some_and(|name| name != "target") {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// The message id and the variable names of each `text!(language, "id",
/// name = value, …)` in a Rust source, outside line comments.
fn text_calls(source: &str) -> Vec<(String, BTreeSet<String>)> {
    const CALL: &str = "text!(";
    let mut calls = Vec::new();
    let mut from = 0;
    while let Some(found) = source[from..].find(CALL) {
        let at = from + found;
        from = at + CALL.len();
        let line_start = source[..at].rfind('\n').map_or(0, |i| i + 1);
        let in_comment = source[line_start..at].contains("//");
        // Another macro's name, or the text of a string.
        let inside_other = source[..at].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '"');
        if in_comment || inside_other {
            continue;
        }
        let arguments = top_level_arguments(&source[from..]);
        let id = arguments.get(1).map(|id| id.trim().trim_matches('"').to_string()).unwrap_or_default();
        let variables = arguments
            .iter()
            .skip(2)
            .filter_map(|argument| argument.split_once('=').map(|(name, _)| name.trim().to_string()))
            .collect();
        calls.push((id, variables));
    }
    calls
}

/// The comma-separated arguments of a macro call, from after its `(` to
/// its matching `)`, with strings and nested brackets kept whole.
fn top_level_arguments(call: &str) -> Vec<&str> {
    let (mut arguments, mut depth, mut start, mut in_string, mut escaped) = (Vec::new(), 0usize, 0, false, false);
    for (i, c) in call.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ')' => {
                arguments.push(&call[start..i]);
                break;
            }
            ',' if depth == 0 => {
                arguments.push(&call[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    arguments.into_iter().filter(|argument| !argument.trim().is_empty()).collect()
}

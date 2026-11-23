use std::path::{Path, PathBuf};

use super::*;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/examples")
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "toml")).collect();
    files.sort();
    files
}

#[test]
fn every_example_keeps_the_contract() {
    let files = toml_files(&examples());
    assert!(files.len() >= 6, "{files:?}");
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        if let Err(error) = Source::from_toml(&text) {
            panic!("{}: {error}", path.display());
        }
    }
}

/// Each file of config/examples/rejected breaks the rule its name gives,
/// before any `--`.
#[test]
fn every_rejected_case_fails_on_its_own_rule() {
    let files = toml_files(&examples().join("rejected"));
    let mut codes = std::collections::BTreeSet::new();
    for path in files {
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let expected = stem.split("--").next().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        match Source::from_toml(&text) {
            Ok(_) => panic!("{stem}: accepted"),
            Err(error) => assert_eq!(error.code(), expected, "{stem}: {error}"),
        }
        codes.insert(expected.to_string());
    }
    let all = [
        "format",
        "unknown-field",
        "missing-field",
        "invalid-value",
        "unsupported-operation",
        "one-of",
        "unknown-reference",
        "type-mismatch",
        "unknown-headers",
        "unknown-resolver",
        "unknown-recipe",
        "call-inputs",
        "duplicate-id",
        "equal-routes",
        "call-cycle",
        "bad-pattern",
        "bad-selector",
        "wrong-output",
    ];
    assert_eq!(codes, all.iter().map(|c| c.to_string()).collect(), "a rule without its rejected case, or the reverse");
}

#[test]
fn the_flows_of_an_event_source_are_read_in_order() {
    let source = Source::from_toml(&std::fs::read_to_string(examples().join("lab-events.toml")).unwrap()).unwrap();
    let Some(Catalogue::Steps(catalogue)) = &source.catalogue else { panic!("{:?}", source.catalogue) };
    assert_eq!(catalogue.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["page", "events"]);
    let Resolve::Routes(routes) = &source.resolve else { panic!("{:?}", source.resolve) };
    assert_eq!(routes[0].when, When { kind: Some("m3u8".into()), ..When::default() });
    assert_eq!(routes[0].to, Use::Stream(Some(Headers::Set("media".into()))));
    assert_eq!(routes[1].to, Use::Resolver("envelope".into()));
    assert_eq!(source.resolvers["envelope"].steps.last().unwrap().operation.output(), Kind::Next);
}

fn reference(path: &[&str]) -> Reference {
    Reference { path: path.iter().map(|name| Segment::Name(name.to_string())).collect(), component: false }
}

#[test]
fn a_reference_starts_at_a_brace_followed_by_a_letter() {
    let template = Template::parse("{base}/v1/{input.item.id|component}.json?a={}&b={\"x\":1}").unwrap();
    let component = Reference { component: true, ..reference(&["input", "item", "id"]) };
    assert_eq!(
        template.parts,
        [
            Part::Reference(reference(&["base"])),
            Part::Text("/v1/".into()),
            Part::Reference(component),
            Part::Text(".json?a={}&b={\"x\":1}".into()),
        ]
    );
    assert_eq!(template.whole(), None);
    assert_eq!(Template::parse("{answer.json.video.0}").unwrap().whole(), Some(&reference(&["answer", "json", "video", "0"])));
}

#[test]
fn a_reference_may_hold_another() {
    let template = Template::parse("{pool.{order.0}}").unwrap();
    let nested = Reference { path: vec![Segment::Name("pool".into()), Segment::Reference(reference(&["order", "0"]))], component: false };
    assert_eq!(template.whole(), Some(&nested));
    assert_eq!(nested.to_string(), "{pool.{order.0}}");
}

#[test]
fn a_field_with_other_characters_is_quoted() {
    let template = Template::parse("{config.\"common:web:client\".trd}").unwrap();
    assert_eq!(template.whole(), Some(&reference(&["config", "common:web:client", "trd"])));
    assert_eq!(template.whole().unwrap().to_string(), "{config.\"common:web:client\".trd}");
    assert!(Template::parse("{config.\"open}").unwrap_err().contains("closing"));
}

#[test]
fn a_broken_reference_is_named() {
    assert!(Template::parse("{a.b").unwrap_err().contains("without its `}`"));
    assert!(Template::parse("{a.}").unwrap_err().contains("empty name"));
    assert!(Template::parse("{a|base64}").unwrap_err().contains("unknown filter `base64`"));
}

#[test]
fn a_response_gives_bytes_text_json_and_headers() {
    let source = |reference: &str, op: &str, field: &str| {
        format!(
            "id = \"x\"\nname = \"X\"\n[[resolve.steps]]\nid = \"answer\"\nop = \"http\"\nurl = \"http://127.0.0.1/\"\n\
             [[resolve.steps]]\nid = \"value\"\nop = \"{op}\"\n{field} = \"{reference}\"\n\
             [[resolve.steps]]\nid = \"stream\"\nop = \"stream\"\nurl = \"http://127.0.0.1/a.m3u8\"\n"
        )
    };
    assert!(Source::from_toml(&source("{answer.body}", "protobuf", "from")).is_ok());
    assert!(Source::from_toml(&source("{answer.text}", "rot47", "from")).is_ok());
    assert!(Source::from_toml(&source("{answer.header.X-Session}", "md5", "from")).is_ok());
    let error = Source::from_toml(&source("{answer.text}", "protobuf", "from")).unwrap_err();
    assert_eq!(error, Error::TypeMismatch { at: "resolve.value".into(), reference: "{answer.text}".into(), expected: Kind::Bytes, found: Kind::Text });
    let error = Source::from_toml(&source("{answer.status}", "rot47", "from")).unwrap_err();
    assert_eq!(error.code(), "unknown-reference");
}

#[test]
fn errors_name_the_place_in_both_languages() {
    let error = Error::UnknownHeaders { at: "resolve.stream".into(), name: "media".into() };
    assert_eq!(error.to_string(), "resolve.stream: no header set `media`");
    assert_eq!(error.text(Language::Italian), "resolve.stream: nessun insieme di header `media`");
}

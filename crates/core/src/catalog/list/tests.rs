use super::*;

const LIST: &str = r#"
# Unknown sections and fields are for later versions.
[future]
something = 1

[sources.raiplay]
name = "RaiPlay"
[[sources.raiplay.resolve.steps]]
id = "live"
op = "http"
url = "https://www.raiplay.invalid/live/{input.item.id|component}.json"
[[sources.raiplay.resolve.steps]]
id = "stream"
op = "stream"
url = "{live.json.video.content_url}"

[sources.pluto]
[[sources.pluto.resolve.steps]]
id = "boot"
op = "http"
url = "https://boot.pluto.invalid/v4/start?clientID={input.device}&channelID={input.item.id}"
[[sources.pluto.resolve.steps]]
id = "stream"
op = "stream"
url = "{boot.json.servers.stitcher}/channel/{input.item.id}/master.m3u8?{boot.json.stitcherParams}"

[[channels]]
name = "Rai 1"
logo = "https://logo.invalid/rai1.png"
group = "Rai"
source = "raiplay"
id = "rai1"
colour = "blue"

[[include]]
m3u = "https://iptv.invalid/it.m3u"

[[channels]]
name = "Sportitalia Live 24"
group = "Sport"
url = "https://si.invalid/silive24/playlist.m3u8"
header = { "User-Agent" = "Mana/1", "Bad Name" = "x" }

[[channels]]
name = "Pluto Cinema"
source = "pluto"
id = 5

[[include]]
list = "https://other.invalid/list.toml"
"#;

fn device() -> Option<String> {
    Some("d-1".into())
}

fn channels(list: &List) -> Vec<&Channel> {
    list.entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Channel(channel) => Some(channel),
            Entry::Include(_) => None,
        })
        .collect()
}

#[test]
fn channels_and_inclusions_keep_the_file_s_order() {
    let list = parse(LIST, device).unwrap();
    let order: Vec<String> = list
        .entries
        .iter()
        .map(|entry| match entry {
            Entry::Channel(channel) => channel.name.clone().unwrap_or_default(),
            Entry::Include(url) => url.to_string(),
        })
        .collect();
    assert_eq!(order, ["Rai 1", "https://iptv.invalid/it.m3u", "Sportitalia Live 24", "Pluto Cinema"]);
    assert_eq!(list.skipped, ["inclusion without `m3u`"]);
}

fn selection(channel: &Channel) -> &Selection {
    match channel.play.as_ref().unwrap() {
        Play::Source(selection) => selection,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_source_s_channel_is_an_item_of_its_fields_and_the_device() {
    let list = parse(LIST, device).unwrap();
    let found = channels(&list);
    let rai = found[0];
    assert_eq!((rai.logo.as_deref(), rai.group.as_deref()), (Some("https://logo.invalid/rai1.png"), Some("Rai")));
    let rai = selection(rai);
    assert_eq!((rai.source.id.as_str(), rai.source.name.as_str()), ("raiplay", "RaiPlay"));
    let fields: Vec<(&str, &str)> = rai.item.fields.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(fields, [("colour", "blue"), ("id", "rai1"), ("title", "Rai 1")]);
    let pluto = selection(found[2]);
    assert_eq!((pluto.source.name.as_str(), pluto.item.fields["id"].as_str(), pluto.device.as_str()), ("pluto", "5", "d-1"));
}

#[test]
fn a_direct_channel_keeps_the_headers_a_request_can_carry() {
    let list = parse(LIST, device).unwrap();
    let Play::Stream(stream) = channels(&list)[1].play.clone().unwrap() else { panic!() };
    assert_eq!(stream.url.as_str(), "https://si.invalid/silive24/playlist.m3u8");
    assert_eq!(stream.headers, [("User-Agent".to_string(), "Mana/1".to_string())]);
}

#[test]
fn the_device_is_asked_for_only_by_channels_with_a_source() {
    let asked = std::cell::Cell::new(0);
    let device = || {
        asked.set(asked.get() + 1);
        None
    };
    parse("[[channels]]\nname = \"A\"\nurl = \"https://a.invalid/1.m3u8\"\n", device).unwrap();
    assert_eq!(asked.get(), 0);
    let list = parse(LIST, device).unwrap();
    assert_eq!(asked.get(), 2);
    assert_eq!(selection(channels(&list)[2]).device, "");
}

#[test]
fn a_broken_channel_is_unavailable_with_the_reason() {
    let text = r#"
[sources.good]
[[sources.good.resolve.steps]]
id = "stream"
op = "stream"
url = "https://a.invalid/{input.item.id}.m3u8"
[sources.broken]
[[sources.broken.resolve.steps]]
id = "code"
op = "eval"

[[channels]]
url = "https://a.invalid/1.m3u8"
[[channels]]
name = "Both"
url = "https://a.invalid/2.m3u8"
source = "good"
[[channels]]
name = "Unknown"
source = "missing"
[[channels]]
name = "Broken"
source = "broken"
[[channels]]
name = "Headers"
source = "good"
header = { "User-Agent" = "x" }
[[channels]]
name = "Neither"
[[channels]]
name = 3
url = "https://a.invalid/3.m3u8"
[[channels]]
name = "Relative"
url = "/live.m3u8"
[[channels]]
name = "Good"
url = "https://a.invalid/4.m3u8"
"#;
    let list = parse(text, || None).unwrap();
    let outcome: Vec<(Option<&str>, Result<(), ChannelError>)> =
        channels(&list).iter().map(|c| (c.name.as_deref(), c.play.as_ref().map(|_| ()).map_err(Clone::clone))).collect();
    let broken = source::Error::UnsupportedOperation { at: "resolve.code".into(), op: "eval".into() };
    assert_eq!(
        outcome,
        [
            (None, Err(ChannelError::NoName)),
            (Some("Both"), Err(ChannelError::BothUrlAndSource)),
            (Some("Unknown"), Err(ChannelError::UnknownSource("missing".into()))),
            (Some("Broken"), Err(ChannelError::BrokenSource { name: "broken".into(), error: broken })),
            (Some("Headers"), Err(ChannelError::HeaderWithSource)),
            (Some("Neither"), Err(ChannelError::NeitherUrlNorSource)),
            (None, Err(ChannelError::NotText("name".into()))),
            (Some("Relative"), Err(ChannelError::BadUrl("relative URL without a base".into()))),
            (Some("Good"), Ok(())),
        ]
    );
    let reasons: Vec<String> = channels(&list)[..4].iter().map(|c| c.play.as_ref().unwrap_err().text(Language::Italian)).collect();
    assert_eq!(
        reasons,
        [
            "canale senza `name`",
            "canale con sia `url` sia `source`",
            "fonte missing inesistente",
            "fonte broken: resolve.code: operazione `eval` sconosciuta a questa versione di Mana"
        ]
    );
    assert_eq!(channels(&list)[0].title(Language::Italian), "Canale senza nome");
    assert_eq!(channels(&list)[0].title(Language::English), "Unnamed channel");
}

#[test]
fn a_file_that_is_not_toml_is_refused() {
    let line = |text| match parse(text, device) {
        Err(Error::NotToml { line, .. }) => line,
        other => panic!("{other:?}"),
    };
    assert_eq!(line("[[channels]\nname = \"A\""), Some(1));
    assert_eq!(line("[[channels]]\nname = \"A\"\nname = \"B\"\n"), Some(3));
    assert!(parse("[[channels]\nname = \"A\"", device).unwrap_err().to_string().starts_with("not valid TOML: line 1: "));
    assert!(parse("<html>404</html>", device).is_err());
    let empty = parse("", device).unwrap();
    assert!(empty.entries.is_empty() && empty.skipped.is_empty());
}

#[test]
fn a_list_with_the_italian_keys_is_refused_naming_the_english_ones() {
    let old = "[ricette.r]\npassi = [\"get https://a.invalid/x\"]\n[[canali]]\nnome = \"A\"\nurl = \"https://a.invalid/1.m3u8\"\n";
    let refused = parse(old, device).unwrap_err();
    assert_eq!(refused, Error::ItalianKeys);
    assert!(refused.to_string().contains("`channels`") && refused.text(Language::Italian).contains("`channels`"));
    // Other old keys are only unknown, as any other.
    let mixed = parse("[[channels]]\nnome = \"A\"\nurl = \"https://a.invalid/1.m3u8\"\n[[includi]]\nm3u = \"x\"\n", device).unwrap();
    assert_eq!(channels(&mixed)[0].play.as_ref().err(), Some(&ChannelError::NoName));
    assert_eq!(mixed.entries.len(), 1);
}

#[test]
fn sections_of_the_wrong_kind_are_skipped() {
    let list = parse("channels = \"tutti\"\ninclude = [1]\n", device).unwrap();
    assert!(list.entries.is_empty());
    assert_eq!(list.skipped, ["`channels` is not a list of tables", "an entry of `include` is not a table"]);
}

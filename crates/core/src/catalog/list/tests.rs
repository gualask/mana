use super::*;
use crate::live::recipe::{self, Step};

const LIST: &str = r#"
# Unknown sections and fields are for later versions.
[future]
something = 1

[recipes.raiplay]
steps = [
  "get https://www.raiplay.invalid/live/{id}.json",
  "json video.content_url",
  "query output=62",
  "get",
  "json video.0",
]

[recipes.pluto]
steps = [
  "get https://boot.pluto.invalid/v4/start?clientID={device}&channelID={id}",
  "text {json:servers.stitcher}/channel/{id}/master.m3u8?{json:stitcherParams}",
]

[recipes.unknown_step]
steps = ["decrypt {id}"]

[[channels]]
name = "Rai 1"
logo = "https://logo.invalid/rai1.png"
group = "Rai"
recipe = "raiplay"
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
recipe = "pluto"
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

#[test]
fn a_recipe_s_channel_gives_its_parameters_and_the_device() {
    let list = parse(LIST, device).unwrap();
    let found = channels(&list);
    let rai = found[0];
    assert_eq!((rai.logo.as_deref(), rai.group.as_deref()), (Some("https://logo.invalid/rai1.png"), Some("Rai")));
    let rai_play = rai.play.as_ref().unwrap();
    assert_eq!(rai_play.resolve[0], Step::Get(Some("https://www.raiplay.invalid/live/rai1.json".into())));
    assert_eq!(rai_play.resolve.len(), 5);
    let pluto = found[2].play.as_ref().unwrap();
    let boot = "https://boot.pluto.invalid/v4/start?clientID=d-1&channelID=5";
    let stitch = "{json:servers.stitcher}/channel/5/master.m3u8?{json:stitcherParams}";
    assert_eq!(pluto.resolve, [Step::Get(Some(boot.into())), Step::Text(stitch.into())]);
}

#[test]
fn a_direct_channel_keeps_the_headers_a_request_can_carry() {
    let list = parse(LIST, device).unwrap();
    let source = channels(&list)[1].play.clone().unwrap();
    assert_eq!(source.url.as_ref().map(Url::as_str), Some("https://si.invalid/silive24/playlist.m3u8"));
    assert_eq!(source.headers, [("User-Agent".to_string(), "Mana/1".to_string())]);
}

#[test]
fn the_device_is_asked_for_only_when_a_recipe_uses_it() {
    let text = "[recipes.r]\nsteps = [\"get https://a.invalid/{id}\"]\n[[channels]]\nnome = \"A\"\nricetta = \"r\"\nid = \"x\"\n";
    let asked = std::cell::Cell::new(0);
    parse(text, || {
        asked.set(asked.get() + 1);
        None
    })
    .unwrap();
    assert_eq!(asked.get(), 0);
}

#[test]
fn a_broken_channel_is_unavailable_with_the_reason() {
    let text = r#"
[recipes.unknown_step]
steps = ["decrypt {id}"]
[recipes.broken]
steps = "get"

[[channels]]
url = "https://a.invalid/1.m3u8"
[[channels]]
name = "Both"
url = "https://a.invalid/2.m3u8"
recipe = "unknown_step"
[[channels]]
name = "Unknown"
recipe = "missing"
[[channels]]
name = "Unknown step"
recipe = "unknown_step"
id = "x"
[[channels]]
name = "No id"
recipe = "unknown_step"
[[channels]]
name = "Broken"
recipe = "broken"
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
    let recipe = |name: &str, error| Err(ChannelError::Recipe { name: name.into(), error });
    assert_eq!(
        outcome,
        [
            (None, Err(ChannelError::NoName)),
            (Some("Both"), Err(ChannelError::BothUrlAndRecipe)),
            (Some("Unknown"), Err(ChannelError::UnknownRecipe("missing".into()))),
            (Some("Unknown step"), recipe("unknown_step", recipe::Error::InvalidStep("decrypt x".into()))),
            (Some("No id"), recipe("unknown_step", recipe::Error::MissingParameter("id".into()))),
            (Some("Broken"), Err(ChannelError::BrokenRecipe { name: "broken".into(), problem: RecipeProblem::StepsNotList })),
            (None, Err(ChannelError::NotText("name".into()))),
            (Some("Relative"), Err(ChannelError::BadUrl("relative URL without a base".into()))),
            (Some("Good"), Ok(())),
        ]
    );
    let reasons: Vec<String> = channels(&list)[..4].iter().map(|c| c.play.as_ref().unwrap_err().text(Language::Italian)).collect();
    assert_eq!(
        reasons,
        ["canale senza `name`", "canale con sia `url` sia `recipe`", "ricetta missing inesistente", "ricetta unknown_step: passo non valido: decrypt x"]
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

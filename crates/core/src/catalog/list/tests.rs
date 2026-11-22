use super::*;
use crate::live::recipe::Step;

const LIST: &str = r#"
# Unknown sections and fields are for later versions.
[futuro]
qualcosa = 1

[ricette.raiplay]
passi = [
  "get https://www.raiplay.invalid/dirette/{id}.json",
  "json video.content_url",
  "query output=62",
  "get",
  "json video.0",
]

[ricette.pluto]
passi = [
  "get https://boot.pluto.invalid/v4/start?clientID={dispositivo}&channelID={id}",
  "testo {json:servers.stitcher}/channel/{id}/master.m3u8?{json:stitcherParams}",
]

[ricette.futura]
passi = ["decifra {id}"]

[[canali]]
nome = "Rai 1"
logo = "https://logo.invalid/rai1.png"
gruppo = "Rai"
ricetta = "raiplay"
id = "rai1"
colore = "blu"

[[includi]]
m3u = "https://iptv.invalid/it.m3u"

[[canali]]
nome = "Sportitalia Live 24"
gruppo = "Sport"
url = "https://si.invalid/silive24/playlist.m3u8"
header = { "User-Agent" = "Mana/1", "Bad Name" = "x" }

[[canali]]
nome = "Pluto Cinema"
ricetta = "pluto"
id = 5

[[includi]]
lista = "https://altro.invalid/lista.toml"
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
            Entry::Channel(channel) => channel.name.clone(),
            Entry::Include(url) => url.to_string(),
        })
        .collect();
    assert_eq!(order, ["Rai 1", "https://iptv.invalid/it.m3u", "Sportitalia Live 24", "Pluto Cinema"]);
    assert_eq!(list.skipped, ["inclusione senza `m3u`"]);
}

#[test]
fn a_recipe_s_channel_gives_its_parameters_and_the_device() {
    let list = parse(LIST, device).unwrap();
    let found = channels(&list);
    let rai = found[0];
    assert_eq!((rai.logo.as_deref(), rai.group.as_deref()), (Some("https://logo.invalid/rai1.png"), Some("Rai")));
    let rai_play = rai.play.as_ref().unwrap();
    assert_eq!(rai_play.resolve[0], Step::Get(Some("https://www.raiplay.invalid/dirette/rai1.json".into())));
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
    assert_eq!(source.url.as_deref(), Some("https://si.invalid/silive24/playlist.m3u8"));
    assert_eq!(source.headers, [("User-Agent".to_string(), "Mana/1".to_string())]);
}

#[test]
fn the_device_is_asked_for_only_when_a_recipe_uses_it() {
    let text = "[ricette.r]\npassi = [\"get https://a.invalid/{id}\"]\n[[canali]]\nnome = \"A\"\nricetta = \"r\"\nid = \"x\"\n";
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
[ricette.futura]
passi = ["decifra {id}"]
[ricette.rotta]
passi = "get"

[[canali]]
url = "https://a.invalid/1.m3u8"
[[canali]]
nome = "Doppio"
url = "https://a.invalid/2.m3u8"
ricetta = "futura"
[[canali]]
nome = "Ignota"
ricetta = "assente"
[[canali]]
nome = "Futura"
ricetta = "futura"
id = "x"
[[canali]]
nome = "Senza id"
ricetta = "futura"
[[canali]]
nome = "Rotta"
ricetta = "rotta"
[[canali]]
nome = 3
url = "https://a.invalid/3.m3u8"
[[canali]]
nome = "Relativo"
url = "/live.m3u8"
[[canali]]
nome = "Buono"
url = "https://a.invalid/4.m3u8"
"#;
    let list = parse(text, || None).unwrap();
    let outcome: Vec<(&str, Result<(), &str>)> =
        channels(&list).iter().map(|c| (c.name.as_str(), c.play.as_ref().map(|_| ()).map_err(String::as_str))).collect();
    assert_eq!(
        outcome,
        [
            (UNNAMED, Err("canale senza `nome`")),
            ("Doppio", Err("canale con sia `url` sia `ricetta`")),
            ("Ignota", Err("ricetta assente inesistente")),
            ("Futura", Err("ricetta futura: passo non valido: decifra x")),
            ("Senza id", Err("ricetta futura: parametro id mancante")),
            ("Rotta", Err("ricetta rotta: `passi` non è un elenco")),
            (UNNAMED, Err("`nome` non è un testo")),
            ("Relativo", Err("`url` non valido: relative URL without a base")),
            ("Buono", Ok(())),
        ]
    );
}

#[test]
fn a_file_that_is_not_toml_is_refused() {
    assert!(parse("[[canali]\nnome = \"A\"", device).unwrap_err().starts_with("riga 1: "));
    assert!(parse("[[canali]]\nnome = \"A\"\nnome = \"B\"\n", device).unwrap_err().starts_with("riga 3: "));
    assert!(parse("<html>404</html>", device).is_err());
    let empty = parse("", device).unwrap();
    assert!(empty.entries.is_empty() && empty.skipped.is_empty());
}

#[test]
fn sections_of_the_wrong_kind_are_skipped() {
    let list = parse("canali = \"tutti\"\nincludi = [1]\n", device).unwrap();
    assert!(list.entries.is_empty());
    assert_eq!(list.skipped, ["`canali` non è un elenco di tabelle", "voce di `includi` che non è una tabella"]);
}

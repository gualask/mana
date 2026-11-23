use super::*;

fn steps(lines: &[&str]) -> Vec<Step> {
    lines.iter().map(|l| Step::parse(l).unwrap()).collect()
}

#[test]
fn raiplay_chain() {
    let recipe = steps(&[
        "get https://www.raiplay.invalid/live/rai1.json",
        "json video.content_url",
        "query output=62",
        "get",
        "json video.0",
    ]);
    let mut asked = Vec::new();
    let url = run(&recipe, |url| {
        asked.push(url.to_string());
        Ok(match url.path() {
            "/live/rai1.json" => br#"{"video":{"content_url":"https://relinker.invalid/r?cont=2606803&output=16"}}"#.to_vec(),
            _ => br#"{"video":["https://cdn.invalid/rai1/master.m3u8?hdnea=x"]}"#.to_vec(),
        })
    })
    .unwrap();
    assert_eq!(url.as_str(), "https://cdn.invalid/rai1/master.m3u8?hdnea=x");
    // The parameter replaces the one already there.
    assert_eq!(asked[1], "https://relinker.invalid/r?cont=2606803&output=62");
}

fn lines(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

#[test]
fn parameters_fill_the_steps_before_they_run() {
    let recipe = lines(&["get https://www.raiplay.invalid/live/{id}.json", "json video.content_url"]);
    let parameter = |name: &str| (name == "id").then(|| "rai2".to_string());
    let steps = instantiate(&recipe, parameter).unwrap();
    assert_eq!(steps[0], Step::Get(Some("https://www.raiplay.invalid/live/rai2.json".into())));
    let missing = instantiate(&lines(&["get https://a.invalid/{channel}"]), parameter);
    assert_eq!(missing, Err(Error::MissingParameter("channel".into())));
    let unknown = instantiate(&lines(&["eval {id}"]), parameter);
    assert_eq!(unknown, Err(Error::InvalidStep("eval rai2".into())));
    assert_eq!(unknown.unwrap_err().text(Language::Italian), "passo non valido: eval rai2");
    // Braces around anything but a name stay, `{json:…}` for `text`.
    let kept = instantiate(&lines(&["text {json:a.b}/{ x }/{id}"]), parameter).unwrap();
    assert_eq!(kept, [Step::Text("{json:a.b}/{ x }/rai2".into())]);
}

#[test]
fn pluto_chain_composes_the_url_from_the_answer() {
    let recipe = lines(&[
        "get https://boot.pluto.invalid/v4/start?clientID={device}&channelID={id}",
        "text {json:servers.stitcher}/v2/stitch/hls/channel/{id}/master.m3u8?{json:stitcherParams}&jwt={json:sessionToken}",
    ]);
    let parameter = |name: &str| match name {
        "id" => Some("5f1ac".to_string()),
        "device" => Some("0b5e6f2a-1c3d-4e5f-8a9b-0c1d2e3f4a5b".to_string()),
        _ => None,
    };
    let mut asked = Vec::new();
    let url = run(&instantiate(&recipe, parameter).unwrap(), |url| {
        asked.push(url.to_string());
        // A query already encoded, inserted as it is.
        Ok(br#"{"servers":{"stitcher":"https://stitcher.invalid"},"stitcherParams":"a=1&b=x%20y","sessionToken":"eyJ.t"}"#.to_vec())
    })
    .unwrap();
    assert_eq!(asked, ["https://boot.pluto.invalid/v4/start?clientID=0b5e6f2a-1c3d-4e5f-8a9b-0c1d2e3f4a5b&channelID=5f1ac"]);
    assert_eq!(url.as_str(), "https://stitcher.invalid/v2/stitch/hls/channel/5f1ac/master.m3u8?a=1&b=x%20y&jwt=eyJ.t");
}

#[test]
fn text_needs_the_fields_it_names() {
    let answer = |_: &Url| Ok(br#"{"a":{"n":3},"t":true}"#.to_vec());
    let number = run(&steps(&["get https://a.invalid/x.json", "text https://b.invalid/{json:a.n}/{json:t}"]), answer);
    assert_eq!(number.unwrap().as_str(), "https://b.invalid/3/true");
    let table = run(&steps(&["get https://a.invalid/x.json", "text https://b.invalid/{json:a}"]), answer);
    assert!(matches!(&table, Err(Error::Step { number: 2, problem: Problem::FieldNotText(path), .. }) if path == "a"));
    assert!(table.unwrap_err().text(Language::Italian).contains("campo a non è un testo"));
    let no_json = run(&steps(&["text https://b.invalid/{json:a}"]), answer);
    assert!(matches!(no_json, Err(Error::Step { problem: Problem::NotJson, .. })));
    assert!(Step::parse("text").is_err() && Step::parse("testo https://b.invalid/").is_err());
}

#[test]
fn rejects_bad_steps_and_results() {
    assert!(Step::parse("eval x").is_err());
    assert!(Step::parse("query noequals").is_err());
    let missing = run(&steps(&["get https://a.invalid/x.json", "json video.url"]), |_| Ok(b"{}".to_vec()));
    assert_eq!(missing.unwrap_err().to_string(), "step 2 (Json(\"video.url\")): field video.url missing");
    let not_url = run(&steps(&["get https://a.invalid/x.json"]), |_| Ok(b"{}".to_vec()));
    assert!(not_url.is_err());
    let too_long = vec![Step::Get(None); MAX_STEPS + 1];
    assert!(run(&too_long, |_| Ok(Vec::new())).is_err());
}

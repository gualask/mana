//! A provisional resolution recipe, from the `resolve = …` lines of a
//! `.stream` file or the `passi` of a recipe in the channel list: the steps
//! that turn a provider's API into the URL to play, run whenever the source
//! is opened or its token expires. Four operations only, on a current
//! value; the executable profile contract (docs/configuration.md, "Ricette
//! componibili") will replace this syntax.
//!
//! - `get [URL]`: downloads URL, or the current value, as JSON or text;
//! - `json PATH`: takes a field of the current JSON, `a.b.0` style;
//! - `query NAME=VALUE`: sets a query parameter of the current URL;
//! - `testo TEMPLATE`: the text of TEMPLATE, where `{json:PATH}` is a field
//!   of the current JSON, inserted as it is.
//!
//! A step written with `{name}` takes the parameter of that name, given by
//! the channel, before it runs (`instantiate`).

use serde_json::Value;
use url::Url;

/// Steps in one recipe: a bound against loops of mistakes, not a feature.
const MAX_STEPS: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Get(Option<String>),
    Json(String),
    Query(String, String),
    Text(String),
}

impl Step {
    pub fn parse(text: &str) -> Result<Self, String> {
        let (operation, argument) = text.trim().split_once(' ').map_or((text.trim(), ""), |(o, a)| (o, a.trim()));
        match operation {
            "get" => Ok(Self::Get((!argument.is_empty()).then(|| argument.to_string()))),
            "json" if !argument.is_empty() => Ok(Self::Json(argument.to_string())),
            "query" => {
                let (name, value) = argument.split_once('=').ok_or(format!("passo non valido: {text}"))?;
                Ok(Self::Query(name.trim().to_string(), value.trim().to_string()))
            }
            "testo" if !argument.is_empty() => Ok(Self::Text(argument.to_string())),
            _ => Err(format!("passo non valido: {text}")),
        }
    }
}

/// The steps written as `lines`, each `{name}` replaced by `parameter(name)`
/// first: a parameter it does not give, or a step that is not one of the
/// operations above, is an error. `{json:…}` is left to `testo`, and
/// braces around anything but a name are text.
pub fn instantiate(lines: &[String], parameter: impl Fn(&str) -> Option<String>) -> Result<Vec<Step>, String> {
    lines.iter().map(|line| Step::parse(&substitute(line, &parameter)?)).collect()
}

/// `line` with its `{name}` placeholders replaced.
fn substitute(line: &str, parameter: &impl Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut text = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|c| open + c) else { break };
        let name = &rest[open + 1..close];
        text.push_str(&rest[..open]);
        if is_name(name) {
            text.push_str(&parameter(name).ok_or_else(|| format!("parametro {name} mancante"))?);
        } else {
            text.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    text.push_str(rest);
    Ok(text)
}

/// Whether `text` names a parameter: letters, digits, `_` and `-`.
fn is_name(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

enum Current {
    Text(String),
    Json(Value),
}

/// Runs `steps` with `fetch` for the downloads; returns the URL they end on.
pub fn run(steps: &[Step], mut fetch: impl FnMut(&Url) -> Result<Vec<u8>, String>) -> Result<Url, String> {
    if steps.len() > MAX_STEPS {
        return Err(format!("ricetta di {} passi, al massimo {MAX_STEPS}", steps.len()));
    }
    let mut current = None;
    for (number, step) in steps.iter().enumerate() {
        let failed = |e: String| format!("passo {} ({step:?}): {e}", number + 1);
        current = Some(match (step, current) {
            (Step::Get(url), current) => {
                let url = match (url, current) {
                    (Some(url), _) => url.clone(),
                    (None, Some(Current::Text(url))) => url,
                    _ => return Err(failed("nessun URL da scaricare".into())),
                };
                let url = Url::parse(&url).map_err(|e| failed(e.to_string()))?;
                let body = fetch(&url).map_err(failed)?;
                match serde_json::from_slice(&body) {
                    Ok(json) => Current::Json(json),
                    Err(_) => Current::Text(String::from_utf8(body).map_err(|_| failed("risposta né JSON né testo".into()))?),
                }
            }
            (Step::Json(path), Some(Current::Json(json))) => match field(&json, path).map_err(failed)? {
                Value::String(text) => Current::Text(text.clone()),
                other => Current::Json(other.clone()),
            },
            (Step::Text(template), current) => {
                let json = match &current {
                    Some(Current::Json(json)) => Some(json),
                    _ => None,
                };
                Current::Text(fill(template, json).map_err(failed)?)
            }
            (Step::Query(name, value), Some(Current::Text(url))) => {
                let mut url = Url::parse(&url).map_err(|e| failed(e.to_string()))?;
                let others: Vec<(String, String)> =
                    url.query_pairs().filter(|(key, _)| key != name).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
                url.query_pairs_mut().clear().extend_pairs(others).append_pair(name, value);
                Current::Text(url.into())
            }
            (_, _) => return Err(failed("il valore corrente non è del tipo atteso".into())),
        });
    }
    match current {
        Some(Current::Text(url)) => Url::parse(&url).map_err(|e| format!("risultato non è un URL: {e}")),
        _ => Err("la ricetta non termina con un URL".into()),
    }
}

/// The field at `path`, `a.b.0` style, of `json`.
fn field<'a>(json: &'a Value, path: &str) -> Result<&'a Value, String> {
    path.split('.')
        .try_fold(json, |value, key| match key.parse::<usize>() {
            Ok(index) => value.get(index),
            Err(_) => value.get(key),
        })
        .ok_or_else(|| format!("campo {path} assente"))
}

/// `template` with each `{json:PATH}` replaced by that field of `json`, as
/// it is: a text without its quotes, a number or a boolean as written.
fn fill(template: &str, json: Option<&Value>) -> Result<String, String> {
    let mut text = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{json:") {
        let close = rest[open..].find('}').map(|c| open + c).ok_or("`{json:` senza `}`")?;
        let path = &rest[open + "{json:".len()..close];
        let json = json.ok_or("il valore corrente non è JSON")?;
        text.push_str(&rest[..open]);
        match field(json, path)? {
            Value::String(value) => text.push_str(value),
            value @ (Value::Number(_) | Value::Bool(_)) => text.push_str(&value.to_string()),
            _ => return Err(format!("campo {path} non è un testo")),
        }
        rest = &rest[close + 1..];
    }
    text.push_str(rest);
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(lines: &[&str]) -> Vec<Step> {
        lines.iter().map(|l| Step::parse(l).unwrap()).collect()
    }

    #[test]
    fn raiplay_chain() {
        let recipe = steps(&[
            "get https://www.raiplay.invalid/dirette/rai1.json",
            "json video.content_url",
            "query output=62",
            "get",
            "json video.0",
        ]);
        let mut asked = Vec::new();
        let url = run(&recipe, |url| {
            asked.push(url.to_string());
            Ok(match url.path() {
                "/dirette/rai1.json" => br#"{"video":{"content_url":"https://relinker.invalid/r?cont=2606803&output=16"}}"#.to_vec(),
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
        let recipe = lines(&["get https://www.raiplay.invalid/dirette/{id}.json", "json video.content_url"]);
        let parameter = |name: &str| (name == "id").then(|| "rai2".to_string());
        let steps = instantiate(&recipe, parameter).unwrap();
        assert_eq!(steps[0], Step::Get(Some("https://www.raiplay.invalid/dirette/rai2.json".into())));
        let missing = instantiate(&lines(&["get https://a.invalid/{canale}"]), parameter);
        assert_eq!(missing, Err("parametro canale mancante".into()));
        let unknown = instantiate(&lines(&["eval {id}"]), parameter);
        assert_eq!(unknown, Err("passo non valido: eval rai2".into()));
        // Braces around anything but a name stay, `{json:…}` for `testo`.
        let kept = instantiate(&lines(&["testo {json:a.b}/{ x }/{id}"]), parameter).unwrap();
        assert_eq!(kept, [Step::Text("{json:a.b}/{ x }/rai2".into())]);
    }

    #[test]
    fn pluto_chain_composes_the_url_from_the_answer() {
        let recipe = lines(&[
            "get https://boot.pluto.invalid/v4/start?clientID={dispositivo}&channelID={id}",
            "testo {json:servers.stitcher}/v2/stitch/hls/channel/{id}/master.m3u8?{json:stitcherParams}&jwt={json:sessionToken}",
        ]);
        let parameter = |name: &str| match name {
            "id" => Some("5f1ac".to_string()),
            "dispositivo" => Some("0b5e6f2a-1c3d-4e5f-8a9b-0c1d2e3f4a5b".to_string()),
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
        let number = run(&steps(&["get https://a.invalid/x.json", "testo https://b.invalid/{json:a.n}/{json:t}"]), answer);
        assert_eq!(number.unwrap().as_str(), "https://b.invalid/3/true");
        let table = run(&steps(&["get https://a.invalid/x.json", "testo https://b.invalid/{json:a}"]), answer);
        assert!(table.unwrap_err().contains("campo a non è un testo"));
        let no_json = run(&steps(&["testo https://b.invalid/{json:a}"]), answer);
        assert!(no_json.unwrap_err().contains("il valore corrente non è JSON"));
        assert!(Step::parse("testo").is_err());
    }

    #[test]
    fn rejects_bad_steps_and_results() {
        assert!(Step::parse("eval x").is_err());
        assert!(Step::parse("query noequals").is_err());
        let missing = run(&steps(&["get https://a.invalid/x.json", "json video.url"]), |_| Ok(b"{}".to_vec()));
        assert!(missing.unwrap_err().contains("campo video.url assente"));
        let not_url = run(&steps(&["get https://a.invalid/x.json"]), |_| Ok(b"{}".to_vec()));
        assert!(not_url.is_err());
        let too_long = vec![Step::Get(None); MAX_STEPS + 1];
        assert!(run(&too_long, |_| Ok(Vec::new())).is_err());
    }
}

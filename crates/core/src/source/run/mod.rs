//! Runs a checked source: its catalogue, the candidates of an item, the
//! resolution of a candidate to the media to play. The network comes from
//! the caller (`Fetch`), so the same runs serve the apps and the tests;
//! limits bound every run, whatever the source asks.

use std::collections::{BTreeMap, BTreeSet};

use url::Url;

use super::{Catalogue, Flow, Headers, Item, Resolve, Source, Use};
use crate::language::{Language, Localized, display_in_english};
use crate::net::{self, Answer};
use crate::text;

mod ops;
mod value;

use value::{Scope, Value};

/// Requests in one run: a catalogue, the candidates of an item or one
/// resolution, its chain included.
pub const MAX_REQUESTS: usize = 16;
/// Candidates routed one after another in one resolution.
pub const MAX_CHAIN: usize = 8;
/// Recipes running inside one another.
pub const MAX_CALL_DEPTH: usize = 4;
/// Entries a list may have.
pub const MAX_ENTRIES: usize = 5000;
/// The largest answer a step reads.
pub const MAX_ANSWER_BYTES: u64 = net::MAX_TEXT_BYTES;

/// A request a step makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// GET or POST.
    pub method: &'static str,
    pub url: Url,
    /// The step's own headers, on top of the client's.
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// A stream an item offers, before it is resolved: `id`, then its other
/// fields, all text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub fields: BTreeMap<String, String>,
}

impl Candidate {
    /// Its `id`, which tells it among its item's, also when they are read
    /// again.
    pub fn id(&self) -> Option<&str> {
        self.fields.get("id").map(String::as_str)
    }

    /// How the candidate at `index` of its item's reads: its label, else its
    /// id, else its number.
    pub fn label(&self, index: usize) -> String {
        let field = |key: &str| self.fields.get(key).filter(|v| !v.is_empty()).cloned();
        field("label").or_else(|| field("id")).unwrap_or_else(|| format!("{}", index + 1))
    }
}

/// The media to play, at the end of a resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub url: Url,
    /// For manifests, keys and segments.
    pub headers: Vec<(String, String)>,
    /// `hls`, `dash` or `progressive`, as the source declares: a hint.
    pub format: Option<String>,
    /// As the source gives it, when it does.
    pub expires_at: Option<String>,
}

/// Runs the flows of one source for one device.
pub struct Engine<'a> {
    source: &'a Source,
    device: String,
    /// The network.
    fetch: Box<dyn FnMut(&Request) -> Result<Answer, net::Error> + 'a>,
    /// Whether the run must give up: the viewer moved on.
    cancelled: Box<dyn Fn() -> bool + 'a>,
    requests: usize,
    depth: usize,
}

impl<'a> Engine<'a> {
    /// An engine for `source` on the device `device`, with `fetch` for the
    /// requests and `cancelled` checked before every step.
    pub fn new(
        source: &'a Source,
        device: &str,
        fetch: impl FnMut(&Request) -> Result<Answer, net::Error> + 'a,
        cancelled: impl Fn() -> bool + 'a,
    ) -> Self {
        Self { source, device: device.into(), fetch: Box::new(fetch), cancelled: Box::new(cancelled), requests: 0, depth: 0 }
    }

    /// The source's channels or events; none without a catalogue.
    pub fn catalogue(&mut self) -> Result<Vec<Item>, Failure> {
        self.requests = 0;
        let flow = match &self.source.catalogue {
            None => return Ok(Vec::new()),
            Some(Catalogue::Items(items)) => return Ok(items.clone()),
            Some(Catalogue::Steps(flow)) => flow,
        };
        let entries = self.flow("catalogue", flow, self.inputs(None, None))?;
        let fields = entries.into_iter().map(|fields| fields_of(fields, &["id", "title"]));
        fields.map(|fields| fields.map(|fields| Item { fields }).map_err(|detail| Failure::changed("catalogue", detail))).collect()
    }

    /// The streams `item` offers, in the source's order; the item itself
    /// when the source has no `sources`.
    pub fn sources(&mut self, item: &Item) -> Result<Vec<Candidate>, Failure> {
        self.requests = 0;
        let Some(flow) = &self.source.sources else { return Ok(vec![Candidate { fields: item.fields.clone() }]) };
        let entries = self.flow("sources", flow, self.inputs(Some(item), None))?;
        let candidates: Vec<_> = entries
            .into_iter()
            .map(|fields| fields_of(fields, &["id"]).map(|fields| Candidate { fields }).map_err(|detail| Failure::changed("sources", detail)))
            .collect::<Result<_, _>>()?;
        if candidates.is_empty() {
            return Err(Failure { outcome: Outcome::NoSources, at: "sources".into(), cause: Cause::None });
        }
        Ok(candidates)
    }

    /// The media `candidate` of `item` leads to, following the next
    /// candidates resolvers give.
    pub fn resolve(&mut self, candidate: &Candidate, item: &Item) -> Result<Resolved, Failure> {
        self.requests = 0;
        let mut candidate = candidate.clone();
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_CHAIN {
            let url = candidate.fields.get("url").cloned().unwrap_or_default();
            if !url.is_empty() && !seen.insert(url.clone()) {
                return Err(Failure { outcome: Outcome::Limit, at: "resolve".into(), cause: Cause::Problem("a candidate leads back to itself".into()) });
            }
            let (at, flow) = match &self.source.resolve {
                Resolve::Steps(flow) => ("resolve".to_string(), flow),
                Resolve::Routes(routes) => {
                    let Some(route) = routes.iter().find(|route| matches(&route.when, &candidate)) else {
                        let host = Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or(url);
                        return Err(Failure { outcome: Outcome::UnsupportedHost, at: "resolve.routes".into(), cause: Cause::Problem(host) });
                    };
                    match &route.to {
                        Use::Stream(headers) => return self.direct(&candidate, headers.as_ref()),
                        Use::Resolver(name) => (format!("resolvers.{name}"), &self.source.resolvers[name]),
                    }
                }
            };
            match self.flow_value(&at, flow, self.inputs(Some(item), Some(&candidate)))? {
                Value::Stream(resolved) => return Ok(resolved),
                Value::Next(fields) => candidate = Candidate { fields },
                other => return Err(Failure::changed(&at, format!("ends on {}", other.name()))),
            }
        }
        Err(Failure { outcome: Outcome::Limit, at: "resolve".into(), cause: Cause::Problem(format!("more than {MAX_CHAIN} candidates")) })
    }

    /// A route's `use = "stream"`: the candidate's URL is the media.
    fn direct(&mut self, candidate: &Candidate, headers: Option<&Headers>) -> Result<Resolved, Failure> {
        let at = "resolve.routes";
        let url = candidate.fields.get("url").ok_or_else(|| Failure::changed(at, "candidate without `url`".into()))?;
        let url = Url::parse(url).map_err(|e| Failure::changed(at, format!("not a URL: {e}")))?;
        let inputs = BTreeMap::new();
        let steps = BTreeMap::new();
        let scope = Scope { inputs: &inputs, steps: &steps, entry: None };
        let headers = self.headers(headers, &scope).map_err(|detail| Failure::changed(at, detail))?;
        Ok(Resolved { url, headers, format: None, expires_at: None })
    }

    fn inputs(&self, item: Option<&Item>, candidate: Option<&Candidate>) -> BTreeMap<String, Value> {
        let record = |fields: &BTreeMap<String, String>| Value::Record(fields.iter().map(|(k, v)| (k.clone(), Value::Text(v.clone()))).collect());
        let mut inputs = BTreeMap::from([("device".to_string(), Value::Text(self.device.clone()))]);
        if let Some(item) = item {
            inputs.insert("item".into(), record(&item.fields));
        }
        if let Some(candidate) = candidate {
            inputs.insert("candidate".into(), record(&candidate.fields));
        }
        inputs
    }

    /// The entries a catalogue or sources flow ends on.
    fn flow(&mut self, at: &str, flow: &Flow, inputs: BTreeMap<String, Value>) -> Result<Vec<Value>, Failure> {
        let value = self.flow_value(at, flow, inputs)?;
        value.list().map_err(|detail| Failure::changed(at, detail))
    }

    /// What the last step of `flow` gives.
    fn flow_value(&mut self, at: &str, flow: &Flow, inputs: BTreeMap<String, Value>) -> Result<Value, Failure> {
        let mut steps = BTreeMap::new();
        let mut last = None;
        for step in &flow.steps {
            if (self.cancelled)() {
                return Err(Failure { outcome: Outcome::Cancelled, at: at.into(), cause: Cause::None });
            }
            let step_at = format!("{at}.{}", step.id);
            let value = ops::run(self, &step_at, &step.operation, &inputs, &steps)?;
            steps.insert(step.id.clone(), value.clone());
            last = Some(value);
        }
        last.ok_or_else(|| Failure::changed(at, "no steps".into()))
    }

    /// Runs `recipe` with `inputs`, within the depth allowed.
    fn call(&mut self, at: &str, recipe: &str, inputs: BTreeMap<String, Value>) -> Result<Value, Failure> {
        if self.depth >= MAX_CALL_DEPTH {
            return Err(Failure { outcome: Outcome::Limit, at: at.into(), cause: Cause::Problem(format!("recipes {MAX_CALL_DEPTH} deep")) });
        }
        let flow = &self.source.recipes[recipe].flow;
        let mut inputs = inputs;
        inputs.insert("device".into(), Value::Text(self.device.clone()));
        self.depth += 1;
        let result = self.flow_value(&format!("recipes.{recipe}"), flow, inputs);
        self.depth -= 1;
        result
    }

    fn send(&mut self, at: &str, request: Request) -> Result<Answer, Failure> {
        if self.requests >= MAX_REQUESTS {
            return Err(Failure { outcome: Outcome::Limit, at: at.into(), cause: Cause::Problem(format!("more than {MAX_REQUESTS} requests")) });
        }
        self.requests += 1;
        (self.fetch)(&request).map_err(|error| Failure::network(at, error))
    }

    /// A header set by name, or a table of templates, as text.
    fn headers(&self, headers: Option<&Headers>, scope: &Scope) -> Result<Vec<(String, String)>, String> {
        match headers {
            None => Ok(Vec::new()),
            Some(Headers::Set(name)) => Ok(self.source.headers[name].iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            Some(Headers::Table(table)) => table.iter().map(|(name, template)| Ok((name.clone(), scope.text(template)?))).collect(),
        }
    }
}

/// An entry as text fields, with the `required` ones.
fn fields_of(entry: Value, required: &[&str]) -> Result<BTreeMap<String, String>, String> {
    let fields: BTreeMap<String, String> = match entry {
        Value::Record(record) => record.into_iter().map(|(k, v)| v.text().map(|text| (k, text))).collect::<Result<_, _>>()?,
        Value::Json(serde_json::Value::Object(map)) => {
            map.into_iter().filter_map(|(k, v)| Value::Json(v).text().ok().map(|text| (k, text))).collect()
        }
        other => return Err(format!("an entry that is {}", other.name())),
    };
    match required.iter().find(|field| !fields.contains_key(**field)) {
        Some(missing) => Err(format!("an entry without `{missing}`")),
        None => Ok(fields),
    }
}

/// Whether `candidate` has what `when` asks: its `type`, and its URL's
/// host and path, exactly.
fn matches(when: &super::When, candidate: &Candidate) -> bool {
    let url = candidate.fields.get("url").and_then(|url| Url::parse(url).ok());
    let host = url.as_ref().and_then(|url| url.host_str());
    let path = url.as_ref().map(Url::path);
    when.kind.as_ref().is_none_or(|kind| candidate.fields.get("type") == Some(kind))
        && when.host.as_deref().is_none_or(|h| host == Some(h))
        && when.path.as_deref().is_none_or(|p| path == Some(p))
}

/// Why a run gave no result, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub outcome: Outcome,
    /// The flow and the step: `resolvers.envelope.key`.
    pub at: String,
    pub cause: Cause,
}

/// The outcomes of docs/architecture.md a run can end on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    NetworkError,
    /// 401, 403 or 410: the server refused, the cause may be unknown.
    AccessDenied,
    RateLimited,
    /// The answers no longer fit the source: a field missing, a pattern
    /// without a match, a decryption that does not authenticate.
    SourceChanged,
    UnsupportedHost,
    NoSources,
    /// A limit of the engine reached.
    Limit,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    None,
    Network(net::Error),
    /// What went wrong, in English: technical, as the libraries say.
    Problem(String),
}

impl Failure {
    fn changed(at: &str, detail: String) -> Self {
        Self { outcome: Outcome::SourceChanged, at: at.into(), cause: Cause::Problem(detail) }
    }

    fn limit(at: &str, detail: String) -> Self {
        Self { outcome: Outcome::Limit, at: at.into(), cause: Cause::Problem(detail) }
    }

    fn network(at: &str, error: net::Error) -> Self {
        let outcome = match error.status() {
            Some(401 | 403 | 410) => Outcome::AccessDenied,
            Some(429) => Outcome::RateLimited,
            _ => Outcome::NetworkError,
        };
        Self { outcome, at: at.into(), cause: Cause::Network(error) }
    }
}

impl Localized for Failure {
    fn text(&self, language: Language) -> String {
        let at = &self.at;
        let outcome = match self.outcome {
            Outcome::NetworkError => text!(language, "source-network-error", at = at),
            Outcome::AccessDenied => text!(language, "source-access-denied", at = at),
            Outcome::RateLimited => text!(language, "source-rate-limited", at = at),
            Outcome::SourceChanged => text!(language, "source-changed", at = at),
            Outcome::UnsupportedHost => text!(language, "source-unsupported-host", at = at),
            Outcome::NoSources => text!(language, "source-no-sources", at = at),
            Outcome::Limit => text!(language, "source-limit", at = at),
            Outcome::Cancelled => text!(language, "source-cancelled", at = at),
        };
        match &self.cause {
            Cause::None => outcome,
            Cause::Network(error) => format!("{outcome}: {}", error.text(language)),
            Cause::Problem(detail) => format!("{outcome}: {detail}"),
        }
    }
}

display_in_english!(Failure);

#[cfg(test)]
mod tests;

# Configuration and extensions

The requirement, linked to the [Modular architecture](architecture.md), is
to keep in the source files everything that can be composed from
capabilities already available. **A new provider must not, by default, mean
a new Rust module.** This document defines the source file: what belongs in
it, its contract, how it runs, and what is not built yet. The contract is
read and checked by [`source/`](../crates/core/src/source/mod.rs) and run
by [`source/run/`](../crates/core/src/source/run/mod.rs); the channel list
names sources ([Channel list](channel-list.md#format)).

## What belongs in the files

| Change | Where it is expressed |
|---|---|
| Domain, path, GET/POST, query/body, headers | Steps of a source or of a shared recipe |
| JSON fields, HTML attributes, Next Flight and Protobuf data | Reading operations and `list.map` |
| Catalogue → detail → nested list of streams | The `catalogue`, `sources` and `resolve` flows |
| Hosts that follow the same protocol | Routes → the same resolver |
| Extraction of a URL parameter or of a raw portion of the query | `url.param` or `url.query_after` |
| Preference between streams, retry, fallback and renewal | The application session, the same for every source |
| A transformation already available, cryptographic ones included | Parameters and composition of existing operations |
| Algorithm, parser or transport missing from the engine | New operation with its own contract |
| New decoding codec/backend | Media and platform support, with a verified capability |

Before adding an operation, look for a composition of existing ones. If a
reusable capability is missing, add a primitive with input/output and
parameters ([operation.rs](../crates/core/src/source/operation.rs)).
The presence of AES, on its own, does not force the provider to be coded in
Rust. No `eval`, shell or arbitrary JavaScript is introduced in the source
files; an algorithm that requires a complete program belongs in a new
operation, with its contract and its tests.

## Source files

A source is one TOML file; JSON with the same data gives the same source.
It has no version: an operation never changes meaning, a new meaning takes
a new name, and a file using an operation this version of Mana does not
know is rejected. [`config/examples/`](../config/examples/) holds one
source for each chain of the [lab](local-lab.md), and
[`rejected/`](../config/examples/rejected/) one case for each rule of
[validation](#validation), named after its error.

```toml
id = "lab-events"           # letters, digits, - and _
name = "Lab events"

[headers.media]             # named header sets, used by steps and streams
User-Agent = "StreamLab/1"

[catalogue]                 # optional: no input        → list of items
[sources]                   # optional: input.item      → list of candidates
[resolve]                   # required: input.candidate → stream or next candidate
[resolvers.<name>]          # chosen by the routes of resolve
[recipes.<name>]            # inputs = [...], run with `call`
```

A flow is a list of `steps`. `catalogue` may instead list `items` written in
the file; `resolve` may instead hold `routes`.

| Model | Fields |
|---|---|
| Item | `id`, `title`; `group`, `logo`, `starts_at` when known; other text fields for the next flows |
| Candidate | `id`; `label`, `language`, `quality` when known; other text fields (`url`, `type`…) |
| Stream | `url`; `headers` for manifests, keys and segments; `format` as a hint (`hls`, `dash`, `progressive`); `expires_at` when known |
| Next | a new candidate, with at least `id` and `url`, routed again |

A source with a `catalogue` gives its items as channels or events: a "Rai"
source lists Rai 1, Rai 2… from RaiPlay. Without `sources`, an item is its
only candidate.

### Steps and references

Each step has `id`, `op` and the operation's parameters; the last step is
the flow's result. A step reads the flow's inputs and the steps before it,
so a cycle cannot be written inside a flow. Parameters are templates:

- `{step}`, `{step.a.b.0}` into a JSON value or a record, `{input.item.id}`,
  `{input.candidate.url}`, `{input.device}` (the app's device UUID, in
  every flow), `{item.x}` inside `list.map`;
- a field whose name has other characters than letters, digits, `-` and
  `_` is quoted: `{config."common:web:client"}`;
- a reference may hold another, resolved first: `{pool.{order.0}}`;
- the response of `http` has `.body` (bytes), `.text`, `.json`,
  `.header.Name` and `.url` (after redirects);
- a template that is one reference keeps the value's kind (bytes, a list…);
  any other template is text, with the values inserted as they are: a
  signed URL is never re-encoded or truncated, and bytes are read as UTF-8.
  `{x|component}` percent-encodes a value as one URL component;
- a `{` not followed by a letter or `_` is text, so a JSON body needs no
  escaping.

Routes are tried in order on the candidate: `when = { type, host, path }`
compares its `type` field and the host and path of its `url`, exactly;
`use` names a resolver, or `"stream"` when the candidate's `url` is already
the media, with optional `headers`. No match ends in `unsupported_host`.
There is no conditional between steps: choices live only in routes.

### Vocabulary

| Operation | In → out | Parameters |
|---|---|---|
| `http` | → response | `url`, `method` (GET/POST), `query`, `body`, `headers` (a set's name, or a table of templates) |
| `json` | text/bytes → JSON | `from` |
| `m3u` | text → list of entries (title, url, logo, group) | `from` |
| `html.attr` | text → text, or list with `all = true` | `from`, `selector` (CSS), `attribute`, `base` to resolve a relative URL |
| `nextflight` | text → JSON | `from`, `key`: the value after `"key":` in the payloads pushed to `self.__next_f` |
| `protobuf` | bytes → record (field number → values; an absent field is an empty list, as in Protobuf) | `from` |
| `text.match` | text → text | `from`, `pattern` (one group) |
| `list.map` | list → list | `list`; `item` (each field a template, `{ from, pattern }`, or `{ from, format, zone }` as `date` reads it), or `call` and `input` to make each entry the result of a recipe (`{item}` is the entry); `where` (fields equal to templates); `present` (fields there and not empty) |
| `text` | → text | `template` |
| `record` | → item fields | `fields`, each as an `item` field of `list.map`: the record a recipe gives for each entry of `list.map` with `call`, when its fields are nested deeper than a template over the entry reaches |
| `date` | text → text | `from`, `format` (`%Y %m %d %H %M %S` and text, or `unix`, `unix_ms`), `zone` (`+08:00`, UTC when absent): the instant as RFC 3339 in UTC, as an item's `starts_at` |
| `url.param` | text → text (decoded) | `from`, `name` |
| `url.query_after` | text → text (raw, to the end) | `from`, `name` |
| `url.set` | text → text | `from`, `name`, `value`: one query parameter set, the others kept |
| `base64.decode` / `base64.encode` | text ↔ bytes | `from`, `alphabet` (`standard`, `url`), `padding = false` to encode without `=` |
| `hex.decode` / `hex.encode` | text ↔ bytes | `from` |
| `rot47` | text → text | `from` |
| `bytes.xor` | bytes, bytes → bytes | `a`, `b` |
| `md5` | text/bytes → hex text | `from` |
| `aes.decrypt` | bytes → bytes | `mode` (`gcm`, `ctr`, `cbc`), `key` and `iv` (bytes, or text as UTF-8), `data`, `aad` (gcm) |
| `aes.encrypt` | bytes → bytes | `mode` (`ctr`), `key`, `iv`, `data` |
| `call` | → the recipe's result | `recipe`, `input` (the recipe's `inputs`) |
| `stream` | → stream | `url`, `headers`, `format`, `expires_at` |
| `next` | → next candidate | `candidate` (a table: a flat `id` would be the step's) |

The vocabulary was checked on 23 November with a throwaway interpreter, to
six seconds of video and audio decoded by FFmpeg with the stream's
headers: the seven lab examples, and real sources (the 15 channels of a Rai
catalogue, of which Rai 4K is DASH; Pluto's guide; TV8). The chains come
from the cases studied: event catalogues with several signals per event,
iframes on several hosts, an AES-GCM envelope in a player page, a raw query
after `url=`, Next Flight data, and an app answering Protobuf with signed
requests, ROT47 and an AES-CTR token in the path (`lab-binary`).

### Validation

Before a source can be used, [validate.rs](../crates/core/src/source/validate.rs)
rejects, naming the flow and the step:

- an unknown operation or field, a missing one, a value of the wrong form;
- a reference to a step that does not come earlier, to an input the flow
  does not have, or to a field a value cannot have;
- a value that cannot fit where it goes, when that is known before running
  (a list where bytes are needed, bytes pasted into a text); values read
  from JSON or Protobuf are checked when they run;
- a header set, resolver or recipe that does not exist, a `call` whose
  inputs are not those the recipe names;
- duplicate step ids, two routes with the same `when`, recipes calling
  themselves;
- a pattern without exactly one group, a selector lol_html does not accept;
- a flow whose last step cannot give its result: a list with `id` (and
  `title` for a catalogue, also in the `record` a `list.map` recipe ends
  on), or a stream, a next candidate or a `call`.

The [JSON Schema](../schemas/source.schema.json) describes the structure
for editors; the rules beyond it are the Rust validation's.

### Running

[`Engine`](../crates/core/src/source/run/mod.rs) runs a checked source:
`catalogue` gives its items, `sources` the candidates of an item, `resolve`
the stream a candidate leads to, following the next candidates resolvers
give. The network comes from the caller, as a function from a request to
an answer: the apps pass `net::Client::send`, the tests a network that
answers as the lab does. A run checks before each step whether it was
cancelled. Each run is bounded, whatever the source asks: 16 requests, 8
candidates in a chain, recipes 4 deep, 5,000 entries in a list (the
constants of `run/mod.rs`); the caller reads answers of at most
`MAX_ANSWER_BYTES`, 4 MB. A run ends on one of the
outcomes of the [architecture](architecture.md#extensions-and-compatibility),
with the flow and the step: `network_error`, `access_denied` (401, 403,
410), `rate_limited` (429), `source_changed` (a field missing, a pattern
without a match, a decryption that does not authenticate), `unsupported_host`,
`no_sources`, a limit reached, cancelled. A candidate leading back to
itself is a limit.

The engine does not check the media: whether the stream plays is the
player's to say, and choosing another candidate when it does not is the
viewer's ([Channel list](channel-list.md#playing-a-channel-with-a-source)).

## Composable resolvers

The main flows stay simple, and the branches with their own
responsibilities are named resolvers, chosen by routes, or recipes run with
`call`, which several resolvers can share. Each resolver returns a stream
or the next candidate to route; the engine continues the chain and applies
its limits, and the resolver does not need to know the later destinations.
In [lab-events](../config/examples/lab-events.toml) the envelope resolver
returns a player URL, and the next route extracts `liveUrl` and gives the
stream.

A recipe is extracted when it has a recognisable responsibility, changes
independently of its callers or is shared: changing the parser of a
URL-parameter player changes that resolver and its example, and the
sources that route to it keep using it.

Retry, renewal and switching to the alternative stream are decisions of the
[application session](architecture.md#modules-and-decision-ownership).
Sources expose the steps to obtain a new link; the recovery policy is shared.

## Profiles, state and versions

These are distinct:

- source files: steps, routes and recipes;
- app settings: user and device preferences;
- temporary contexts: cookies, tokens, URLs and renewal state, kept in
  memory by the runtime.

A source describes how to obtain a token or renew a link; temporary values
do not become configuration. Headers are those of the step or the stream
that names them; a provider's HTTP context is not extended to other
domains.

One loading pipeline, whatever the format:

```text
file → TOML (or JSON) parser → common data model → Source → validation
```

The data model is the one TOML and JSON share
([`Source::from_toml`](../crates/core/src/source/mod.rs)); the engine will
receive the checked `Source` and never read extensions. Errors name the
flow, the step and the field, and a TOML error its line.

## JSON, TOML and YAML

**TOML is the format of source files**, because they are recipes to read,
comment and change over time: comments sit next to the parameters, steps
are ordered arrays of tables, and literal strings suit patterns and
backslashes ([TOML 1.0](https://toml.io/en/v1.0.0)). JSON stays useful for
exchange and generated data, with the same data model. YAML adds choices on
types, tags and aliases with no need yet.

TOML's practical limit is deep trees of branches: section paths grow
longer, and a sub-table of an array binds to its most recent element.
Named resolvers and recipes keep the files flat, so this limit does not
justify making TOML secondary now.

The common data model admits tables with text keys, lists, texts and
booleans; a TOML date where a text is expected is an invalid value, and
duplicate keys are a TOML error.

## Not built yet

- JSON import into the same `Source`, with the file in the errors.
- Catalogue pagination: no case has needed it.

The order is in [Next steps](todo.md).

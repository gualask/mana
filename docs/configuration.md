# Configuration and extensions

Proposed direction of 15 November 2026, linked to the [Modular architecture](architecture.md).
The requirement is to keep in the profiles everything that can be composed
from capabilities already available. **A new provider must not, by default,
mean a new Rust module.**

The [current schema draft](../schemas/source.schema.json) does not implement
the language described here yet. This document sets the criteria for
extending it together with the future engine, without presenting new fields
as already supported.

## What belongs in the files

| Change | Where it is expressed |
|---|---|
| Domain, path, GET/POST, query/body, headers | Source profile or shared recipe |
| JSON fields, HTML selectors, metadata, dates and time zone | Mapping with operations already implemented |
| Catalogue → detail → nested list of streams | Flows and references between their inputs/outputs |
| Hosts that follow the same protocol | Host rule → reference to the same recipe/resolver |
| Extraction of a URL parameter or of a raw portion of the query | Explicit operation with a declared decoding mode |
| Preference between streams, retry, fallback and renewal | Profile policy executed by the application |
| A transformation already available, cryptographic ones included | Parameters and composition of existing operations |
| Algorithm, parser or transport missing from the engine | New operation or adapter with its own contract |
| New decoding codec/backend | Media and platform support, with a verified capability |

Before creating an adapter, look for a composition of existing operations.
If a reusable capability is missing, add a primitive with input/output and a
parameter schema. Provider-specific code remains for protocols or algorithms
that cannot be described clearly with that vocabulary. The presence of AES,
on its own, does not force the whole provider to be coded in Rust.

The lab offers synthetic encrypted envelopes and binary sessions.
The [test client's functions](../dev/lab/codecs.py) check the data, but they
are not yet operations of the future runtime. The vocabulary must be defined
and validated before profiles are made executable.

## Composable recipes

The source file declares the public operations `catalogue`, `sources` and
`resolve`; renewal calls a recipe or an explicit adapter. The input and
output contracts are defined in the [Modular architecture](architecture.md#common-contract).

A recipe contains steps with an ID, operation/version, parameters, explicit
input and named output. Steps can refer to the flow's inputs and to earlier
results; the loader rejects missing references, type mismatches and cycles.
The engine applies common limits to the number of requests, response size,
pages, candidates and depth.

The initial vocabulary must cover:

- HTTP and reading of JSON, M3U, HTML and Next Flight data;
- selection of fields and lists, mapping, default values, simple filters and
  declared string/date transformations;
- construction of URLs, queries and paths with explicit encoding;
- choice between cases with limited conditions, such as field present or
  value equal;
- calls to shared recipes or registered adapters;
- production of the normalised models and resolution of subsequent
  candidates.

These are required capabilities, not names of operations already
implemented. No `eval`, shell or arbitrary JavaScript is introduced in the
profiles. The necessary iterations, such as pagination and mapping of items,
are limited operations of the engine. An algorithm that requires a complete
program belongs in the extension, with its contract and its tests.

A single host rule chooses the recipe. Conflicts or ambiguous rules are
loading errors. Common recipes, for example a shared JSON API resolver, are
referenced by ID and revision from several sources. Parameters and overrides
are explicit and validated; there is no implicit merging of files or
arbitrary inheritance. Choosing an unknown host produces an outcome on the
candidate.

Templates have explicit namespaces and transformations: an ID inserted into
the path is encoded once only, as a component. The standard extraction of
`liveUrl` and the raw query after `url=` are two different operations, with
distinct extraction contracts. No general rule may automatically truncate or
re-encode a URL that is already signed.

### Composable resolvers

Decided approach: the main flows stay simple, and the branches with their own
responsibilities call named recipes, which can be shared between providers.
The [resolution contract](architecture.md#step-by-step-resolution) lets each
resolver return a media resource or the next link/candidate to resolve. The
engine continues the chain and applies its limits; the resolver does not need
to know all the subsequent destinations.

A synthetic source exposes catalogue and details; the envelope resolver
returns a player URL; a subsequent recipe extracts the `liveUrl` parameter
and produces the HLS resource. The lab provides this chain, while the recipe
engine is still to be built.

Example of TOML dispatch, with proposed syntax:

```toml
[[routes]]
when = { path = "/wrapper" }
use = "synthetic-envelope@1"

[[routes]]
when = { path = "/player" }
use = "query-parameter@1"
```

A recipe is extracted when it has a recognisable responsibility, changes
independently of its callers or can be shared by several sources.
Simple local conditions stay in the recipe. Changing the parser of a
URL-parameter player, for example, requires a change to the resolver and to
its fixtures; the providers that call it keep using it through the compatible
reference. One file per condition is not needed.

Retry, renewal and switching to the alternative stream are decisions of the
[application session](architecture.md#modules-and-decision-ownership),
parametrised by the profile. Recipes expose the specific steps to obtain a
new link; the recovery policy is shared.

## Profiles, state and versions

These are distinct:

- source profiles: endpoints, mappings and flows;
- resolver recipes: steps shared per host/protocol;
- app settings: user and device preferences;
- temporary contexts: cookies, tokens, URLs and renewal state, managed by the
  runtime.

The static file describes how to obtain a token or renew a link. Temporary
values do not become permanent configuration. Any user credential is
referenced through a pointer to a store outside the profile. Headers are
scoped by role and host; a provider's HTTP context is not extended
automatically to other domains.

The future contract must distinguish the schema version, the profile
revision and the version of the operations/adapters. The loader checks all
the required capabilities and also validates the adapter-specific
parameters. Unknown IDs produce an error before activation. A change of
semantics requires a compatible version or an explicit migration.

There is a single loading pipeline:

```text
file → format parser → common ProfileSpec
     → structural and semantic validation → ExecutionPlan
     → execution with the registry of available capabilities
```

`ProfileSpec` and `ExecutionPlan` are types proposed by the design. The
engine receives the typed plan and does not interpret `.json`, `.toml` or
`.yaml` extensions. Errors must keep the file and the path of the
field/step, so that a wrong profile can be fixed without following internal
logs.

## JSON, TOML and YAML

**The recommendation is TOML as the main format for profiles written and
maintained by hand**, because they are recipes to read, comment and change
over time. JSON stays useful for exchange, generation and the examples of the
current draft. The Rust parser has not been chosen yet.

| Format | Assessment for this project | Proposed direction |
|---|---|---|
| TOML | Comments next to the parameters; ordered steps with arrays of tables; literal strings useful for patterns and backslashes | Main format for sources and shared recipes |
| JSON | Natural for recursive structures and generated data; standard syntax without comments | Interoperability and fixtures; import support stays in the loader |
| YAML | Compact syntax for nested structures, with extra choices on types, tags and aliases | No need has emerged for the first runtime |

TOML suits flows because the steps are **ordered lists**. Named tables serve
instead for parameters, mappings and callable recipes; their order in the
file must not define the execution order. A comment can explain why a header
or a query parser is needed, next to the value. These are properties of the
[TOML](https://toml.io/en/v1.0.0) syntax, not a workflow engine already
included in the format.

### Comparison on project examples

Two comparisons that can be checked with synthetic data are available:

- [TOML resolver profile](../config/design-examples/resolver-v1.toml) and
  [JSON v1](../config/examples/resolver-demo.json): same data and same schema.
- [TOML flow](../config/design-examples/fixture-flow.toml) and
  [equivalent JSON](../config/design-examples/fixture-flow.json): catalogue,
  details, ordered steps and dispatch. This is an example of the proposed
  language, not accepted by the v1 schema nor executed by a runtime.
  Operations, references and templates remain illustrative.

The comparison supports TOML for these mostly linear recipes. Repeated lists
of steps and comments turn out easier to maintain; fewer lines or brackets
alone are not a measure of modularity. The format does not solve typing,
fallback or versioning: they remain responsibilities of the model and of the
engine.

### Branches and nested structures

TOML's practical limit appears with deep trees of branches and sub-flows:
section paths grow longer, and a sub-table of an array refers to the most
recent element. The [synthetic profile](../config/design-examples/resolver-v1.toml)
shows this binding for `resolution.rules.operation`.

For the cases studied, the [composable resolvers](#composable-resolvers)
approach applies. If a future feature really requires complex recursive
trees, their representation will be reassessed; this limit does not justify
making TOML secondary now.

### Parsing, validation and compatibility

The engine always receives the same internal types:

```text
TOML profile → TOML parser ┐
                         ├→ ProfileSpec → validation → ExecutionPlan
JSON profile → JSON parser┘
```

The schema can stay written in JSON Schema: its subject is the JSON-compatible
data model, which can also be obtained from TOML parsing. There is no need to
rewrite the schemas in TOML or to convert every profile on disk before use.
This distinction is explained by the
[official JSON Schema glossary](https://json-schema.org/learn/glossary#json).

The common subset admits maps with string keys, lists, strings, booleans and
finite numbers within the limits of the contract. IDs and timestamps stay
explicit strings. Optional fields are omitted: the absence of `null` in TOML
is not an obstacle for the profiles examined. Native TOML dates and
non-finite numeric values are rejected by the experimental policy, instead of
being converted implicitly. Duplicate keys must be rejected during parsing.

The examples use constructs available in TOML 1.0, checked against the
[specification](https://toml.io/en/v1.0.0). New features are not required
just because a later version exists. A possible YAML frontend will have to
produce the same values, with explicit rules for types and without custom
tags or aliases. The term “JSON schema” in the YAML specification concerns
the resolution of YAML types and is not the same as the JSON Schema standard.

### Repeatable check of the comparison

With Python 3.11+ and the single development dependency `jsonschema`:

```sh
uv run --no-project --with jsonschema python dev/checks/check-config-formats.py
```

The [check](../dev/checks/check-config-formats.py) verifies data equivalence,
order of steps and branches, scope of sub-tables, reuse of the v1 schema, and
rejection of invalid fields, duplicate keys, native dates and non-finite
numbers. It also verifies that the flow example is not mistaken for a v1
profile. It does not verify a Rust parser, an executable plan or live access.
Comments are kept in the source file; the model read by the parser does not
keep their text. A future editor that rewrites files will have to handle this
requirement separately.

Additional references: [JSON, RFC 8259](https://www.rfc-editor.org/rfc/rfc8259),
[YAML 1.2.2](https://yaml.org/spec/1.2.2/).

## Gap from the current draft

The [current schema](../schemas/source.schema.json) validates five examples
through [check-source-profiles.py](../dev/checks/check-source-profiles.py).
It is not yet an executable contract and does not contain all of this
proposal. On the TV only a provisional recipe runs, in the `.stream` files and
in the channel list, with four operations and textual parameters, without
types or references ([recipe.rs](../crates/core/src/live/recipe.rs),
[recipe steps](channel-list.md#recipe-steps)); the contract will
replace it. Before the runtime, the schema and semantics of the following
must be defined together:

- a `sources` flow distinct from the catalogue, and nested lists of
  candidates;
- typed steps, references and shared recipes;
- request bodies, adapter parameters and HTTP scopes per host;
- profile revision, required capabilities and compatibility;
- renewal and fallback that keep the stream's identity and the errors.

The existing draft stays recognisable and its examples disabled. The new
structure will have to be validated before schema, examples and loader are
updated together.

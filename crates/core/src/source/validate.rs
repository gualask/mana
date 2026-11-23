//! The checks a source passes before it is used, beyond reading: each
//! reference names something that comes before it, values fit where they
//! go when that is known before running, names of header sets, resolvers
//! and recipes exist, ids and routes are not repeated, recipes do not call
//! themselves, patterns and selectors compile, and each flow can end on
//! what it must give.

use std::collections::{BTreeMap, BTreeSet};

use super::{Catalogue, Error, Field, Flow, Headers, Kind, Mapping, Operation, Reference, Resolve, Route, Segment, Source, Template, Use};

/// What a flow must end on.
#[derive(Clone, Copy)]
enum Output {
    /// A list of entries with these fields.
    Entries(&'static [&'static str]),
    /// A stream or a next candidate.
    Resolution,
    Anything,
}

pub(super) fn check(source: &Source) -> Result<(), Error> {
    if let Some(Catalogue::Steps(flow)) = &source.catalogue {
        check_flow(source, "catalogue", flow, &[("device", Kind::Text)], Output::Entries(&["id", "title"]))?;
    }
    let item = [("item", Kind::Fields), ("device", Kind::Text)];
    if let Some(flow) = &source.sources {
        check_flow(source, "sources", flow, &item, Output::Entries(&["id"]))?;
    }
    let candidate = [("candidate", Kind::Fields), ("item", Kind::Fields), ("device", Kind::Text)];
    match &source.resolve {
        Resolve::Steps(flow) => check_flow(source, "resolve", flow, &candidate, Output::Resolution)?,
        Resolve::Routes(routes) => check_routes(source, routes)?,
    }
    for (name, flow) in &source.resolvers {
        check_flow(source, &format!("resolvers.{name}"), flow, &candidate, Output::Resolution)?;
    }
    for (name, recipe) in &source.recipes {
        let mut inputs: Vec<(&str, Kind)> = recipe.inputs.iter().map(|input| (input.as_str(), Kind::Any)).collect();
        inputs.push(("device", Kind::Text));
        check_flow(source, &format!("recipes.{name}"), &recipe.flow, &inputs, Output::Anything)?;
    }
    check_calls(source)
}

/// Each route of `resolve`: not the same `when` as an earlier one, to a
/// header set or a resolver that exists.
fn check_routes(source: &Source, routes: &[Route]) -> Result<(), Error> {
    for (n, route) in routes.iter().enumerate() {
        let at = format!("resolve.routes.{n}");
        if routes[..n].iter().any(|earlier| earlier.when == route.when) {
            return Err(Error::EqualRoutes { at });
        }
        match &route.to {
            Use::Stream(headers) => check_headers(source, &at, headers.as_ref())?,
            Use::Resolver(name) if !source.resolvers.contains_key(name) => return Err(Error::UnknownResolver { at, name: name.clone() }),
            Use::Resolver(_) => {}
        }
    }
    Ok(())
}

/// The names a flow can read: its inputs, then each step once it has run.
struct Scope<'a> {
    at: String,
    inputs: &'a [(&'a str, Kind)],
    steps: BTreeMap<&'a str, Kind>,
    /// The entry of `list.map`, while its fields are read.
    entry: bool,
}

fn check_flow(source: &Source, at: &str, flow: &Flow, inputs: &[(&str, Kind)], output: Output) -> Result<(), Error> {
    let mut scope = Scope { at: String::new(), inputs, steps: BTreeMap::new(), entry: false };
    for step in &flow.steps {
        scope.at = format!("{at}.{}", step.id);
        if scope.steps.contains_key(step.id.as_str()) {
            return Err(Error::DuplicateId { at: scope.at.clone(), id: step.id.clone() });
        }
        for (_, template, expected) in step.operation.parameters() {
            check_template(&scope, template, expected)?;
        }
        check_operation(source, &mut scope, &step.operation)?;
        scope.steps.insert(&step.id, step.operation.output());
    }
    check_output(source, at, &scope.at, flow, output)
}

/// The flow's last step, at `last_at`, can give what the flow must end on:
/// for entries, those of `list.map` with their required fields, also when
/// a recipe makes each one with `record`.
fn check_output(source: &Source, at: &str, last_at: &str, flow: &Flow, output: Output) -> Result<(), Error> {
    let missing = |required: &[&'static str], fields: &BTreeMap<String, Field>| required.iter().find(|field| !fields.contains_key(**field)).copied();
    let last = flow.steps.last().map(|step| &step.operation);
    let fits = match (output, last) {
        (Output::Entries(required), Some(Operation::ListMap { map: Mapping::Fields(item), .. })) => match missing(required, item) {
            Some(field) => return Err(Error::MissingField { at: last_at.into(), field: format!("item.{field}") }),
            None => true,
        },
        (Output::Entries(required), Some(Operation::ListMap { map: Mapping::Call { recipe, .. }, .. })) => {
            let called = source.recipes.get(recipe).and_then(|recipe| recipe.flow.steps.last());
            if let Some(step) = called
                && let Operation::Record { fields } = &step.operation
                && let Some(field) = missing(required, fields)
            {
                return Err(Error::MissingField { at: format!("recipes.{recipe}.{}", step.id), field: format!("fields.{field}") });
            }
            true
        }
        (Output::Entries(_), Some(operation)) => matches!(operation.output(), Kind::List | Kind::Json | Kind::Any),
        (Output::Resolution, Some(operation)) => matches!(operation.output(), Kind::Stream | Kind::Next | Kind::Any),
        (Output::Anything, _) => true,
        (_, None) => false,
    };
    if !fits {
        return Err(Error::WrongOutput { at: at.into(), resolution: matches!(output, Output::Resolution) });
    }
    Ok(())
}

/// The checks particular to an operation: names, patterns, selectors and
/// the fields of `list.map`.
fn check_operation(source: &Source, scope: &mut Scope, operation: &Operation) -> Result<(), Error> {
    match operation {
        Operation::Http { headers, .. } | Operation::Stream { headers, .. } => check_headers(source, &scope.at, headers.as_ref()),
        Operation::TextMatch { pattern, .. } => check_pattern(&scope.at, pattern),
        Operation::HtmlAttr { selector, .. } => match selector.parse::<lol_html::Selector>() {
            Ok(_) => Ok(()),
            Err(error) => Err(Error::BadSelector { at: scope.at.clone(), detail: error.to_string() }),
        },
        Operation::ListMap { map, filter, .. } => {
            scope.entry = true;
            let checked = check_entry(source, scope, map, filter);
            scope.entry = false;
            checked
        }
        Operation::Call { recipe, input } => check_call(source, &scope.at, recipe, input),
        Operation::Record { fields } => check_fields(scope, fields),
        _ => Ok(()),
    }
}

/// The templates of `list.map` that read its entry (`{item…}`).
fn check_entry(source: &Source, scope: &Scope, map: &Mapping, filter: &BTreeMap<String, Template>) -> Result<(), Error> {
    for template in filter.values() {
        check_template(scope, template, Kind::Text)?;
    }
    match map {
        Mapping::Fields(item) => check_fields(scope, item),
        Mapping::Call { recipe, input } => {
            check_call(source, &scope.at, recipe, input)?;
            input.values().try_for_each(|template| check_template(scope, template, Kind::Any))
        }
    }
}

/// The fields of a `list.map` item or of a `record`: text, from templates,
/// patterns and dates.
fn check_fields(scope: &Scope, fields: &BTreeMap<String, Field>) -> Result<(), Error> {
    fields.values().try_for_each(|field| match field {
        Field::Value(template) => check_template(scope, template, Kind::Text),
        Field::Match { from, pattern } => check_template(scope, from, Kind::Text).and_then(|()| check_pattern(&scope.at, pattern)),
        Field::Date { from, .. } => check_template(scope, from, Kind::Text),
    })
}

/// A recipe that exists, given exactly the inputs it names.
fn check_call(source: &Source, at: &str, recipe: &str, input: &BTreeMap<String, Template>) -> Result<(), Error> {
    let Some(called) = source.recipes.get(recipe) else {
        return Err(Error::UnknownRecipe { at: at.into(), name: recipe.into() });
    };
    let given: BTreeSet<&str> = input.keys().map(String::as_str).collect();
    let named: BTreeSet<&str> = called.inputs.iter().map(String::as_str).collect();
    if given != named {
        return Err(Error::CallInputs { at: at.into(), recipe: recipe.into() });
    }
    Ok(())
}

fn check_headers(source: &Source, at: &str, headers: Option<&Headers>) -> Result<(), Error> {
    match headers {
        Some(Headers::Set(name)) if !source.headers.contains_key(name) => Err(Error::UnknownHeaders { at: at.into(), name: name.clone() }),
        _ => Ok(()),
    }
}

/// A pattern must compile and have exactly one group, the value it gives.
fn check_pattern(at: &str, pattern: &str) -> Result<(), Error> {
    let regex = regex::Regex::new(pattern).map_err(|e| Error::BadPattern { at: at.into(), detail: e.to_string() })?;
    match regex.captures_len() - 1 {
        1 => Ok(()),
        groups => Err(Error::BadPattern { at: at.into(), detail: format!("{groups} groups, one is needed") }),
    }
}

/// A template alone keeps its reference's kind; a template with text or
/// several references is text, and each reference in it must be text.
fn check_template(scope: &Scope, template: &Template, expected: Kind) -> Result<(), Error> {
    if let Some(reference) = template.whole() {
        let found = kind_of(scope, reference)?;
        return fits(scope, reference, found, expected);
    }
    for reference in template.references() {
        fits(scope, reference, kind_of(scope, reference)?, Kind::Text)?;
    }
    if !Kind::Text.fits(expected) {
        let reference = template.references().next().map_or_else(|| "text".to_string(), Reference::to_string);
        return Err(Error::TypeMismatch { at: scope.at.clone(), reference, expected, found: Kind::Text });
    }
    Ok(())
}

fn fits(scope: &Scope, reference: &Reference, found: Kind, expected: Kind) -> Result<(), Error> {
    if found.fits(expected) {
        return Ok(());
    }
    Err(Error::TypeMismatch { at: scope.at.clone(), reference: reference.to_string(), expected, found })
}

/// What `reference` gives, if it names something the scope has.
fn kind_of(scope: &Scope, reference: &Reference) -> Result<Kind, Error> {
    let unknown = || Error::UnknownReference { at: scope.at.clone(), reference: reference.to_string() };
    let mut path = reference.path.iter();
    let Some(Segment::Name(first)) = path.next() else { return Err(unknown()) };
    let mut kind = match first.as_str() {
        "input" => {
            let Some(Segment::Name(name)) = path.next() else { return Err(unknown()) };
            scope.inputs.iter().find(|(input, _)| input == name).map(|(_, kind)| *kind).ok_or_else(unknown)?
        }
        "item" if scope.entry => Kind::Any,
        step => *scope.steps.get(step).ok_or_else(unknown)?,
    };
    for segment in path {
        let name = match segment {
            Segment::Name(name) => Some(name.as_str()),
            Segment::Reference(inner) => {
                fits(scope, inner, kind_of(scope, inner)?, Kind::Text)?;
                None
            }
        };
        kind = match (kind, name) {
            (Kind::Response, Some("body")) => Kind::Bytes,
            (Kind::Response, Some("text" | "url")) => Kind::Text,
            (Kind::Response, Some("json")) => Kind::Json,
            (Kind::Response, Some("header")) => Kind::Fields,
            (Kind::Response, _) => return Err(unknown()),
            (Kind::Fields, _) => Kind::Text,
            (Kind::Json, _) => Kind::Json,
            (Kind::Record | Kind::List | Kind::Any, _) => Kind::Any,
            (found, _) => {
                return Err(Error::TypeMismatch { at: scope.at.clone(), reference: reference.to_string(), expected: Kind::Record, found });
            }
        };
    }
    Ok(kind)
}

/// Recipes calling themselves, directly or through others, would never end.
fn check_calls(source: &Source) -> Result<(), Error> {
    let calls = |flow: &Flow| -> Vec<String> {
        let called = flow.steps.iter().filter_map(|step| match &step.operation {
            Operation::Call { recipe, .. } | Operation::ListMap { map: Mapping::Call { recipe, .. }, .. } => Some(recipe.clone()),
            _ => None,
        });
        called.collect()
    };
    let graph: BTreeMap<&str, Vec<String>> = source.recipes.iter().map(|(name, recipe)| (name.as_str(), calls(&recipe.flow))).collect();
    match graph.keys().find(|start| reaches_itself(&graph, start)) {
        Some(start) => Err(Error::CallCycle { recipe: start.to_string() }),
        None => Ok(()),
    }
}

/// Whether the recipe `start`, through the calls of `graph`, calls itself.
fn reaches_itself(graph: &BTreeMap<&str, Vec<String>>, start: &str) -> bool {
    let mut stack: Vec<&str> = graph[start].iter().map(String::as_str).collect();
    let mut seen = BTreeSet::new();
    while let Some(name) = stack.pop() {
        if name == start {
            return true;
        }
        if seen.insert(name) {
            stack.extend(graph.get(name).into_iter().flatten().map(String::as_str));
        }
    }
    false
}

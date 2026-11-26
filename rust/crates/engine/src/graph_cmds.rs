//! Graph editor commands: open, edit and save material graphs and visual scripts.
//!
//! One graph is open at a time. Edits are undo steps in the same history as scene edits.

use std::path::{Path, PathBuf};

use bayleaf_graph::{Catalog, Graph, Link, PinRef, Problem};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// Why graph edits are disabled with no graph open.
pub(crate) const NO_GRAPH: &str = "open or create a graph first";

/// The graph open in the graph editor.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphDoc {
    /// Which open graph it is, for as long as it is open.
    pub id: u64,
    pub graph: Graph,
    /// File the graph was opened from or last saved to.
    pub path: Option<PathBuf>,
    /// True when the graph has changes that are not saved.
    pub dirty: bool,
}

impl GraphDoc {
    pub fn catalog(&self) -> Option<&'static Catalog> {
        catalog_for_domain(&self.graph.domain)
    }
}

/// The catalogs of every graph domain.
const CATALOGS: [&Catalog; 3] = [&bayleaf_render::material::CATALOG, &bayleaf_script::visual::CATALOG, &bayleaf_animation::states::CATALOG];

/// The node kinds of `domain` (`material`, `script` or `animation`).
pub fn catalog_for_domain(domain: &str) -> Option<&'static Catalog> {
    CATALOGS.into_iter().find(|c| c.domain == domain)
}

/// The domain whose files have `path`'s extension.
pub fn catalog_for_path(path: &Path) -> Option<&'static Catalog> {
    let extension = path.extension()?.to_str()?;
    CATALOGS.into_iter().find(|c| c.extension == extension)
}

/// What stops `graph` from compiling, or nothing.
pub fn problems(graph: &Graph) -> Vec<Problem> {
    let result = match graph.domain.as_str() {
        "material" => bayleaf_render::material::compile(graph).map(|_| ()),
        "script" => bayleaf_script::visual::compile(graph).map(|_| ()),
        "animation" => bayleaf_animation::states::compile(graph).map(|_| ()),
        other => Err(vec![Problem::graph(format!("unknown graph domain {other}"))]),
    };
    result.err().unwrap_or_default()
}

fn has_graph(session: &Session) -> Result<(), String> {
    if session.graph.is_some() { Ok(()) } else { Err(NO_GRAPH.to_owned()) }
}

fn spec(id: &'static str, label: &'static str, params: &'static str, mutates: Mutates, run: crate::RunFn) -> CommandSpec {
    let enabled: crate::EnabledFn =
        if id.starts_with("graph.node.") || id.starts_with("graph.link.") || id.starts_with("graph.comment.") || matches!(id, "graph.save") {
            has_graph
        } else {
            always
        };
    CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates, enabled, run }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("graph.catalog", "Graph Node Kinds", "{domain: \"material\" | \"script\" | \"animation\"}", Mutates::No, graph_catalog),
        spec("graph.new", "New Graph", "{domain: \"material\" | \"script\" | \"animation\"}", Mutates::No, graph_new),
        spec("graph.open", "Open Graph", "{path} (.bmaterial or .bscript)", Mutates::No, graph_open),
        spec("graph.save", "Save Graph", "{path?} (needed for a graph that has no file yet)", Mutates::No, graph_save),
        spec(
            "graph.close",
            "Close Graph",
            "{id?}: closes the graph in use (or the open graph id) with its undo steps; the one opened before it is then in use",
            Mutates::No,
            graph_close,
        ),
        spec(
            "graph.get",
            "Show Graph",
            "none; returns the graph in use: {id, domain, path, dirty, graph, problems, outputTypes}; outputTypes says what each number-or-vector output gives now, by its inputs: [{node, pin, type}]",
            Mutates::No,
            graph_get,
        ),
        spec(
            "graph.list",
            "Open Graphs",
            "none; the open graphs, each in a tab of its own: {graphs: [{id, domain, path, dirty, inUse}]}",
            Mutates::No,
            graph_list,
        ),
        spec("graph.activate", "Use Graph", "{id} or {path}: makes that open graph the one the graph.* commands work on", Mutates::No, graph_activate),
        spec("graph.node.add", "Add Node", "{kind, position: [x, y]}", Mutates::Graph, node_add),
        spec("graph.node.remove", "Remove Node", "{node}", Mutates::Graph, node_remove),
        spec("graph.node.move", "Move Node", "{node, position: [x, y]}", Mutates::Graph, node_move),
        spec("graph.node.set", "Set Node Value", "{node, name, value} (null restores the default)", Mutates::Graph, node_set),
        spec("graph.node.copy", "Copy Nodes", "{nodes: [id]}: returns {text}, the nodes and the links between them as clipboard text", Mutates::No, node_copy),
        spec(
            "graph.node.paste",
            "Paste Nodes",
            "{text, offset?: [x, y]}: adds copies of copied nodes with new ids; returns {nodes, skipped}",
            Mutates::Graph,
            node_paste,
        ),
        spec(
            "graph.node.duplicate",
            "Duplicate Nodes",
            "{nodes: [id]}: adds copies beside them, linked as the originals are to each other; returns {nodes, skipped}",
            Mutates::Graph,
            node_duplicate,
        ),
        spec("graph.link.add", "Link Pins", "{from: {node, pin}, to: {node, pin}}", Mutates::Graph, link_add),
        spec("graph.link.remove", "Unlink Pin", "{node, pin}: removes every link at that pin", Mutates::Graph, link_remove),
        spec(
            "graph.comment.add",
            "Add Comment",
            "{position: [x, y], size?: [w, h], text?}: a box behind nodes, with a note; returns {comment}",
            Mutates::Graph,
            comment_add,
        ),
        spec("graph.comment.set", "Edit Comment", "{comment, text?, position?, size?}", Mutates::Graph, comment_set),
        spec("graph.comment.remove", "Remove Comment", "{comment}: the nodes in it stay", Mutates::Graph, comment_remove),
        spec(
            "graph.variable.add",
            "Add Variable",
            "{name, type: \"number\" | \"wholeNumber\" | \"vector\" | \"trueFalse\" | \"text\" | \"object\" | \"color\", shape?, key?, start?}: declares a variable of a visual script, starting at start (else zero); shape \"list\" or \"map\" makes it a list (items in order, by index from 0) or a map (values under keys of type key, else text) of that type, starting empty or with start ([...] or {...})",
            Mutates::Graph,
            variable_add,
        ),
        spec(
            "graph.variable.set",
            "Change Variable",
            "{name, rename?, type?, shape?, key?, start?}: a new start value, a new name that the variable's nodes follow, or a new type, shape (\"single\", \"list\", \"map\") or map key type (key); the start stays where it still fits, else it goes to zero, and its nodes follow the new type",
            Mutates::Graph,
            variable_set,
        ),
        spec("graph.variable.remove", "Remove Variable", "{name}: its nodes keep working, starting from zero", Mutates::Graph, variable_remove),
        spec(
            "graph.property.add",
            "Add Property",
            "{name, type, shape?, key?, default?}: declares a property of a visual script (type and shape as for variables), with a default (else zero); the Inspector sets it per object",
            Mutates::Graph,
            property_add,
        ),
        spec(
            "graph.property.set",
            "Change Property",
            "{name, rename?, type?, shape?, key?, default?, inspector?}: a new default, a new type or shape (as for variables; options that no longer fit go), a new name that its Property and Set Property nodes follow, or how the Inspector shows it ({label?, hint?, group?, range?, slider?, readOnly?, choices?, showIf?}; {} for a plain row)",
            Mutates::Graph,
            property_set,
        ),
        spec("graph.property.remove", "Remove Property", "{name}: its nodes stay, reading zero where no object sets it", Mutates::Graph, property_remove),
        spec(
            "graph.function.add",
            "Add Function",
            "{name}: declares a function of a visual script, with no inputs or outputs yet; its Function node begins its actions, Return gives back its outputs and Call Function runs it",
            Mutates::Graph,
            function_add,
        ),
        spec(
            "graph.function.set",
            "Change Function",
            "{name, rename?, inputs?, outputs?}: a new name, or new inputs or outputs ([{name, type}], type as for variables); the function's nodes follow, and links to pins that went away go",
            Mutates::Graph,
            function_set,
        ),
        spec("graph.function.remove", "Remove Function", "{name}: its nodes stay, and say they name no function", Mutates::Graph, function_remove),
        spec(
            "graph.event.add",
            "Add Event",
            "{name}: declares an event of a visual script, bringing no values yet; its On Event nodes run when a script or the interface triggers it on the object, and Trigger Event nodes trigger it",
            Mutates::Graph,
            event_add,
        ),
        spec(
            "graph.event.set",
            "Change Event",
            "{name, rename?, params?}: a new name, or new values ([{name, type}], type as for variables); the event's nodes follow, and links to pins that went away go",
            Mutates::Graph,
            event_set,
        ),
        spec("graph.event.remove", "Remove Event", "{name}: its nodes stay (a Trigger Event may name another script's event)", Mutates::Graph, event_remove),
        spec(
            "graph.menu.set",
            "Set Menu Path",
            "{path}: where Add Component lists the visual script (Light/Flicker); empty for under Scripts",
            Mutates::Graph,
            menu_set,
        ),
    ]
}

/// The open graph, when it is a visual script: only scripts have variables.
fn open_script(session: &mut Session) -> Result<&mut Graph, CommandError> {
    let (graph, catalog) = open(session)?;
    if catalog.domain != bayleaf_script::visual::CATALOG.domain {
        return Err(CommandError::Failed("only visual scripts have variables, functions and a menu path".to_owned()));
    }
    Ok(graph)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VariableAddParams {
    name: String,
    #[serde(rename = "type")]
    ty: bayleaf_graph::VariableType,
    #[serde(default)]
    shape: Option<bayleaf_graph::Shape>,
    /// A map's keys' type (text when left out).
    #[serde(default)]
    key: Option<bayleaf_graph::VariableType>,
    #[serde(default)]
    start: Option<Value>,
}

fn variable_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: VariableAddParams = parse(params)?;
    let graph = open_script(session)?;
    let mut trial = graph.clone();
    trial.add_variable(&p.name, p.ty, None).map_err(failed)?;
    trial.retype_variable(&p.name, p.ty, p.shape.unwrap_or_default(), p.key.unwrap_or(bayleaf_graph::VariableType::Text)).map_err(failed)?;
    if let Some(start) = p.start {
        trial.set_variable(&p.name, None, Some(start), &[]).map_err(failed)?;
    }
    *graph = trial;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VariableSetParams {
    name: String,
    #[serde(default)]
    rename: Option<String>,
    #[serde(default, rename = "type")]
    ty: Option<bayleaf_graph::VariableType>,
    #[serde(default)]
    shape: Option<bayleaf_graph::Shape>,
    /// A map's keys' type (text when left out).
    #[serde(default)]
    key: Option<bayleaf_graph::VariableType>,
    #[serde(default)]
    start: Option<Value>,
}

/// The node kinds that name a declaration of `ty` by their `name` setting: its Get and Set nodes
/// (`kinds`), and the list and map nodes.
fn naming_kinds(kinds: [&'static str; 2]) -> Vec<&'static str> {
    kinds.into_iter().chain(bayleaf_script::visual::COLLECTION_KINDS).collect()
}

/// After a declaration `name` went from `old` to `new` type and shape: its Get and Set nodes become
/// the new type's (`kinds` gives a type's two), keeping their places and the links that still fit,
/// and its list and map nodes take what it now holds and the type of a map's keys.
fn follow_retype(
    graph: &mut Graph,
    name: &str,
    old: bayleaf_graph::VariableType,
    new: (bayleaf_graph::VariableType, bayleaf_graph::Shape, bayleaf_graph::VariableType),
    kinds: fn(bayleaf_graph::VariableType) -> [&'static str; 2],
) -> Result<(), CommandError> {
    let catalog = &bayleaf_script::visual::CATALOG;
    if old != new.0 {
        let (before, after) = (kinds(old), kinds(new.0));
        let named: Vec<(u32, usize)> = graph
            .nodes
            .iter()
            .filter(|n| n.values.get("name").and_then(Value::as_str) == Some(name))
            .filter_map(|n| before.iter().position(|k| *k == n.kind).map(|i| (n.id, i)))
            .collect();
        for (id, i) in named {
            graph.change_kind(catalog, id, after[i]).map_err(failed)?;
        }
    }
    if !new.1.is_single() {
        graph.follow_item(catalog, name, new.0, new.2, &bayleaf_script::visual::COLLECTION_KINDS).map_err(failed)?;
    }
    Ok(())
}

fn variable_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: VariableSetParams = parse(params)?;
    let graph = open_script(session)?;
    let (ty, shape, key) =
        graph.variable(&p.name).map(|v| (v.ty, v.shape, v.key)).ok_or_else(|| CommandError::BadParams(format!("there is no variable {}", p.name)))?;
    let new = (p.ty.unwrap_or(ty), p.shape.unwrap_or(shape), p.key.unwrap_or(key));
    let mut trial = graph.clone();
    if new != (ty, shape, key) {
        trial.retype_variable(&p.name, new.0, new.1, new.2).map_err(failed)?;
        follow_retype(&mut trial, &p.name, ty, new, bayleaf_script::visual::variable_kinds)?;
    }
    trial.set_variable(&p.name, p.rename.as_deref(), p.start, &naming_kinds(bayleaf_script::visual::variable_kinds(new.0))).map_err(failed)?;
    *graph = trial;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VariableNameParams {
    name: String,
}

fn variable_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: VariableNameParams = parse(params)?;
    open_script(session)?.remove_variable(&p.name).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PropertyAddParams {
    name: String,
    #[serde(rename = "type")]
    ty: bayleaf_graph::VariableType,
    #[serde(default)]
    shape: Option<bayleaf_graph::Shape>,
    /// A map's keys' type (text when left out).
    #[serde(default)]
    key: Option<bayleaf_graph::VariableType>,
    #[serde(default)]
    default: Option<Value>,
}

fn property_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PropertyAddParams = parse(params)?;
    let graph = open_script(session)?;
    let mut trial = graph.clone();
    trial.add_property(&p.name, p.ty, None).map_err(failed)?;
    trial.retype_property(&p.name, p.ty, p.shape.unwrap_or_default(), p.key.unwrap_or(bayleaf_graph::VariableType::Text)).map_err(failed)?;
    if let Some(default) = p.default {
        trial.set_property(&p.name, None, Some(default), &[]).map_err(failed)?;
    }
    *graph = trial;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PropertySetParams {
    name: String,
    #[serde(default)]
    rename: Option<String>,
    #[serde(default, rename = "type")]
    ty: Option<bayleaf_graph::VariableType>,
    #[serde(default)]
    shape: Option<bayleaf_graph::Shape>,
    /// A map's keys' type (text when left out).
    #[serde(default)]
    key: Option<bayleaf_graph::VariableType>,
    #[serde(default)]
    default: Option<Value>,
    #[serde(default)]
    inspector: Option<bayleaf_graph::PropertyOptions>,
}

fn property_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PropertySetParams = parse(params)?;
    let graph = open_script(session)?;
    let property = graph.property(&p.name).ok_or_else(|| CommandError::BadParams(format!("there is no property {}", p.name)))?;
    let (ty, shape, key) = (property.ty, property.shape, property.key);
    let new = (p.ty.unwrap_or(ty), p.shape.unwrap_or(shape), p.key.unwrap_or(key));
    let mut trial = graph.clone();
    if new != (ty, shape, key) {
        trial.retype_property(&p.name, new.0, new.1, new.2).map_err(failed)?;
        follow_retype(&mut trial, &p.name, ty, new, bayleaf_script::visual::property_kinds)?;
        // Options of the old type that no longer fit (a range on text) go; its words stay.
        if bayleaf_script::visual::check_inspector(&trial).is_err()
            && let Some(kept) = trial.property(&p.name).map(|p| bayleaf_graph::PropertyOptions {
                label: p.inspector.label.clone(),
                hint: p.inspector.hint.clone(),
                group: p.inspector.group.clone(),
                read_only: p.inspector.read_only,
                ..Default::default()
            })
        {
            trial.set_property_options(&p.name, kept).map_err(failed)?;
        }
    }
    if let Some(options) = &p.inspector {
        trial.set_property_options(&p.name, options.clone()).map_err(failed)?;
    }
    // Options are checked as the script host checks them, before anything changes.
    bayleaf_script::visual::check_inspector(&trial).map_err(CommandError::BadParams)?;
    trial.set_property(&p.name, p.rename.as_deref(), p.default, &naming_kinds(bayleaf_script::visual::property_kinds(new.0))).map_err(failed)?;
    *graph = trial;
    Ok(json!({}))
}

fn property_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: VariableNameParams = parse(params)?;
    open_script(session)?.remove_property(&p.name).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FunctionAddParams {
    name: String,
}

fn function_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: FunctionAddParams = parse(params)?;
    open_script(session)?.add_function(&p.name).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FunctionSetParams {
    name: String,
    #[serde(default)]
    rename: Option<String>,
    #[serde(default)]
    inputs: Option<Vec<bayleaf_graph::Param>>,
    #[serde(default)]
    outputs: Option<Vec<bayleaf_graph::Param>>,
}

fn function_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: FunctionSetParams = parse(params)?;
    let catalog = &bayleaf_script::visual::CATALOG;
    open_script(session)?.set_function(catalog, &p.name, p.rename.as_deref(), p.inputs, p.outputs, &bayleaf_script::visual::FUNCTION_KINDS).map_err(failed)?;
    Ok(json!({}))
}

fn function_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: FunctionAddParams = parse(params)?;
    open_script(session)?.remove_function(&p.name).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventSetParams {
    name: String,
    #[serde(default)]
    rename: Option<String>,
    #[serde(default)]
    params: Option<Vec<bayleaf_graph::Param>>,
}

fn event_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: FunctionAddParams = parse(params)?;
    open_script(session)?.add_event(&p.name).map_err(failed)?;
    Ok(json!({}))
}

fn event_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: EventSetParams = parse(params)?;
    let catalog = &bayleaf_script::visual::CATALOG;
    open_script(session)?.set_event(catalog, &p.name, p.rename.as_deref(), p.params, &bayleaf_script::visual::EVENT_KINDS).map_err(failed)?;
    Ok(json!({}))
}

fn event_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: FunctionAddParams = parse(params)?;
    open_script(session)?.remove_event(&p.name).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MenuParams {
    path: String,
}

fn menu_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: MenuParams = parse(params)?;
    let path = if p.path.trim().is_empty() { String::new() } else { bayleaf_script::menu_path(&p.path).map_err(CommandError::BadParams)? };
    open_script(session)?.menu = path.clone();
    Ok(json!({ "path": path }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentAddParams {
    position: [f32; 2],
    #[serde(default)]
    size: Option<[f32; 2]>,
    #[serde(default)]
    text: Option<String>,
}

fn comment_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CommentAddParams = parse(params)?;
    let (graph, _) = open(session)?;
    let id = graph.add_comment(p.text.as_deref().unwrap_or("Comment"), p.position, p.size.unwrap_or(bayleaf_graph::COMMENT_SIZE)).map_err(failed)?;
    Ok(json!({ "comment": id }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentSetParams {
    comment: u32,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    position: Option<[f32; 2]>,
    #[serde(default)]
    size: Option<[f32; 2]>,
}

fn comment_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CommentSetParams = parse(params)?;
    if p.text.is_none() && p.position.is_none() && p.size.is_none() {
        return Err(CommandError::BadParams("give text, position or size".to_owned()));
    }
    let (graph, _) = open(session)?;
    graph.edit_comment(p.comment, p.text.as_deref(), p.position, p.size).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentParams {
    comment: u32,
}

fn comment_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CommentParams = parse(params)?;
    let (graph, _) = open(session)?;
    graph.remove_comment(p.comment).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DomainParams {
    domain: String,
}

fn domain(params: &Value) -> Result<&'static Catalog, CommandError> {
    let p: DomainParams = parse(params)?;
    catalog_for_domain(&p.domain)
        .ok_or_else(|| CommandError::BadParams(format!("unknown domain \"{}\"; use \"material\", \"script\" or \"animation\"", p.domain)))
}

fn graph_catalog(_: &mut Session, params: &Value) -> Result<Value, CommandError> {
    Ok(domain(params)?.to_json())
}

fn graph_new(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let catalog = domain(params)?;
    session.add_graph(GraphDoc { id: 0, graph: Graph::new(catalog), path: None, dirty: false });
    graph_get(session, &Value::Null)
}

/// One spelling per file, to tell whether a graph is open already.
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    let canonical = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canonical(a) == canonical(b)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: PathBuf,
}

fn graph_open(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let catalog = catalog_for_path(&p.path).ok_or_else(|| CommandError::BadParams("graph files end in .bmaterial or .bscript".to_owned()))?;
    let path = session.allowed(&p.path, Access::Read)?;
    // A graph open already is brought forward, with its changes.
    if let Some(id) = session.graphs().iter().find(|g| g.path.as_deref().is_some_and(|open| same_file(open, &path))).map(|g| g.id) {
        session.activate_graph(id)?;
        return graph_get(session, &Value::Null);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| CommandError::Failed(format!("cannot read {}: {e}", path.display())))?;
    let graph = Graph::from_json(&text, catalog).map_err(|e| CommandError::Failed(format!("{}: {e}", path.display())))?;
    session.add_graph(GraphDoc { id: 0, graph, path: Some(path), dirty: false });
    graph_get(session, &Value::Null)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveParams {
    #[serde(default)]
    path: Option<PathBuf>,
}

fn graph_save(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SaveParams = parse(params)?;
    let doc = session.graph.as_ref().ok_or_else(|| CommandError::Failed(NO_GRAPH.to_owned()))?;
    let catalog = doc.catalog().ok_or_else(|| CommandError::Failed("unknown graph domain".to_owned()))?;
    let path = p.path.or_else(|| doc.path.clone()).ok_or_else(|| CommandError::BadParams("this graph has no file yet; give a path".to_owned()))?;
    if path.extension().is_none_or(|e| e != catalog.extension) {
        return Err(CommandError::BadParams(format!("{} files end in .{}", catalog.label, catalog.extension)));
    }
    let path = session.allowed(&path, Access::Write)?;
    std::fs::write(&path, doc.graph.to_json()).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))?;
    if let Some(doc) = session.graph.as_mut() {
        doc.path = Some(path.clone());
        doc.dirty = false;
    }
    Ok(json!({ "path": path }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseParams {
    #[serde(default)]
    id: Option<u64>,
}

fn graph_close(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CloseParams = parse(params)?;
    if let Some(id) = p.id {
        session.activate_graph(id)?;
    }
    if session.graph.is_none() {
        return Err(CommandError::Failed(NO_GRAPH.to_owned()));
    }
    session.close_graph();
    Ok(json!({ "inUse": session.graph.as_ref().map(|g| g.id) }))
}

fn graph_list(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let _: Empty = parse(params)?;
    let in_use = session.graph.as_ref().map(|g| g.id);
    let graphs: Vec<Value> = session
        .graphs()
        .iter()
        .map(|g| json!({ "id": g.id, "domain": g.graph.domain, "path": g.path, "dirty": g.dirty, "inUse": Some(g.id) == in_use }))
        .collect();
    Ok(json!({ "graphs": graphs }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivateParams {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    path: Option<PathBuf>,
}

fn graph_activate(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ActivateParams = parse(params)?;
    let id = match (p.id, p.path) {
        (Some(id), None) => id,
        (None, Some(path)) => session
            .graphs()
            .iter()
            .find(|g| g.path.as_deref().is_some_and(|open| same_file(open, &path)))
            .map(|g| g.id)
            .ok_or_else(|| CommandError::BadParams(format!("{} is not open", path.display())))?,
        _ => return Err(CommandError::BadParams("give the graph's id or its path".to_owned())),
    };
    session.activate_graph(id)?;
    Ok(json!({ "id": id }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn graph_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let _: Empty = parse(params)?;
    let doc = session.graph.as_ref().ok_or_else(|| CommandError::Failed(NO_GRAPH.to_owned()))?;
    Ok(json!({
        "id": doc.id,
        "domain": doc.graph.domain,
        "path": doc.path,
        "dirty": doc.dirty,
        "graph": doc.graph,
        "problems": problems(&doc.graph),
        "outputTypes": output_types(&doc.graph),
    }))
}

/// What each number-or-vector output gives now (a vector, a number or a whole number), as the
/// graph editor shows it.
fn output_types(graph: &Graph) -> Vec<Value> {
    let Some(catalog) = catalog_for_domain(&graph.domain) else { return Vec::new() };
    let mut out = Vec::new();
    for node in &graph.nodes {
        let Some(def) = catalog.def_of(node) else { continue };
        for pin in def.outputs.iter().filter(|p| p.ty == bayleaf_graph::PinType::Any) {
            let ty = graph.output_type(catalog, &bayleaf_graph::PinRef::new(node.id, pin.name));
            out.push(json!({ "node": node.id, "pin": pin.name, "type": ty }));
        }
    }
    out
}

/// The open graph and its catalog, for an edit.
fn open(session: &mut Session) -> Result<(&mut Graph, &'static Catalog), CommandError> {
    let doc = session.graph.as_mut().ok_or_else(|| CommandError::Failed(NO_GRAPH.to_owned()))?;
    let catalog = catalog_for_domain(&doc.graph.domain).ok_or_else(|| CommandError::Failed("unknown graph domain".to_owned()))?;
    Ok((&mut doc.graph, catalog))
}

fn failed(error: bayleaf_graph::GraphError) -> CommandError {
    CommandError::Failed(error.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddParams {
    kind: String,
    #[serde(default)]
    position: [f32; 2],
}

fn node_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: AddParams = parse(params)?;
    let (graph, catalog) = open(session)?;
    let id = graph.add_node(catalog, &p.kind, p.position).map_err(failed)?;
    Ok(json!({ "node": id }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeParams {
    node: u32,
}

fn node_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: NodeParams = parse(params)?;
    let (graph, _) = open(session)?;
    graph.remove_node(p.node).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveParams {
    node: u32,
    position: [f32; 2],
}

fn node_move(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: MoveParams = parse(params)?;
    let (graph, _) = open(session)?;
    graph.move_node(p.node, p.position).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    node: u32,
    name: String,
    value: Value,
}

fn node_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SetParams = parse(params)?;
    let (graph, catalog) = open(session)?;
    graph.set_value(catalog, p.node, &p.name, p.value).map_err(failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkParams {
    from: PinRef,
    to: PinRef,
}

fn link_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: LinkParams = parse(params)?;
    let (graph, catalog) = open(session)?;
    let link = Link { from: p.from, to: p.to };
    // A list or map node takes the type of the list linked into it (one undo step with the link).
    let mut trial = graph.clone();
    if catalog.domain == bayleaf_script::visual::CATALOG.domain
        && let Some((id, item, key)) = bayleaf_script::visual::adopted_item(&trial, &link)
    {
        trial.set_value(catalog, id, "item", json!([item.code(), key.code()])).map_err(failed)?;
        trial.drop_unfit_links(catalog, id);
    }
    trial.connect(catalog, link).map_err(failed)?;
    *graph = trial;
    Ok(json!({}))
}

fn link_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PinRef = parse(params)?;
    let (graph, _) = open(session)?;
    match graph.disconnect(&p) {
        0 => Err(CommandError::Failed(format!("node {} pin \"{}\" has no links", p.node, p.pin))),
        removed => Ok(json!({ "removed": removed })),
    }
}

/// Where pasted and duplicated nodes go, from the originals: down and to the right.
const DUPLICATE_OFFSET: [f32; 2] = [40.0, 40.0];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodesParams {
    nodes: Vec<u32>,
}

/// The ids in `ids` that are not nodes of `graph`, refused so a typo does not copy nothing quietly.
fn known(graph: &Graph, ids: &[u32]) -> Result<(), CommandError> {
    match ids.iter().find(|id| graph.node(**id).is_none()) {
        Some(id) => Err(CommandError::Failed(format!("there is no node {id}"))),
        None if ids.is_empty() => Err(CommandError::BadParams("name at least one node".to_owned())),
        None => Ok(()),
    }
}

fn node_copy(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: NodesParams = parse(params)?;
    let (graph, _) = open(session)?;
    known(graph, &p.nodes)?;
    let text = serde_json::to_string(&graph.copy(&p.nodes)).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok(json!({ "text": text }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasteParams {
    text: String,
    #[serde(default)]
    offset: Option<[f32; 2]>,
}

fn node_paste(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PasteParams = parse(params)?;
    let fragment: bayleaf_graph::Fragment = serde_json::from_str(&p.text).map_err(|e| CommandError::BadParams(format!("the text is not copied nodes: {e}")))?;
    let (graph, catalog) = open(session)?;
    let pasted = graph.paste(catalog, &fragment, p.offset.unwrap_or(DUPLICATE_OFFSET)).map_err(failed)?;
    if pasted.ids.is_empty() {
        return Err(CommandError::Failed(format!("nothing to paste: a graph has only one {}", pasted.skipped.join(", "))));
    }
    Ok(json!({ "nodes": pasted.ids, "skipped": pasted.skipped }))
}

fn node_duplicate(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: NodesParams = parse(params)?;
    let (graph, catalog) = open(session)?;
    known(graph, &p.nodes)?;
    let fragment = graph.copy(&p.nodes);
    let pasted = graph.paste(catalog, &fragment, DUPLICATE_OFFSET).map_err(failed)?;
    if pasted.ids.is_empty() {
        return Err(CommandError::Failed(format!("nothing to duplicate: a graph has only one {}", pasted.skipped.join(", "))));
    }
    Ok(json!({ "nodes": pasted.ids, "skipped": pasted.skipped }))
}

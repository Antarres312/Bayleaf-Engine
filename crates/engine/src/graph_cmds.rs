//! Graph editor commands: open, edit and save material graphs and visual scripts.
//!
//! One graph is open at a time. Edits are undo steps in the same history as scene edits.

use std::path::{Path, PathBuf};

use bayleaf_graph::{Catalog, Graph, Link, PinRef, Problem};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, Snapshot, always, parse};

/// Why graph edits are disabled with no graph open.
pub(crate) const NO_GRAPH: &str = "open or create a graph first";

/// The graph open in the graph editor.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphDoc {
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
const CATALOGS: [&Catalog; 2] = [&bayleaf_render::material::CATALOG, &bayleaf_script::visual::CATALOG];

/// The node kinds of `domain` (`material` or `script`).
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
        other => Err(vec![Problem::graph(format!("unknown graph domain {other}"))]),
    };
    result.err().unwrap_or_default()
}

fn has_graph(session: &Session) -> Result<(), String> {
    if session.graph.is_some() { Ok(()) } else { Err(NO_GRAPH.to_owned()) }
}

fn spec(id: &'static str, label: &'static str, params: &'static str, mutates: Mutates, run: crate::RunFn) -> CommandSpec {
    let enabled: crate::EnabledFn =
        if id.starts_with("graph.node.") || id.starts_with("graph.link.") || matches!(id, "graph.save" | "graph.close") { has_graph } else { always };
    CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates, enabled, run }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("graph.catalog", "Graph Node Kinds", "{domain: \"material\" | \"script\"}", Mutates::No, graph_catalog),
        spec("graph.new", "New Graph", "{domain: \"material\" | \"script\"}", Mutates::No, graph_new),
        spec("graph.open", "Open Graph", "{path} (.bmaterial or .bscript)", Mutates::No, graph_open),
        spec("graph.save", "Save Graph", "{path?} (needed for a graph that has no file yet)", Mutates::No, graph_save),
        spec("graph.close", "Close Graph", "none", Mutates::No, graph_close),
        spec("graph.get", "Show Graph", "none; returns the open graph, its file and what stops it from compiling", Mutates::No, graph_get),
        spec("graph.node.add", "Add Node", "{kind, position: [x, y]}", Mutates::Graph, node_add),
        spec("graph.node.remove", "Remove Node", "{node}", Mutates::Graph, node_remove),
        spec("graph.node.move", "Move Node", "{node, position: [x, y]}", Mutates::Graph, node_move),
        spec("graph.node.set", "Set Node Value", "{node, name, value} (null restores the default)", Mutates::Graph, node_set),
        spec("graph.link.add", "Link Pins", "{from: {node, pin}, to: {node, pin}}", Mutates::Graph, link_add),
        spec("graph.link.remove", "Unlink Pin", "{node, pin}: removes every link at that pin", Mutates::Graph, link_remove),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DomainParams {
    domain: String,
}

fn domain(params: &Value) -> Result<&'static Catalog, CommandError> {
    let p: DomainParams = parse(params)?;
    catalog_for_domain(&p.domain).ok_or_else(|| CommandError::BadParams(format!("unknown domain \"{}\"; use \"material\" or \"script\"", p.domain)))
}

fn graph_catalog(_: &mut Session, params: &Value) -> Result<Value, CommandError> {
    Ok(domain(params)?.to_json())
}

/// Replaces the open graph; its undo steps go with it.
fn replace(session: &mut Session, doc: Option<GraphDoc>) {
    session.graph = doc;
    session.forget(|s| matches!(s, Snapshot::Graph(_)));
}

fn graph_new(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let catalog = domain(params)?;
    replace(session, Some(GraphDoc { graph: Graph::new(catalog), path: None, dirty: false }));
    graph_get(session, &Value::Null)
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
    let text = std::fs::read_to_string(&path).map_err(|e| CommandError::Failed(format!("cannot read {}: {e}", path.display())))?;
    let graph = Graph::from_json(&text, catalog).map_err(|e| CommandError::Failed(format!("{}: {e}", path.display())))?;
    replace(session, Some(GraphDoc { graph, path: Some(path), dirty: false }));
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

fn graph_close(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let _: Empty = parse(params)?;
    replace(session, None);
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn graph_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let _: Empty = parse(params)?;
    let doc = session.graph.as_ref().ok_or_else(|| CommandError::Failed(NO_GRAPH.to_owned()))?;
    Ok(json!({
        "domain": doc.graph.domain,
        "path": doc.path,
        "dirty": doc.dirty,
        "graph": doc.graph,
        "problems": problems(&doc.graph),
    }))
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
    graph.connect(catalog, Link { from: p.from, to: p.to }).map_err(failed)?;
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

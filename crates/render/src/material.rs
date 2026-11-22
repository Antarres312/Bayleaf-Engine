//! Material graphs: their node kinds and the compiler that turns a graph into a WGSL surface function.
//!
//! The mesh shader asks `surface(s)` for the base colour, metallic, roughness, emission and opacity
//! of each pixel and then applies the lighting. Meshes without a material use a built-in `surface`
//! that reads the mesh component's colour and sliders; a material graph replaces it.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use bayleaf_graph::{Catalog, Graph, NodeDef, PinDef, PinDefault, PinType, Problem, SettingDef, SettingKind, summary};

use PinType::{Any, Color, Float, Vec3};

const fn pin(name: &'static str, label: &'static str, ty: PinType) -> PinDef {
    PinDef::new(name, label, ty)
}

const fn number(name: &'static str, label: &'static str, ty: PinType, value: f32) -> PinDef {
    PinDef::with(name, label, ty, PinDefault::Float(value))
}

const fn node(
    kind: &'static str,
    label: &'static str,
    category: &'static str,
    description: &'static str,
    inputs: &'static [PinDef],
    outputs: &'static [PinDef],
) -> NodeDef {
    NodeDef { kind, label, category, description, inputs, outputs, settings: &[], unique: false }
}

const MATH_OUT: &[PinDef] = &[pin("result", "Result", Any)];

/// The outline option of a material's output: back faces grown by the width, in the colour.
const OUTLINE_SETTINGS: &[SettingDef] = &[
    SettingDef { name: "outlineWidth", label: "Outline Width", kind: SettingKind::Float(0.0) },
    SettingDef { name: "outlineColor", label: "Outline Color", kind: SettingKind::Color([0.0, 0.0, 0.0]) },
];

/// The node kinds of material graphs.
pub const CATALOG: Catalog = Catalog {
    domain: "material",
    label: "Material",
    extension: "bmaterial",
    starter: &[("output", [360.0, 40.0])],
    variant: bayleaf_graph::no_variant,
    upgrade: bayleaf_graph::no_upgrade,
    nodes: &[
        NodeDef {
            kind: "output",
            label: "Material Output",
            category: "Output",
            description: "What the surface looks like, lit by the engine. A material has one output: this or Custom Lighting Output.",
            inputs: &[
                PinDef::with("baseColor", "Base Color", Color, PinDefault::Color([0.8, 0.8, 0.8])),
                number("metallic", "Metallic", Float, 0.0),
                number("roughness", "Roughness", Float, 0.5),
                PinDef::with("emissive", "Emission", Color, PinDefault::Color([0.0, 0.0, 0.0])),
                number("opacity", "Opacity", Float, 1.0),
            ],
            outputs: &[],
            settings: OUTLINE_SETTINGS,
            unique: true,
        },
        NodeDef {
            kind: "customLighting",
            label: "Custom Lighting Output",
            category: "Output",
            description: "The colour the surface shows, worked out by the graph itself (with Main Light and Other Lights) instead of the engine's lighting.",
            inputs: &[PinDef::with("color", "Color", Color, PinDefault::Color([0.8, 0.8, 0.8])), number("alpha", "Opacity", Float, 1.0)],
            outputs: &[],
            settings: OUTLINE_SETTINGS,
            unique: true,
        },
        node(
            "mainLight",
            "Main Light",
            "Lights",
            "The main light at this point (the sun when there is one): the direction towards it, its colour and strength, and how much of it the shadows let through (1 lit).",
            &[],
            &[pin("direction", "Direction", Vec3), pin("color", "Color", Color), pin("shadow", "Shadow", Float)],
        ),
        node(
            "otherLights",
            "Other Lights",
            "Lights",
            "The light every other light gives this point, added up (facing them, past shadows).",
            &[],
            &[pin("color", "Color", Color)],
        ),
        node("viewDirection", "View Direction", "Inputs", "From the point towards the camera.", &[], &[pin("direction", "Direction", Vec3)]),
        node(
            "screenPosition",
            "Screen Position",
            "Inputs",
            "Where the pixel is on the screen, 0 to 1 from the top left, as a vector (x, y, 0).",
            &[],
            &[pin("position", "Position", Vec3)],
        ),
        node(
            "dot",
            "Dot Product",
            "Math",
            "How much two directions agree: 1 the same, 0 at right angles, −1 opposite (for unit vectors).",
            &[pin("a", "A", Vec3), pin("b", "B", Vec3)],
            &[pin("result", "Result", Float)],
        ),
        node("normalize", "Normalize", "Math", "The same direction with length 1.", &[pin("vector", "Vector", Vec3)], &[pin("vector", "Vector", Vec3)]),
        node(
            "step",
            "Step",
            "Math",
            "0 below the edge, 1 from the edge up: a hard band.",
            &[number("edge", "Edge", Float, 0.5), number("value", "Value", Any, 0.0)],
            MATH_OUT,
        ),
        node(
            "smoothstep",
            "Smooth Step",
            "Math",
            "0 below Low, 1 above High, a smooth curve between: a soft band.",
            &[number("low", "Low", Float, 0.0), number("high", "High", Float, 1.0), number("value", "Value", Any, 0.5)],
            MATH_OUT,
        ),
        node(
            "posterize",
            "Posterize",
            "Math",
            "Cuts values from 0 to 1 into a number of flat bands (2 or more).",
            &[number("value", "Value", Any, 0.0), number("steps", "Bands", Float, 3.0)],
            MATH_OUT,
        ),
        node(
            "remap",
            "Remap",
            "Math",
            "Moves a value from one range to another: From Low becomes To Low and From High To High.",
            &[
                number("value", "Value", Any, 0.5),
                number("fromLow", "From Low", Float, 0.0),
                number("fromHigh", "From High", Float, 1.0),
                number("toLow", "To Low", Float, 0.0),
                number("toHigh", "To High", Float, 1.0),
            ],
            MATH_OUT,
        ),
        NodeDef {
            kind: "number",
            label: "Number",
            category: "Values",
            description: "A fixed number.",
            inputs: &[],
            outputs: &[pin("value", "Value", Float)],
            settings: &[SettingDef { name: "value", label: "Value", kind: SettingKind::Float(1.0) }],
            unique: false,
        },
        NodeDef {
            kind: "color",
            label: "Color",
            category: "Values",
            description: "A fixed colour.",
            inputs: &[],
            outputs: &[pin("color", "Color", Color)],
            settings: &[SettingDef { name: "color", label: "Color", kind: SettingKind::Color([1.0, 1.0, 1.0]) }],
            unique: false,
        },
        node(
            "vector",
            "Make Vector",
            "Values",
            "A vector from three numbers.",
            &[number("x", "X", Float, 0.0), number("y", "Y", Float, 0.0), number("z", "Z", Float, 0.0)],
            &[pin("vector", "Vector", Vec3)],
        ),
        node(
            "split",
            "Split Vector",
            "Values",
            "The three numbers of a vector or colour.",
            &[pin("vector", "Vector", Vec3)],
            &[pin("x", "X", Float), pin("y", "Y", Float), pin("z", "Z", Float)],
        ),
        node(
            "objectColor",
            "Object Color",
            "Inputs",
            "The colour set on the object's Mesh component.",
            &[],
            &[pin("color", "Color", Color), pin("alpha", "Alpha", Float)],
        ),
        node("time", "Time", "Inputs", "Seconds since the view started; animates the material.", &[], &[pin("seconds", "Seconds", Float)]),
        node("position", "World Position", "Inputs", "Where the pixel is in the world.", &[], &[pin("position", "Position", Vec3)]),
        node(
            "uv",
            "Texture Coordinates",
            "Inputs",
            "Where on the shape's surface the pixel is: 0 to 1 across each face, as a vector (u, v, 0).",
            &[],
            &[pin("uv", "Coordinates", Vec3)],
        ),
        NodeDef {
            kind: "texture",
            label: "Image",
            category: "Inputs",
            description: "Colours from a PNG image, laid over the surface by its texture coordinates.",
            inputs: &[PinDef::with("uv", "Coordinates", Vec3, PinDefault::Implicit("uv"))],
            outputs: &[pin("color", "Color", Color), pin("alpha", "Alpha", Float)],
            settings: &[SettingDef { name: "image", label: "Image File", kind: SettingKind::Text("") }],
            unique: false,
        },
        node("normal", "Normal", "Inputs", "Which way the surface faces, in world space.", &[], &[pin("normal", "Normal", Vec3)]),
        node(
            "fresnel",
            "Edge Glow",
            "Inputs",
            "1 where the surface turns away from the viewer, 0 where it faces them.",
            &[number("power", "Sharpness", Float, 5.0)],
            &[pin("factor", "Factor", Float)],
        ),
        node("add", "Add", "Math", "A + B.", &[number("a", "A", Any, 0.0), number("b", "B", Any, 0.0)], MATH_OUT),
        node("subtract", "Subtract", "Math", "A − B.", &[number("a", "A", Any, 0.0), number("b", "B", Any, 0.0)], MATH_OUT),
        node("multiply", "Multiply", "Math", "A × B.", &[number("a", "A", Any, 1.0), number("b", "B", Any, 1.0)], MATH_OUT),
        node("divide", "Divide", "Math", "A ÷ B.", &[number("a", "A", Any, 1.0), number("b", "B", Any, 1.0)], MATH_OUT),
        node(
            "blend",
            "Blend",
            "Math",
            "From A (factor 0) to B (factor 1).",
            &[number("a", "A", Any, 0.0), number("b", "B", Any, 1.0), number("factor", "Factor", Float, 0.5)],
            MATH_OUT,
        ),
        node(
            "power",
            "Power",
            "Math",
            "Base to the power of the exponent.",
            &[number("base", "Base", Any, 1.0), number("exponent", "Exponent", Float, 2.0)],
            MATH_OUT,
        ),
        node("oneMinus", "One Minus", "Math", "1 − the input.", &[number("value", "Value", Any, 0.0)], MATH_OUT),
        node("clamp", "Clamp 0 to 1", "Math", "Keeps the input between 0 and 1.", &[number("value", "Value", Any, 0.0)], MATH_OUT),
        node("sine", "Sine", "Math", "The sine of the input, in radians.", &[number("value", "Value", Any, 0.0)], MATH_OUT),
        node(
            "checker",
            "Checker",
            "Patterns",
            "Alternating 0 and 1 cubes in space.",
            &[PinDef::with("position", "Position", Vec3, PinDefault::Implicit("position")), number("scale", "Cells per Unit", Float, 1.0)],
            &[pin("value", "Value", Float)],
        ),
        node(
            "noise",
            "Noise",
            "Patterns",
            "Smooth random values between 0 and 1.",
            &[PinDef::with("position", "Position", Vec3, PinDefault::Implicit("position")), number("scale", "Scale", Float, 1.0)],
            &[pin("value", "Value", Float)],
        ),
    ],
};

/// Most images one material may use.
pub const MAX_TEXTURES: usize = 4;

/// A material graph turned into shader code.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledMaterial {
    /// The image files its Image nodes read, by texture slot, as written in the graph.
    pub textures: Vec<String>,
    /// WGSL `fn surface(s: SurfaceIn) -> Surface`.
    pub surface: String,
    /// Identifies the shader: equal code, equal key.
    pub key: u64,
    /// Drawn with blending, after opaque objects.
    pub transparent: bool,
    /// Uses time, so views that show it keep redrawing.
    pub animated: bool,
    /// An outline around the shape: its colour (linear RGB) and width in world units.
    pub outline: Option<([f32; 3], f32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    F,
    V,
}

/// WGSL code and its type.
type Code = (String, Ty);

struct Compiler<'g> {
    graph: &'g Graph,
    lines: Vec<String>,
    textures: Vec<String>,
    sampled: std::collections::HashSet<u32>,
    done: HashMap<(u32, &'static str), Code>,
    animated: bool,
    /// Reads Main Light or Other Lights, so the shader works them out first.
    uses_lights: bool,
}

/// Compiles a material graph. Problems name the node they are on.
pub fn compile(graph: &Graph) -> Result<CompiledMaterial, Vec<Problem>> {
    if let Err(e) = graph.check(&CATALOG) {
        return Err(vec![Problem::graph(e.to_string())]);
    }
    let outputs: Vec<&bayleaf_graph::Node> = graph.nodes.iter().filter(|n| n.kind == "output" || n.kind == "customLighting").collect();
    let output = match outputs.as_slice() {
        [one] => *one,
        [] => return Err(vec![Problem::graph("add a Material Output node (or a Custom Lighting Output)")]),
        _ => return Err(vec![Problem::graph("a material has one output: remove the Material Output or the Custom Lighting Output")]),
    };
    let custom = output.kind == "customLighting";
    let mut compiler =
        Compiler { graph, lines: Vec::new(), textures: Vec::new(), sampled: Default::default(), done: HashMap::new(), animated: false, uses_lights: false };
    let mut problems = Vec::new();
    let mut fields = Vec::new();
    // Custom lighting: its colour goes out as emission, which the shader shows as it is.
    let pins: &[(&str, Ty)] = if custom {
        &[("color", Ty::V), ("alpha", Ty::F)]
    } else {
        &[("baseColor", Ty::V), ("metallic", Ty::F), ("roughness", Ty::F), ("emissive", Ty::V), ("opacity", Ty::F)]
    };
    for (pin, want) in pins {
        match compiler.input(output.id, pin).and_then(|value| convert(value, *want).map_err(|m| problem(output.id, &m))) {
            Ok(code) => fields.push(code),
            Err(p) => problems.push(p),
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    if custom {
        let (color, alpha) = (fields[0].clone(), fields[1].clone());
        fields = vec!["vec3<f32>(0.0)".to_owned(), "0.0".to_owned(), "1.0".to_owned(), color, alpha];
    }
    // Material graphs keep the surface's own normal.
    fields.push("s.normal".to_owned());
    let opacity = if custom { "alpha" } else { "opacity" };
    let transparent = graph.link_into(output.id, opacity).is_some() || graph.value(output.id, opacity).and_then(bayleaf_graph::as_f32).is_some_and(|v| v < 1.0);
    let width = graph.value(output.id, "outlineWidth").and_then(bayleaf_graph::as_f32).filter(|w| w.is_finite() && *w > 0.0);
    let outline = width.map(|w| (graph.value(output.id, "outlineColor").and_then(bayleaf_graph::as_triple).unwrap_or([0.0; 3]), w.min(MAX_OUTLINE)));
    let mut surface = String::from("fn surface(s: SurfaceIn) -> Surface {\n");
    for line in &compiler.lines {
        let _ = writeln!(surface, "    {line}");
    }
    let _ = writeln!(surface, "    return Surface({});\n}}", fields.join(", "));
    let (outline_color, outline_width) = outline.unwrap_or(([0.0; 3], 0.0));
    let _ = writeln!(surface, "const USES_LIGHTS: bool = {};", compiler.uses_lights);
    let _ = writeln!(surface, "const CUSTOM_LIGHTING: bool = {custom};");
    let _ = writeln!(surface, "const OUTLINE: vec4<f32> = vec4<f32>({}, {});", vec_lit(outline_color), lit(outline_width));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    surface.hash(&mut hasher);
    transparent.hash(&mut hasher);
    Ok(CompiledMaterial { surface, key: hasher.finish() | 1, transparent, animated: compiler.animated, textures: compiler.textures, outline })
}

/// Widest outline, in world units.
const MAX_OUTLINE: f32 = 1.0;

fn problem(node: u32, message: &str) -> Problem {
    Problem::on(node, message)
}

fn convert((code, ty): Code, want: Ty) -> Result<String, String> {
    match (ty, want) {
        (Ty::F, Ty::V) => Ok(format!("vec3<f32>({code})")),
        (Ty::V, Ty::F) => Err("this input takes a number but gets a vector or colour; use Split Vector".to_owned()),
        _ => Ok(code),
    }
}

/// A WGSL float literal.
fn lit(value: f32) -> String {
    let text = format!("{value:?}");
    let text = if text.contains(['.', 'e']) { text } else { format!("{text}.0") };
    // The `f` suffix makes the literal an f32, so built-ins like `mix` never see only abstract floats.
    let text = format!("{text}f");
    if value < 0.0 { format!("({text})") } else { text }
}

fn vec_lit(v: [f32; 3]) -> String {
    format!("vec3<f32>({}, {}, {})", lit(v[0]), lit(v[1]), lit(v[2]))
}

impl Compiler<'_> {
    /// The code and type of an input: its link, its stored value, or its default.
    fn input(&mut self, node: u32, pin: &str) -> Result<Code, Problem> {
        let graph = self.graph;
        let (_, def) = graph.node_def(&CATALOG, node).map_err(|e| problem(node, &e.to_string()))?;
        let pin_def = def.input(pin).ok_or_else(|| problem(node, &format!("{} has no input {pin}", def.label)))?;
        if let Some(link) = self.graph.link_into(node, pin) {
            let (from, from_pin) = (link.from.node, link.from.pin.clone());
            return self.output(from, &from_pin);
        }
        let declared = match pin_def.ty {
            Float => Ty::F,
            _ => Ty::V,
        };
        if let Some(value) = self.graph.value(node, pin) {
            if let Some(v) = bayleaf_graph::as_f32(value) {
                return Ok((lit(v), Ty::F));
            }
            if let Some(v) = bayleaf_graph::as_triple(value) {
                return Ok((vec_lit(v), Ty::V));
            }
        }
        Ok(match pin_def.default {
            PinDefault::Float(v) => (lit(v), if pin_def.ty == Any { Ty::F } else { declared }),
            PinDefault::Vec3(v) | PinDefault::Color(v) => (vec_lit(v), Ty::V),
            PinDefault::Implicit("normal") => ("s.normal".to_owned(), Ty::V),
            PinDefault::Implicit("uv") => ("vec3<f32>(s.uv, 0.0)".to_owned(), Ty::V),
            // A world position, as the World Position node gives it.
            PinDefault::Implicit(_) => ("(s.position + globals.origin.xyz)".to_owned(), Ty::V),
            PinDefault::Bool(_) | PinDefault::Text(_) | PinDefault::Int(_) | PinDefault::None => {
                (if declared == Ty::F { "0.0f".to_owned() } else { "vec3<f32>(0.0)".to_owned() }, declared)
            }
        })
        .map(|(code, ty)| if declared == Ty::V && ty == Ty::F && pin_def.ty != Any { (format!("vec3<f32>({code})"), Ty::V) } else { (code, ty) })
    }

    fn input_as(&mut self, node: u32, pin: &str, want: Ty) -> Result<String, Problem> {
        let value = self.input(node, pin)?;
        convert(value, want).map_err(|m| problem(node, &m))
    }

    /// The variable holding one output of a node, computing the node first if needed.
    fn output(&mut self, node: u32, pin: &str) -> Result<Code, Problem> {
        let graph = self.graph;
        let (n, def) = graph.node_def(&CATALOG, node).map_err(|e| problem(node, &e.to_string()))?;
        let pin_def = def.output(pin).ok_or_else(|| problem(node, &format!("{} has no output {pin}", def.label)))?;
        if let Some(done) = self.done.get(&(node, pin_def.name)) {
            return Ok(done.clone());
        }
        let name = |p: &str| format!("n{node}_{p}");
        let (code, ty): Code = match n.kind.as_str() {
            "number" => (lit(self.graph.value(node, "value").and_then(bayleaf_graph::as_f32).unwrap_or(1.0)), Ty::F),
            "color" => (vec_lit(self.graph.value(node, "color").and_then(bayleaf_graph::as_triple).unwrap_or([1.0; 3])), Ty::V),
            "vector" => {
                let (x, y, z) = (self.input_as(node, "x", Ty::F)?, self.input_as(node, "y", Ty::F)?, self.input_as(node, "z", Ty::F)?);
                (format!("vec3<f32>({x}, {y}, {z})"), Ty::V)
            }
            "split" => {
                let v = self.input_as(node, "vector", Ty::V)?;
                (format!("({v}).{pin}"), Ty::F)
            }
            "objectColor" if pin == "alpha" => ("s.color.a".to_owned(), Ty::F),
            "objectColor" => ("s.color.rgb".to_owned(), Ty::V),
            "time" => {
                self.animated = true;
                ("s.time".to_owned(), Ty::F)
            }
            // Measured from the world's centre: the surface's positions start from the view's origin,
            // which moves with the eye, so a pattern from them would slide along with the camera.
            "position" => ("(s.position + globals.origin.xyz)".to_owned(), Ty::V),
            "uv" => ("vec3<f32>(s.uv, 0.0)".to_owned(), Ty::V),
            "texture" => {
                let sample = format!("n{node}_sample");
                if self.sampled.insert(node) {
                    let image = graph.value(node, "image").and_then(|v| v.as_str()).unwrap_or_default().trim().to_owned();
                    if image.is_empty() {
                        return Err(problem(node, "choose a PNG image for this Image node"));
                    }
                    let slot = match self.textures.iter().position(|t| *t == image) {
                        Some(slot) => slot,
                        None if self.textures.len() < MAX_TEXTURES => {
                            self.textures.push(image);
                            self.textures.len() - 1
                        }
                        None => return Err(problem(node, &format!("a material may use at most {MAX_TEXTURES} images"))),
                    };
                    let uv = self.input_as(node, "uv", Ty::V)?;
                    self.lines.push(format!("let {sample} = textureSample(t{slot}, material_sampler, ({uv}).xy);"));
                }
                if pin == "alpha" { (format!("{sample}.a"), Ty::F) } else { (format!("{sample}.rgb"), Ty::V) }
            }
            "normal" => ("s.normal".to_owned(), Ty::V),
            "fresnel" => {
                let power = self.input_as(node, "power", Ty::F)?;
                (format!("pow(1.0 - clamp(dot(s.normal, s.view), 0.0, 1.0), max({power}, 0.0))"), Ty::F)
            }
            "add" | "subtract" | "multiply" | "divide" => {
                let op = match n.kind.as_str() {
                    "add" => "+",
                    "subtract" => "-",
                    "multiply" => "*",
                    _ => "/",
                };
                let (a, b) = self.pair(node, "a", "b")?;
                (format!("({} {op} {})", a.0, b.0), a.1)
            }
            "blend" => {
                let (a, b) = self.pair(node, "a", "b")?;
                let factor = self.input_as(node, "factor", Ty::F)?;
                (format!("mix({}, {}, {factor})", a.0, b.0), a.1)
            }
            "power" => {
                let (base, ty) = self.input(node, "base")?;
                let exponent = self.input_as(node, "exponent", Ty::F)?;
                let exponent = if ty == Ty::V { format!("vec3<f32>({exponent})") } else { exponent };
                (format!("pow(max({base}, {}), {exponent})", if ty == Ty::V { "vec3<f32>(0.0)" } else { "0.0" }), ty)
            }
            "oneMinus" => {
                let (v, ty) = self.input(node, "value")?;
                (format!("(1.0 - {v})"), ty)
            }
            "clamp" => {
                let (v, ty) = self.input(node, "value")?;
                (
                    format!(
                        "clamp({v}, {z}, {o})",
                        z = if ty == Ty::V { "vec3<f32>(0.0)" } else { "0.0" },
                        o = if ty == Ty::V { "vec3<f32>(1.0)" } else { "1.0" }
                    ),
                    ty,
                )
            }
            "sine" => {
                let (v, ty) = self.input(node, "value")?;
                (format!("sin({v})"), ty)
            }
            "checker" => {
                let p = self.input_as(node, "position", Ty::V)?;
                let scale = self.input_as(node, "scale", Ty::F)?;
                (format!("bl_checker({p} * {scale})"), Ty::F)
            }
            "noise" => {
                let p = self.input_as(node, "position", Ty::V)?;
                let scale = self.input_as(node, "scale", Ty::F)?;
                (format!("bl_noise({p} * {scale})"), Ty::F)
            }
            "mainLight" => {
                self.uses_lights = true;
                match pin {
                    "direction" => ("s.light_dir".to_owned(), Ty::V),
                    "color" => ("s.light_color".to_owned(), Ty::V),
                    _ => ("s.light_shadow".to_owned(), Ty::F),
                }
            }
            "otherLights" => {
                self.uses_lights = true;
                ("s.other_light".to_owned(), Ty::V)
            }
            "viewDirection" => ("s.view".to_owned(), Ty::V),
            "screenPosition" => ("vec3<f32>(s.screen, 0.0)".to_owned(), Ty::V),
            "dot" => {
                let (a, b) = (self.input_as(node, "a", Ty::V)?, self.input_as(node, "b", Ty::V)?);
                (format!("dot({a}, {b})"), Ty::F)
            }
            "normalize" => {
                let v = self.input_as(node, "vector", Ty::V)?;
                (format!("normalize({v} + vec3<f32>(0.0, 1e-9, 0.0))"), Ty::V)
            }
            "step" => {
                let (v, ty) = self.input(node, "value")?;
                let edge = self.input_as(node, "edge", Ty::F)?;
                (format!("step({}, {v})", widen(&edge, ty)), ty)
            }
            "smoothstep" => {
                let (v, ty) = self.input(node, "value")?;
                let (low, high) = (self.input_as(node, "low", Ty::F)?, self.input_as(node, "high", Ty::F)?);
                (format!("smoothstep({}, {}, {v})", widen(&low, ty), widen(&high, ty)), ty)
            }
            "posterize" => {
                let (v, ty) = self.input(node, "value")?;
                let steps = self.input_as(node, "steps", Ty::F)?;
                let bands = format!("max(floor({steps}), 2.0)");
                let (zero, one) = (widen("0.0", ty), widen("1.0", ty));
                (format!("clamp(floor({v} * {bands}) / ({bands} - 1.0), {zero}, {one})"), ty)
            }
            "remap" => {
                let (v, ty) = self.input(node, "value")?;
                let [from_low, from_high, to_low, to_high] = [
                    self.input_as(node, "fromLow", Ty::F)?,
                    self.input_as(node, "fromHigh", Ty::F)?,
                    self.input_as(node, "toLow", Ty::F)?,
                    self.input_as(node, "toHigh", Ty::F)?,
                ];
                // A zero-wide range divides by a tiny number instead of zero.
                let span = format!("select({from_high} - {from_low}, 1e-6, abs({from_high} - {from_low}) < 1e-6)");
                (format!("({to_low} + ({v} - {from_low}) / {span} * ({to_high} - {to_low}))"), ty)
            }
            other => return Err(problem(node, &format!("{other} nodes do not belong in a material"))),
        };
        let variable = name(pin_def.name);
        self.lines.push(format!("let {variable} = {code};"));
        self.done.insert((node, pin_def.name), (variable.clone(), ty));
        Ok((variable, ty))
    }

    /// Two inputs brought to the same type: a number next to a vector becomes a vector.
    fn pair(&mut self, node: u32, a: &str, b: &str) -> Result<(Code, Code), Problem> {
        let a = self.input(node, a)?;
        let b = self.input(node, b)?;
        if a.1 == b.1 {
            return Ok((a, b));
        }
        let widen = |(code, ty): Code| if ty == Ty::F { (format!("vec3<f32>({code})"), Ty::V) } else { (code, ty) };
        Ok((widen(a), widen(b)))
    }
}

/// Material files, compiled once and again when the file changes.
#[derive(Default)]
pub struct MaterialLibrary {
    entries: HashMap<PathBuf, Entry>,
    /// Graphs being edited, used instead of their files so changes show before they are saved:
    /// by file, the graph and what it compiled to (an unchanged graph is not compiled again).
    live: HashMap<PathBuf, (Graph, Result<Arc<CompiledMaterial>, String>)>,
    /// Lighting files, read once and again when the file changes.
    lightings: HashMap<PathBuf, LightingEntry>,
    /// Post-processing files, read once and again when the file changes.
    posts: HashMap<PathBuf, PostEntry>,
}

/// A post-processing file as last read: when it changed, and what it held or why it did not read.
type PostEntry = (Option<SystemTime>, Result<Arc<bayleaf_scene::post::PostProcessing>, String>);

/// A lighting file as last read: when it changed, and what it held or why it did not read.
type LightingEntry = (Option<SystemTime>, Result<Arc<bayleaf_scene::lighting::Lighting>, String>);

struct Entry {
    modified: Option<SystemTime>,
    result: Result<Arc<CompiledMaterial>, String>,
}

impl MaterialLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// A lighting file (`.blight`), read again when it changes on disk.
    pub fn lighting(&mut self, path: &Path) -> Result<Arc<bayleaf_scene::lighting::Lighting>, String> {
        let key = normalize(path);
        let modified = std::fs::metadata(&key).and_then(|m| m.modified()).ok();
        if let Some((seen, result)) = self.lightings.get(&key)
            && *seen == modified
            && modified.is_some()
        {
            return result.clone();
        }
        let result = bayleaf_scene::lighting::Lighting::load(&key).map(Arc::new).map_err(|e| e.to_string());
        self.lightings.insert(key, (modified, result.clone()));
        result
    }

    /// A post-processing file (`.bpost`), read again when it changes on disk.
    pub fn post(&mut self, path: &Path) -> Result<Arc<bayleaf_scene::post::PostProcessing>, String> {
        let key = normalize(path);
        let modified = std::fs::metadata(&key).and_then(|m| m.modified()).ok();
        if let Some((seen, result)) = self.posts.get(&key)
            && *seen == modified
            && modified.is_some()
        {
            return result.clone();
        }
        let result = bayleaf_scene::post::PostProcessing::load(&key).map(Arc::new).map_err(|e| e.to_string());
        self.posts.insert(key, (modified, result.clone()));
        result
    }

    /// Uses `graph` for `path` instead of the file, or stops with `None`.
    pub fn set_live(&mut self, live: Option<(&Path, &Graph)>) {
        self.set_lives(live.into_iter().collect());
    }

    /// Uses each graph for its file instead of the file; files not named here are read again.
    pub fn set_lives(&mut self, lives: Vec<(&Path, &Graph)>) {
        let keys: Vec<PathBuf> = lives.iter().map(|(p, _)| normalize(p)).collect();
        self.live.retain(|k, _| keys.contains(k));
        for (key, (_, graph)) in keys.into_iter().zip(lives) {
            if self.live.get(&key).is_some_and(|(seen, _)| seen == graph) {
                continue;
            }
            let compiled = compile(graph).map(Arc::new).map_err(|p| summary(&p));
            self.live.insert(key, (graph.clone(), compiled));
        }
    }

    /// The compiled material at `path`, reading and compiling the file if it is new or changed.
    pub fn get(&mut self, path: &Path) -> Result<Arc<CompiledMaterial>, String> {
        let key = normalize(path);
        if let Some((_, result)) = self.live.get(&key) {
            return result.clone();
        }
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some(entry) = self.entries.get(&key)
            && entry.modified == modified
            && modified.is_some()
        {
            return entry.result.clone();
        }
        let result = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))
            .and_then(|text| Graph::from_json(&text, &CATALOG).map_err(|e| format!("{}: {e}", path.display())))
            .and_then(|graph| compile(&graph).map(Arc::new).map_err(|p| format!("{}: {}", path.display(), summary(&p))));
        self.entries.insert(key, Entry { modified, result: result.clone() });
        result
    }
}

/// One spelling per file, so a live graph matches the path a mesh uses for it.
fn normalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.components().collect())
}

/// A number as the type `ty` wants: itself, or three of it.
fn widen(code: &str, ty: Ty) -> String {
    if ty == Ty::V { format!("vec3<f32>({code})") } else { code.to_owned() }
}

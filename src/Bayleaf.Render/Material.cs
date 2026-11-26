using System.Numerics;
using System.Text;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Graph;

namespace Bayleaf.Render;

/// <summary>A material graph turned into shader code.</summary>
/// <param name="Textures">The image files its Image nodes read, by texture slot, as written in the graph.</param>
/// <param name="Surface">WGSL <c>fn surface(s: SurfaceIn) -> Surface</c>, with the constants the mesh shader reads.</param>
/// <param name="Key">Identifies the shader: equal code, equal key (within one engine; the Rust engine's keys differ).</param>
/// <param name="Transparent">Drawn with blending, after opaque objects.</param>
/// <param name="Animated">Uses time, so views that show it keep redrawing.</param>
/// <param name="Outline">An outline around the shape: its colour (linear RGB) and width in world units.</param>
public sealed record CompiledMaterial(IReadOnlyList<string> Textures, string Surface, ulong Key, bool Transparent, bool Animated, (Vector3 Color, float Width)? Outline);

/// <summary>
/// Material graphs: their node kinds and the compiler that turns a graph into a WGSL surface function.
/// The mesh shader asks <c>surface(s)</c> for the base colour, metallic, roughness, emission and opacity
/// of each pixel and then applies the lighting. Meshes without a material use a built-in <c>surface</c>
/// that reads the mesh component's colour and sliders; a material graph replaces it.
/// </summary>
public static partial class Material
{
    /// <summary>Most images one material may use.</summary>
    public const int MaxTextures = 4;

    /// <summary>Widest outline, in world units.</summary>
    private const float MaxOutline = 1f;

    /// <summary>Compiles a material graph. Problems name the node they are on.</summary>
    public static Result<CompiledMaterial, IReadOnlyList<Problem>> Compile(Graph.Graph graph)
    {
        var valid = graph.Check(Catalog);
        if (valid.IsError)
        {
            return Fail(Problem.OfGraph(valid.Error.Message));
        }
        var outputs = graph.Nodes.Where(n => n.Kind is "output" or "customLighting").ToList();
        if (outputs.Count == 0)
        {
            return Fail(Problem.OfGraph("add a Material Output node (or a Custom Lighting Output)"));
        }
        if (outputs.Count > 1)
        {
            return Fail(Problem.OfGraph("a material has one output: remove the Material Output or the Custom Lighting Output"));
        }
        var output = outputs[0];
        var custom = output.Kind == "customLighting";
        var compiler = new Compiler(graph);
        var problems = new List<Problem>();
        var fields = new List<string>();
        // Custom lighting: its colour goes out as emission, which the shader shows as it is.
        (string Pin, Ty Want)[] pins = custom
            ? [("color", Ty.V), ("alpha", Ty.F)]
            : [("baseColor", Ty.V), ("metallic", Ty.F), ("roughness", Ty.F), ("emissive", Ty.V), ("opacity", Ty.F)];
        foreach (var (pin, want) in pins)
        {
            var code = compiler.Input(output.Id, pin);
            if (code.TryGet(out var value, out var problem))
            {
                if (Convert(value, want).TryGet(out var converted, out var message))
                {
                    fields.Add(converted);
                }
                else
                {
                    problems.Add(Problem.On(output.Id, message));
                }
            }
            else
            {
                problems.Add(problem);
            }
        }
        if (problems.Count > 0)
        {
            return Result.Fail<IReadOnlyList<Problem>>(problems);
        }
        if (custom)
        {
            fields = ["vec3<f32>(0.0)", "0.0", "1.0", fields[0], fields[1]];
        }
        // Material graphs keep the surface's own normal.
        fields.Add("s.normal");
        var opacity = custom ? "alpha" : "opacity";
        var transparent = graph.LinkInto(output.Id, opacity) is not null || (graph.ValueOf(output.Id, opacity) is { } o && Graph.Graph.AsF32(o) is < 1f);
        (Vector3 Color, float Width)? outline = null;
        if (graph.ValueOf(output.Id, "outlineWidth") is { } w && Graph.Graph.AsF32(w) is { } width && float.IsFinite(width) && width > 0f)
        {
            var color = graph.ValueOf(output.Id, "outlineColor") is { } c && Graph.Graph.AsTriple(c) is { } triple ? triple : Vector3.Zero;
            outline = (color, Math.Min(width, MaxOutline));
        }
        var surface = new StringBuilder("fn surface(s: SurfaceIn) -> Surface {\n");
        foreach (var line in compiler.Lines)
        {
            surface.Append("    ").Append(line).Append('\n');
        }
        surface.Append("    return Surface(").Append(string.Join(", ", fields)).Append(");\n}\n");
        var (outlineColor, outlineWidth) = outline ?? (Vector3.Zero, 0f);
        surface.Append("const USES_LIGHTS: bool = ").Append(Bool(compiler.UsesLights)).Append(";\n");
        surface.Append("const CUSTOM_LIGHTING: bool = ").Append(Bool(custom)).Append(";\n");
        surface.Append("const OUTLINE: vec4<f32> = vec4<f32>(").Append(VecLit(outlineColor)).Append(", ").Append(Lit(outlineWidth)).Append(");\n");
        var text = surface.ToString();
        return new CompiledMaterial(compiler.Textures, text, KeyOf(text, transparent), transparent, compiler.Animated, outline);
    }

    private static Result<CompiledMaterial, IReadOnlyList<Problem>> Fail(Problem problem) => Result.Fail<IReadOnlyList<Problem>>([problem]);

    private static string Bool(bool value) => value ? "true" : "false";

    /// <summary>A key for the shader: FNV-1a over its code and whether it blends, never 0.</summary>
    private static ulong KeyOf(string surface, bool transparent)
    {
        var hash = 14695981039346656037UL;
        foreach (var b in Encoding.UTF8.GetBytes(surface).Append(transparent ? (byte)1 : (byte)0))
        {
            hash = (hash ^ b) * 1099511628211UL;
        }
        return hash | 1;
    }

    /// <summary>The type of compiled code: a number or three of them.</summary>
    internal enum Ty
    {
        F,
        V,
    }

    private static Result<string, string> Convert((string Code, Ty Ty) value, Ty want) => (value.Ty, want) switch
    {
        (Ty.F, Ty.V) => $"vec3<f32>({value.Code})",
        (Ty.V, Ty.F) => Result.Fail("this input takes a number but gets a vector or colour; use Split Vector"),
        _ => value.Code,
    };

    /// <summary>A WGSL float literal, an <c>f32</c> (the <c>f</c> suffix keeps built-ins like <c>mix</c> from seeing only abstract floats).</summary>
    internal static string Lit(float value)
    {
        var text = RustFloat.Debug(value);
        if (!text.Contains('.', StringComparison.Ordinal) && !text.Contains('e', StringComparison.Ordinal))
        {
            text += ".0";
        }
        text += "f";
        return value < 0f ? $"({text})" : text;
    }

    internal static string VecLit(Vector3 v) => $"vec3<f32>({Lit(v.X)}, {Lit(v.Y)}, {Lit(v.Z)})";

    /// <summary>A number as the type <paramref name="ty"/> wants: itself, or three of it.</summary>
    private static string Widen(string code, Ty ty) => ty == Ty.V ? $"vec3<f32>({code})" : code;

    /// <summary>The zero of <paramref name="ty"/>, as math nodes write it.</summary>
    private static string Zero(Ty ty) => ty == Ty.V ? "vec3<f32>(0.0)" : "0.0";

    private sealed class Compiler(Graph.Graph graph)
    {
        public List<string> Lines { get; } = [];

        public List<string> Textures { get; } = [];

        public bool Animated { get; private set; }

        /// <summary>Reads Main Light or Other Lights, so the shader works them out first.</summary>
        public bool UsesLights { get; private set; }

        private readonly HashSet<uint> sampled = [];

        private readonly Dictionary<(uint Node, string Pin), (string Code, Ty Ty)> done = [];

        private static Problem On(uint node, string message) => Problem.On(node, message);

        /// <summary>The code and type of an input: its link, its stored value, or its default.</summary>
        public Result<(string Code, Ty Ty), Problem> Input(uint node, string pin)
        {
            if (!graph.NodeWithDef(Catalog, node).TryGet(out var found, out var error))
            {
                return Result.Fail(On(node, error.Message));
            }
            var def = found.Def;
            if (def.Input(pin) is not { } pinDef)
            {
                return Result.Fail(On(node, $"{def.Label} has no input {pin}"));
            }
            if (graph.LinkInto(node, pin) is { } link)
            {
                return Output(link.From.Node, link.From.Pin);
            }
            var declared = pinDef.Type == PinType.Number ? Ty.F : Ty.V;
            if (graph.ValueOf(node, pin) is { } value)
            {
                if (Graph.Graph.AsF32(value) is { } number)
                {
                    return (Lit(number), Ty.F);
                }
                if (Graph.Graph.AsTriple(value) is { } triple)
                {
                    return (VecLit(triple), Ty.V);
                }
            }
            (string Code, Ty Ty) code = pinDef.Default switch
            {
                PinDefault.Number n => (Lit(n.Value), pinDef.Type == PinType.Any ? Ty.F : declared),
                PinDefault.Vec3 v => (VecLit(v.Value), Ty.V),
                PinDefault.Color c => (VecLit(c.Value), Ty.V),
                PinDefault.Implicit { What: "normal" } => ("s.normal", Ty.V),
                PinDefault.Implicit { What: "uv" } => ("vec3<f32>(s.uv, 0.0)", Ty.V),
                // A world position, as the World Position node gives it.
                PinDefault.Implicit => ("(s.position + globals.origin.xyz)", Ty.V),
                _ => (declared == Ty.F ? "0.0f" : "vec3<f32>(0.0)", declared),
            };
            return declared == Ty.V && code.Ty == Ty.F && pinDef.Type != PinType.Any ? ($"vec3<f32>({code.Code})", Ty.V) : code;
        }

        private Result<string, Problem> InputAs(uint node, string pin, Ty want)
        {
            if (!Input(node, pin).TryGet(out var value, out var problem))
            {
                return Result.Fail(problem);
            }
            return Convert(value, want).MapError(m => On(node, m));
        }

        /// <summary>A setting's choice as its position in the list (0 for the first, or when unset).</summary>
        private int Choice(uint node, string name, IReadOnlyList<string> choices)
        {
            var chosen = graph.ValueOf(node, name) is JsonString s ? s.Value : "";
            var at = choices.ToList().IndexOf(chosen);
            return at < 0 ? 0 : at;
        }

        /// <summary>The inputs <paramref name="pins"/> of a node, each as <paramref name="want"/>, in order.</summary>
        private Result<string[], Problem> InputsAs(uint node, Ty want, params string[] pins)
        {
            var codes = new string[pins.Length];
            for (var i = 0; i < pins.Length; i++)
            {
                if (!InputAs(node, pins[i], want).TryGet(out var code, out var problem))
                {
                    return Result.Fail(problem);
                }
                codes[i] = code;
            }
            return codes;
        }

        /// <summary>Fractal Noise, Voronoi, Waves, Gradient and Random.</summary>
        private Result<(string Code, Ty Ty), Problem> Pattern(uint node, string kind, string pin)
        {
            switch (kind)
            {
                case "fractalNoise":
                {
                    if (!InputAs(node, "position", Ty.V).TryGet(out var p, out var bad) || !InputsAs(node, Ty.F, "scale", "detail", "roughness", "lacunarity").TryGet(out var n, out bad))
                    {
                        return Result.Fail(bad);
                    }
                    var style = Choice(node, "kind", FractalKinds);
                    return (JsonRead.Invariant($"bl_fractal({p} * {n[0]}, {n[1]}, {n[2]}, {n[3]}, {style})"), Ty.F);
                }
                case "voronoi":
                {
                    var cells = JsonRead.Invariant($"n{node}_cells");
                    if (sampled.Add(node))
                    {
                        if (!InputAs(node, "position", Ty.V).TryGet(out var p, out var bad) || !InputsAs(node, Ty.F, "scale", "randomness").TryGet(out var n, out bad))
                        {
                            return Result.Fail(bad);
                        }
                        Lines.Add($"let {cells} = bl_voronoi({p} * {n[0]}, {n[1]});");
                    }
                    return pin switch
                    {
                        "distance" => ($"{cells}.x", Ty.F),
                        "edge" => ($"({cells}.y - {cells}.x)", Ty.F),
                        _ => ($"{cells}.z", Ty.F),
                    };
                }
                case "waves":
                {
                    if (!InputAs(node, "position", Ty.V).TryGet(out var p, out var bad) || !InputsAs(node, Ty.F, "scale", "distortion").TryGet(out var n, out bad))
                    {
                        return Result.Fail(bad);
                    }
                    var rings = Choice(node, "kind", WaveKinds);
                    return (JsonRead.Invariant($"bl_waves({p} * {n[0]}, {n[1]}, {rings})"), Ty.F);
                }
                case "gradient":
                {
                    if (!InputAs(node, "uv", Ty.V).TryGet(out var uv, out var bad))
                    {
                        return Result.Fail(bad);
                    }
                    var style = Choice(node, "kind", GradientKinds);
                    return (JsonRead.Invariant($"bl_gradient({uv}, {style})"), Ty.F);
                }
                default:
                {
                    if (!InputAs(node, "position", Ty.V).TryGet(out var p, out var bad) || !InputAs(node, "scale", Ty.F).TryGet(out var scale, out bad))
                    {
                        return Result.Fail(bad);
                    }
                    return pin == "color" ? ($"bl_hash3(floor({p} * {scale}))", Ty.V) : ($"bl_hash(floor({p} * {scale}))", Ty.F);
                }
            }
        }

        /// <summary>The variable holding one output of a node, computing the node first if needed.</summary>
        public Result<(string Code, Ty Ty), Problem> Output(uint node, string pin)
        {
            if (!graph.NodeWithDef(Catalog, node).TryGet(out var found, out var error))
            {
                return Result.Fail(On(node, error.Message));
            }
            var (n, def) = found;
            if (def.Output(pin) is not { } pinDef)
            {
                return Result.Fail(On(node, $"{def.Label} has no output {pin}"));
            }
            if (done.TryGetValue((node, pinDef.Name), out var cached))
            {
                return cached;
            }
            if (!Compute(node, n.Kind, pin).TryGet(out var computed, out var problem))
            {
                return Result.Fail(problem);
            }
            var variable = JsonRead.Invariant($"n{node}_{pinDef.Name}");
            Lines.Add($"let {variable} = {computed.Code};");
            done[(node, pinDef.Name)] = (variable, computed.Ty);
            return (variable, computed.Ty);
        }

        private Result<(string Code, Ty Ty), Problem> Compute(uint node, string kind, string pin)
        {
            Result<(string Code, Ty Ty), Problem> Bad(Problem problem) => Result.Fail(problem);
            switch (kind)
            {
                case "number":
                    return (Lit(graph.ValueOf(node, "value") is { } v && Graph.Graph.AsF32(v) is { } f ? f : 1f), Ty.F);
                case "color":
                    return (VecLit(graph.ValueOf(node, "color") is { } c && Graph.Graph.AsTriple(c) is { } t ? t : Vector3.One), Ty.V);
                case "vector":
                    return InputsAs(node, Ty.F, "x", "y", "z").TryGet(out var xyz, out var badVector) ? ($"vec3<f32>({xyz[0]}, {xyz[1]}, {xyz[2]})", Ty.V) : Bad(badVector);
                case "split":
                    return InputAs(node, "vector", Ty.V).TryGet(out var split, out var badSplit) ? ($"({split}).{pin}", Ty.F) : Bad(badSplit);
                case "objectColor":
                    return pin == "alpha" ? ("s.color.a", Ty.F) : ("s.color.rgb", Ty.V);
                case "time":
                    Animated = true;
                    return ("s.time", Ty.F);
                // Measured from the world's centre: the surface's positions start from the view's origin,
                // which moves with the eye, so a pattern from them would slide along with the camera.
                case "position":
                    return ("(s.position + globals.origin.xyz)", Ty.V);
                case "uv":
                    return ("vec3<f32>(s.uv, 0.0)", Ty.V);
                case "texture":
                    return Texture(node, pin);
                case "normal":
                    return ("s.normal", Ty.V);
                case "fresnel":
                    return InputAs(node, "power", Ty.F).TryGet(out var power, out var badPower)
                        ? ($"pow(1.0 - clamp(dot(s.normal, s.view), 0.0, 1.0), max({power}, 0.0))", Ty.F)
                        : Bad(badPower);
                case "add" or "subtract" or "multiply" or "divide":
                {
                    var op = kind switch
                    {
                        "add" => "+",
                        "subtract" => "-",
                        "multiply" => "*",
                        _ => "/",
                    };
                    return Pair(node, "a", "b").TryGet(out var ab, out var bad) ? ($"({ab.A.Code} {op} {ab.B.Code})", ab.A.Ty) : Bad(bad);
                }
                case "blend":
                {
                    if (!Pair(node, "a", "b").TryGet(out var ab, out var bad) || !InputAs(node, "factor", Ty.F).TryGet(out var factor, out bad))
                    {
                        return Bad(bad);
                    }
                    return ($"mix({ab.A.Code}, {ab.B.Code}, {factor})", ab.A.Ty);
                }
                case "inverseLerp":
                {
                    if (!Input(node, "value").TryGet(out var value, out var bad) || !InputsAs(node, Ty.F, "a", "b").TryGet(out var ends, out bad))
                    {
                        return Bad(bad);
                    }
                    var (a, b) = (ends[0], ends[1]);
                    // Equal ends give 0 instead of dividing by zero.
                    var span = $"select({b} - {a}, 1.0, {b} == {a})";
                    return ($"select(({value.Code} - {Widen(a, value.Ty)}) / {span}, {Widen("0.0", value.Ty)}, {b} == {a})", value.Ty);
                }
                case "power":
                {
                    if (!Input(node, "base").TryGet(out var b, out var bad) || !InputAs(node, "exponent", Ty.F).TryGet(out var exponent, out bad))
                    {
                        return Bad(bad);
                    }
                    var e = b.Ty == Ty.V ? $"vec3<f32>({exponent})" : exponent;
                    return ($"pow(max({b.Code}, {Zero(b.Ty)}), {e})", b.Ty);
                }
                case "squareRoot":
                    return Input(node, "value").TryGet(out var root, out var badRoot) ? ($"sqrt(max({root.Code}, {Zero(root.Ty)}))", root.Ty) : Bad(badRoot);
                case "vectorLength":
                    return InputAs(node, "vector", Ty.V).TryGet(out var length, out var badLength) ? ($"length({length})", Ty.F) : Bad(badLength);
                case "oneMinus":
                    return Input(node, "value").TryGet(out var minus, out var badMinus) ? ($"(1.0 - {minus.Code})", minus.Ty) : Bad(badMinus);
                case "clamp":
                    return Input(node, "value").TryGet(out var clamp, out var badClamp)
                        ? ($"clamp({clamp.Code}, {Zero(clamp.Ty)}, {(clamp.Ty == Ty.V ? "vec3<f32>(1.0)" : "1.0")})", clamp.Ty)
                        : Bad(badClamp);
                case "sine":
                    return Input(node, "value").TryGet(out var sine, out var badSine) ? ($"sin({sine.Code})", sine.Ty) : Bad(badSine);
                case "fractalNoise" or "voronoi" or "waves" or "gradient" or "random":
                    return Pattern(node, kind, pin);
                case "checker" or "noise":
                {
                    if (!InputAs(node, "position", Ty.V).TryGet(out var p, out var bad) || !InputAs(node, "scale", Ty.F).TryGet(out var scale, out bad))
                    {
                        return Bad(bad);
                    }
                    return ($"bl_{kind}({p} * {scale})", Ty.F);
                }
                case "mainLight":
                    UsesLights = true;
                    return pin switch
                    {
                        "direction" => ("s.light_dir", Ty.V),
                        "color" => ("s.light_color", Ty.V),
                        _ => ("s.light_shadow", Ty.F),
                    };
                case "otherLights":
                    UsesLights = true;
                    return ("s.other_light", Ty.V);
                case "viewDirection":
                    return ("s.view", Ty.V);
                case "screenPosition":
                    return ("vec3<f32>(s.screen, 0.0)", Ty.V);
                case "dot":
                    return InputsAs(node, Ty.V, "a", "b").TryGet(out var dot, out var badDot) ? ($"dot({dot[0]}, {dot[1]})", Ty.F) : Bad(badDot);
                case "normalize":
                    return InputAs(node, "vector", Ty.V).TryGet(out var normal, out var badNormal) ? ($"normalize({normal} + vec3<f32>(0.0, 1e-9, 0.0))", Ty.V) : Bad(badNormal);
                case "step":
                {
                    if (!Input(node, "value").TryGet(out var value, out var bad) || !InputAs(node, "edge", Ty.F).TryGet(out var edge, out bad))
                    {
                        return Bad(bad);
                    }
                    return ($"step({Widen(edge, value.Ty)}, {value.Code})", value.Ty);
                }
                case "smoothstep":
                {
                    if (!Input(node, "value").TryGet(out var value, out var bad) || !InputsAs(node, Ty.F, "low", "high").TryGet(out var ends, out bad))
                    {
                        return Bad(bad);
                    }
                    return ($"smoothstep({Widen(ends[0], value.Ty)}, {Widen(ends[1], value.Ty)}, {value.Code})", value.Ty);
                }
                case "posterize":
                {
                    if (!Input(node, "value").TryGet(out var value, out var bad) || !InputAs(node, "steps", Ty.F).TryGet(out var steps, out bad))
                    {
                        return Bad(bad);
                    }
                    var bands = $"max(floor({steps}), 2.0)";
                    return ($"clamp(floor({value.Code} * {bands}) / ({bands} - 1.0), {Widen("0.0", value.Ty)}, {Widen("1.0", value.Ty)})", value.Ty);
                }
                case "remap":
                {
                    if (!Input(node, "value").TryGet(out var value, out var bad) || !InputsAs(node, Ty.F, "fromLow", "fromHigh", "toLow", "toHigh").TryGet(out var r, out bad))
                    {
                        return Bad(bad);
                    }
                    var (fromLow, fromHigh, toLow, toHigh) = (r[0], r[1], r[2], r[3]);
                    // A zero-wide range divides by a tiny number instead of zero.
                    var span = $"select({fromHigh} - {fromLow}, 1e-6, abs({fromHigh} - {fromLow}) < 1e-6)";
                    return ($"({toLow} + ({value.Code} - {fromLow}) / {span} * ({toHigh} - {toLow}))", value.Ty);
                }
                default:
                    return Bad(On(node, $"{kind} nodes do not belong in a material"));
            }
        }

        private Result<(string Code, Ty Ty), Problem> Texture(uint node, string pin)
        {
            var sample = JsonRead.Invariant($"n{node}_sample");
            if (sampled.Add(node))
            {
                var image = (graph.ValueOf(node, "image") is JsonString s ? s.Value : "").Trim();
                if (image.Length == 0)
                {
                    return Result.Fail(On(node, "choose a PNG image for this Image node"));
                }
                var slot = Textures.IndexOf(image);
                if (slot < 0)
                {
                    if (Textures.Count >= MaxTextures)
                    {
                        return Result.Fail(On(node, JsonRead.Invariant($"a material may use at most {MaxTextures} images")));
                    }
                    Textures.Add(image);
                    slot = Textures.Count - 1;
                }
                if (!InputAs(node, "uv", Ty.V).TryGet(out var uv, out var bad))
                {
                    return Result.Fail(bad);
                }
                Lines.Add(JsonRead.Invariant($"let {sample} = textureSample(t{slot}, material_sampler, ({uv}).xy);"));
            }
            return pin == "alpha" ? ($"{sample}.a", Ty.F) : ($"{sample}.rgb", Ty.V);
        }

        /// <summary>Two inputs brought to the same type: a number next to a vector becomes a vector.</summary>
        private Result<((string Code, Ty Ty) A, (string Code, Ty Ty) B), Problem> Pair(uint node, string a, string b)
        {
            if (!Input(node, a).TryGet(out var first, out var bad) || !Input(node, b).TryGet(out var second, out bad))
            {
                return Result.Fail(bad);
            }
            if (first.Ty == second.Ty)
            {
                return (first, second);
            }
            static (string, Ty) Up((string Code, Ty Ty) v) => v.Ty == Ty.F ? ($"vec3<f32>({v.Code})", Ty.V) : v;
            return (Up(first), Up(second));
        }
    }
}

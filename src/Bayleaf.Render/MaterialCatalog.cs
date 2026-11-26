using System.Numerics;

using Bayleaf.Graph;

namespace Bayleaf.Render;

/// <summary>The node kinds of material graphs (domain <c>material</c>, files <c>.bmaterial</c>).</summary>
public static partial class Material
{
    private static PinDef Pin(string name, string label, PinType type) => new(name, label, type);

    private static PinDef Number(string name, string label, PinType type, float value) => new(name, label, type, new PinDefault.Number(value));

    private static PinDef Colour(string name, string label, float r, float g, float b) => new(name, label, PinType.Color, new PinDefault.Color(new Vector3(r, g, b)));

    private static PinDef Implicit(string name, string label, string what) => new(name, label, PinType.Vec3, new PinDefault.Implicit(what));

    private static NodeDef Node(string kind, string label, string category, string description, IReadOnlyList<PinDef> inputs, IReadOnlyList<PinDef> outputs) =>
        new(kind, label, category, description, inputs, outputs, [], false);

    private static NodeDef Node(string kind, string label, string category, string description, IReadOnlyList<PinDef> inputs, IReadOnlyList<PinDef> outputs, IReadOnlyList<SettingDef> settings) =>
        new(kind, label, category, description, inputs, outputs, settings, false);

    private static readonly IReadOnlyList<PinDef> MathOut = [Pin("result", "Result", PinType.Any)];

    /// <summary>The outline option of a material's output: back faces grown by the width, in the colour.</summary>
    private static readonly IReadOnlyList<SettingDef> OutlineSettings =
    [
        new("outlineWidth", "Outline Width", new SettingKind.Number(0f)),
        new("outlineColor", "Outline Color", new SettingKind.Color(Vector3.Zero)),
    ];

    private static readonly PinType F = PinType.Number;
    private static readonly PinType V = PinType.Vec3;
    private static readonly PinType C = PinType.Color;
    private static readonly PinType A = PinType.Any;

    // The choices of the patterns' Kind settings, which the compiler passes on as their place in the list.
    // They stay in this file, above the catalog, so they are set before it.
    private static readonly IReadOnlyList<string> FractalKinds = ["Smooth", "Ridged", "Billowy"];
    private static readonly IReadOnlyList<string> WaveKinds = ["Bands", "Rings"];
    private static readonly IReadOnlyList<string> GradientKinds = ["Linear", "Quadratic", "Spherical", "Radial"];

    /// <summary>The node kinds of material graphs.</summary>
    public static readonly Catalog Catalog = new()
    {
        Domain = "material",
        Label = "Material",
        Extension = "bmaterial",
        Starter = [("output", new Vector2(360f, 40f))],
        // Materials have no whole numbers; the list stays for the rule's sake.
        Fractional = [],
        Nodes =
        [
            new(
                "output",
                "Material Output",
                "Output",
                "What the surface looks like, lit by the engine. A material has one output: this or Custom Lighting Output.",
                [
                    Colour("baseColor", "Base Color", 0.8f, 0.8f, 0.8f),
                    Number("metallic", "Metallic", F, 0f),
                    Number("roughness", "Roughness", F, 0.5f),
                    Colour("emissive", "Emission", 0f, 0f, 0f),
                    Number("opacity", "Opacity", F, 1f),
                ],
                [],
                OutlineSettings,
                true),
            new(
                "customLighting",
                "Custom Lighting Output",
                "Output",
                "The colour the surface shows, worked out by the graph itself (with Main Light and Other Lights) instead of the engine's lighting.",
                [Colour("color", "Color", 0.8f, 0.8f, 0.8f), Number("alpha", "Opacity", F, 1f)],
                [],
                OutlineSettings,
                true),
            Node(
                "mainLight",
                "Main Light",
                "Lights",
                "The main light at this point (the sun when there is one): the direction towards it, its colour and strength, and how much of it the shadows let through (1 lit).",
                [],
                [Pin("direction", "Direction", V), Pin("color", "Color", C), Pin("shadow", "Shadow", F)]),
            Node(
                "otherLights",
                "Other Lights",
                "Lights",
                "The light every other light gives this point, added up (facing them, past shadows).",
                [],
                [Pin("color", "Color", C)]),
            Node("viewDirection", "View Direction", "Inputs", "From the point towards the camera.", [], [Pin("direction", "Direction", V)]),
            Node(
                "screenPosition",
                "Screen Position",
                "Inputs",
                "Where the pixel is on the screen, 0 to 1 from the top left, as a vector (x, y, 0).",
                [],
                [Pin("position", "Position", V)]),
            Node(
                "dot",
                "Dot Product",
                "Math",
                "How much two directions agree: 1 the same, 0 at right angles, −1 opposite (for unit vectors).",
                [Pin("a", "A", V), Pin("b", "B", V)],
                [Pin("result", "Result", F)]),
            Node("normalize", "Normalize", "Math", "The same direction with length 1.", [Pin("vector", "Vector", V)], [Pin("vector", "Vector", V)]),
            Node(
                "step",
                "Step",
                "Math",
                "0 below the edge, 1 from the edge up: a hard band.",
                [Number("edge", "Edge", F, 0.5f), Number("value", "Value", A, 0f)],
                MathOut),
            Node(
                "smoothstep",
                "Smooth Step",
                "Math",
                "0 below Low, 1 above High, a smooth curve between: a soft band.",
                [Number("low", "Low", F, 0f), Number("high", "High", F, 1f), Number("value", "Value", A, 0.5f)],
                MathOut),
            Node(
                "posterize",
                "Posterize",
                "Math",
                "Cuts values from 0 to 1 into a number of flat bands (2 or more).",
                [Number("value", "Value", A, 0f), Number("steps", "Bands", F, 3f)],
                MathOut),
            Node(
                "remap",
                "Remap",
                "Math",
                "Moves a value from one range to another: From Low becomes To Low and From High To High.",
                [
                    Number("value", "Value", A, 0.5f),
                    Number("fromLow", "From Low", F, 0f),
                    Number("fromHigh", "From High", F, 1f),
                    Number("toLow", "To Low", F, 0f),
                    Number("toHigh", "To High", F, 1f),
                ],
                MathOut),
            Node("number", "Number", "Values", "A fixed number.", [], [Pin("value", "Value", F)], [new("value", "Value", new SettingKind.Number(1f))]),
            Node("color", "Color", "Values", "A fixed colour.", [], [Pin("color", "Color", C)], [new("color", "Color", new SettingKind.Color(Vector3.One))]),
            Node(
                "vector",
                "Make Vector",
                "Values",
                "A vector from three numbers.",
                [Number("x", "X", F, 0f), Number("y", "Y", F, 0f), Number("z", "Z", F, 0f)],
                [Pin("vector", "Vector", V)]),
            Node(
                "split",
                "Split Vector",
                "Values",
                "The three numbers of a vector or colour.",
                [Pin("vector", "Vector", V)],
                [Pin("x", "X", F), Pin("y", "Y", F), Pin("z", "Z", F)]),
            Node(
                "objectColor",
                "Object Color",
                "Inputs",
                "The colour set on the object's Mesh component.",
                [],
                [Pin("color", "Color", C), Pin("alpha", "Alpha", F)]),
            Node("time", "Time", "Inputs", "Seconds since the view started; animates the material.", [], [Pin("seconds", "Seconds", F)]),
            Node("position", "World Position", "Inputs", "Where the pixel is in the world.", [], [Pin("position", "Position", V)]),
            Node(
                "uv",
                "Texture Coordinates",
                "Inputs",
                "Where on the shape's surface the pixel is: 0 to 1 across each face, as a vector (u, v, 0).",
                [],
                [Pin("uv", "Coordinates", V)]),
            Node(
                "texture",
                "Image",
                "Inputs",
                "Colours from a PNG image, laid over the surface by its texture coordinates.",
                [Implicit("uv", "Coordinates", "uv")],
                [Pin("color", "Color", C), Pin("alpha", "Alpha", F)],
                [new("image", "Image File", new SettingKind.Text(""))]),
            Node("normal", "Normal", "Inputs", "Which way the surface faces, in world space.", [], [Pin("normal", "Normal", V)]),
            Node(
                "fresnel",
                "Edge Glow",
                "Inputs",
                "1 where the surface turns away from the viewer, 0 where it faces them.",
                [Number("power", "Sharpness", F, 5f)],
                [Pin("factor", "Factor", F)]),
            Node("add", "Add", "Math", "A + B.", [Number("a", "A", A, 0f), Number("b", "B", A, 0f)], MathOut),
            Node("subtract", "Subtract", "Math", "A − B.", [Number("a", "A", A, 0f), Number("b", "B", A, 0f)], MathOut),
            Node("multiply", "Multiply", "Math", "A × B.", [Number("a", "A", A, 1f), Number("b", "B", A, 1f)], MathOut),
            Node("divide", "Divide", "Math", "A ÷ B.", [Number("a", "A", A, 1f), Number("b", "B", A, 1f)], MathOut),
            Node(
                "blend",
                "Lerp",
                "Math",
                "Linear interpolation (a blend): from A (factor 0) to B (factor 1) on a straight line; factors outside 0 to 1 go past A or B.",
                [Number("a", "A", A, 0f), Number("b", "B", A, 1f), Number("factor", "Factor", F, 0.5f)],
                MathOut),
            Node(
                "inverseLerp",
                "Inverse Lerp",
                "Math",
                "Where the value lies from A to B: 0 at A, 1 at B, 0.5 halfway, and past 0 or 1 beyond them; 0 when A and B are equal.",
                [Number("a", "A", F, 0f), Number("b", "B", F, 1f), Number("value", "Value", A, 0.5f)],
                MathOut),
            Node(
                "power",
                "Power",
                "Math",
                "Base to the power of the exponent.",
                [Number("base", "Base", A, 1f), Number("exponent", "Exponent", F, 2f)],
                MathOut),
            Node(
                "squareRoot",
                "Square Root",
                "Math",
                "The number that times itself gives the input (0 below 0); for a vector, of each part.",
                [Number("value", "Value", A, 1f)],
                MathOut),
            Node(
                "vectorLength",
                "Vector Length",
                "Math",
                "How long a vector is: the distance from 0 to the point it names.",
                [Pin("vector", "Vector", V)],
                [Pin("length", "Length", F)]),
            Node("oneMinus", "One Minus", "Math", "1 − the input.", [Number("value", "Value", A, 0f)], MathOut),
            Node("clamp", "Clamp 0 to 1", "Math", "Keeps the input between 0 and 1.", [Number("value", "Value", A, 0f)], MathOut),
            Node("sine", "Sine", "Math", "The sine of the input, in radians.", [Number("value", "Value", A, 0f)], MathOut),
            Node(
                "checker",
                "Checker",
                "Patterns",
                "Alternating 0 and 1 cubes in space.",
                [Implicit("position", "Position", "position"), Number("scale", "Cells per Unit", F, 1f)],
                [Pin("value", "Value", F)]),
            Node(
                "noise",
                "Noise",
                "Patterns",
                "Smooth random values between 0 and 1.",
                [Implicit("position", "Position", "position"), Number("scale", "Scale", F, 1f)],
                [Pin("value", "Value", F)]),
            Node(
                "fractalNoise",
                "Fractal Noise",
                "Patterns",
                "Noise in layers, each finer and fainter than the one before: Smooth for clouds and terrain, Ridged for sharp crests, Billowy for puffy shapes. Detail is how many layers (1 to 8), Roughness how strong each finer one is, Lacunarity how much finer.",
                [
                    Implicit("position", "Position", "position"),
                    Number("scale", "Scale", F, 1f),
                    Number("detail", "Detail", F, 4f),
                    Number("roughness", "Roughness", F, 0.5f),
                    Number("lacunarity", "Lacunarity", F, 2f),
                ],
                [Pin("value", "Value", F)],
                [new("kind", "Kind", new SettingKind.Choice(FractalKinds))]),
            Node(
                "voronoi",
                "Voronoi",
                "Patterns",
                "Cells round scattered points: Distance to the nearest point (0 at it), Edge (0 on a border between cells), and a random value 0 to 1 for each cell. Randomness 0 lines the points up in a grid.",
                [
                    Implicit("position", "Position", "position"),
                    Number("scale", "Scale", F, 1f),
                    Number("randomness", "Randomness", F, 1f),
                ],
                [Pin("distance", "Distance", F), Pin("edge", "Edge", F), Pin("cell", "Cell Value", F)]),
            Node(
                "waves",
                "Waves",
                "Patterns",
                "Soft stripes from 0 to 1: Bands along X, or Rings round the centre. Distortion bends them with noise.",
                [
                    Implicit("position", "Position", "position"),
                    Number("scale", "Scale", F, 1f),
                    Number("distortion", "Distortion", F, 0f),
                ],
                [Pin("value", "Value", F)],
                [new("kind", "Kind", new SettingKind.Choice(WaveKinds))]),
            Node(
                "gradient",
                "Gradient",
                "Patterns",
                "0 to 1 across the coordinates: Linear along X, Quadratic (eased), Spherical (1 at the middle, 0 half a unit away) or Radial (round the middle); the middle is X and Y 0.5, the centre of texture coordinates.",
                [Implicit("uv", "Coordinates", "uv")],
                [Pin("value", "Value", F)],
                [new("kind", "Kind", new SettingKind.Choice(GradientKinds))]),
            Node(
                "random",
                "Random",
                "Patterns",
                "A different random value 0 to 1 for every point (white noise), and a random colour; Scale sets how fine.",
                [Implicit("position", "Position", "position"), Number("scale", "Scale", F, 100f)],
                [Pin("value", "Value", F), Pin("color", "Color", C)]),
        ],
    };
}

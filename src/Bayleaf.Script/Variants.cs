using System.Collections.Concurrent;
using System.Numerics;
using System.Text;

using Bayleaf.Core.Json;
using Bayleaf.Graph;

namespace Bayleaf.Script;

/// <summary>One component's field pins, as Get Component, Try Get Component and Set Component show them.</summary>
/// <param name="Name">The component's name in scene files (<c>pointLight</c>).</param>
/// <param name="Label">Its name in the editor (<c>Point Light</c>).</param>
public sealed record ComponentNodes(string Name, string Label, NodeDef Get, NodeDef TryGet, NodeDef Set)
{
    /// <summary>
    /// Whether Set Component can change it while the game plays. Physics reads Rigid Body, Collider
    /// and Character once, when play starts, so those are read only.
    /// </summary>
    public bool Settable => Name is not ("rigidBody" or "collider" or "characterBody");

    /// <summary>Its fields, as Get Component shows them.</summary>
    public IReadOnlyList<PinDef> Fields => Get.Outputs;

    private static ComponentNodes Of(string name, string label, params (string Field, string Label, PinType Type)[] fields)
    {
        var pins = fields.Select(f => new PinDef(f.Field, f.Label, f.Type)).ToList();
        var settings = new[] { Visual.ComponentSetting, Visual.FieldsSetting };
        return new(
            name,
            label,
            new("getComponent", "Get Component", "Find", Visual.GetText, [Visual.Target], pins, settings, false),
            new("tryGetComponent", "Try Get Component", "Find", Visual.TryGetText, [Visual.Target], [new PinDef("found", "Found", PinType.Bool), .. pins], settings, false),
            new("setComponent", "Set Component", "Actions", Visual.SetText, [Visual.Run, Visual.Target, .. fields.Select(f => new PinDef(f.Field, f.Label, f.Type, new PinDefault.Implicit("unchanged")))], [Visual.Then], settings, false));
    }

    private static readonly PinType F = PinType.Number;
    private static readonly PinType V = PinType.Vec3;
    private static readonly PinType C = PinType.Color;
    private static readonly PinType B = PinType.Bool;

    /// <summary>Every component whose fields visual scripts can read and set. A field's pin name is its name in scene files.</summary>
    public static readonly IReadOnlyList<ComponentNodes> All =
    [
        Of("transform", "Transform", ("position", "Position", V), ("rotation", "Rotation", V), ("scale", "Scale", V)),
        Of("mesh", "Mesh", ("color", "Color", C), ("metallic", "Metallic", F), ("roughness", "Roughness", F)),
        Of("sprite", "Sprite", ("color", "Color", C)),
        Of("camera", "Camera", ("fov", "Field of View", F), ("size", "Size", F), ("near", "Near", F), ("far", "Far", F)),
        Of("directionalLight", "Directional Light", ("color", "Color", C), ("intensity", "Intensity", F), ("shadows", "Shadows", B)),
        Of("pointLight", "Point Light", ("color", "Color", C), ("intensity", "Intensity", F), ("range", "Range", F), ("shadows", "Shadows", B)),
        Of(
            "spotLight",
            "Spot Light",
            ("color", "Color", C),
            ("intensity", "Intensity", F),
            ("range", "Range", F),
            ("angle", "Angle", F),
            ("softness", "Softness", F),
            ("shadows", "Shadows", B)),
        Of(
            "environment",
            "Environment",
            ("skyColor", "Sky Color", C),
            ("horizonColor", "Horizon Color", C),
            ("groundColor", "Ground Color", C),
            ("ambient", "Ambient", F),
            ("fogDistance", "Fog Distance", F),
            ("showSky", "Show Sky", B)),
        Of("rigidBody", "Rigid Body", ("mass", "Mass", F), ("gravityScale", "Gravity Scale", F), ("lockRotation", "Lock Rotation", B)),
        Of("collider", "Collider", ("size", "Size", V), ("friction", "Friction", F), ("restitution", "Restitution", F), ("trigger", "Trigger", B)),
        Of("characterBody", "Character", ("height", "Height", F), ("radius", "Radius", F), ("maxSlope", "Max Slope", F), ("stepHeight", "Step Height", F)),
        Of(
            "playerControl",
            "Player Control",
            ("speed", "Speed", F),
            ("jumpSpeed", "Jump Speed", F),
            ("turnToMove", "Turn to Move", B),
            ("cameraDistance", "Camera Distance", F),
            ("cameraHeight", "Camera Height", F)),
        Of(
            "joint",
            "Joint",
            ("motorSpeed", "Motor Speed", F),
            ("motorForce", "Motor Force", F),
            ("limited", "Limited", B),
            ("lower", "Lower", F),
            ("upper", "Upper", F)),
        Of("gameRules", "Game Rules", ("respawn", "Respawn", B), ("respawnHeight", "Respawn Height", F), ("timeLimit", "Time Limit", F)),
    ];
}

/// <summary>The pins of nodes whose pins depend on their settings.</summary>
public static partial class Visual
{
    /// <summary>The node kinds that name a function in their <c>function</c> setting.</summary>
    public static readonly IReadOnlyList<string> FunctionKinds = ["functionStart", "functionReturn", "callFunction"];

    /// <summary>The node kinds that name an event in their <c>event</c> setting.</summary>
    public static readonly IReadOnlyList<string> EventKinds = ["eventStart", "triggerEvent"];

    /// <summary>The node kinds that name another script's event, with values: triggering it, and hearing it.</summary>
    public static readonly IReadOnlyList<string> ComponentEventKinds = ["triggerComponentEvent", "componentEventStart"];

    /// <summary>Every node kind that names another script's event (subscribing too).</summary>
    public static readonly IReadOnlyList<string> ScriptEventKinds = ["triggerComponentEvent", "componentEventStart", "subscribeEvent", "unsubscribeEvent"];

    /// <summary>The node kinds that work on a declared list.</summary>
    public static readonly IReadOnlyList<string> ListKinds = ["getList", "listAdd", "listInsert", "listSet", "listRemove", "listClear", "listItem", "listLength", "listFind", "listForEach"];

    /// <summary>The node kinds that work on a declared map.</summary>
    public static readonly IReadOnlyList<string> MapKinds = ["getMap", "mapSet", "mapRemove", "mapClear", "mapGet", "mapCount", "mapForEach"];

    /// <summary>Every node kind that names a declared list or map in its <c>name</c> setting.</summary>
    public static readonly IReadOnlyList<string> CollectionKinds =
        ["getList", "getMap", "listAdd", "listInsert", "listSet", "listRemove", "listClear", "listItem", "listLength", "listFind", "listForEach", "mapSet", "mapRemove", "mapClear", "mapGet", "mapCount", "mapForEach"];

    /// <summary>A script's Get Component also gives the script itself, as a component.</summary>
    private static readonly PinDef ComponentOut = new("component", "Component", PinType.Component);

    /// <summary>Most variants kept; past it, variants are made again each time they are asked for.</summary>
    private const int MaxKeptVariants = 4096;

    /// <summary>Variants made so far, by node kind and the settings that shape them.</summary>
    private static readonly ConcurrentDictionary<string, NodeDef> Made = new(StringComparer.Ordinal);

    /// <summary>
    /// The pins of a node whose pins depend on its settings: a Get Component, Try Get Component or Set
    /// Component node (its component's fields, or the properties of the script it names), a Data File
    /// node (its shape's fields), a Function, Return or Call Function node (its function's inputs and
    /// outputs), an event node (the event's values) or a list or map node (what it holds). Null for the
    /// kind's own pins.
    /// </summary>
    public static NodeDef? Variant(Node node)
    {
        if (CollectionKinds.Contains(node.Kind))
        {
            return CollectionVariant(node);
        }
        if (FunctionKinds.Contains(node.Kind) || EventKinds.Contains(node.Kind) || ComponentEventKinds.Contains(node.Kind))
        {
            return FunctionVariant(node);
        }
        if (node.Kind == "dataFile")
        {
            return DataVariant(node);
        }
        if (node.Kind is not ("getComponent" or "tryGetComponent" or "setComponent"))
        {
            return null;
        }
        if (ComponentOf(node) is not { } component)
        {
            return ScriptFields(node) is { } fields ? ScriptVariant(node, fields) : null;
        }
        return node.Kind switch
        {
            "getComponent" => component.Get,
            "tryGetComponent" => component.TryGet,
            _ => component.Set,
        };
    }

    /// <summary>A variant from the cache, made and kept when it is new.</summary>
    private static NodeDef? Kept(string key, Func<NodeDef?> make)
    {
        if (Made.TryGetValue(key, out var def))
        {
            return def;
        }
        var made = make();
        if (made is not null && Made.Count < MaxKeptVariants)
        {
            Made.TryAdd(key, made);
        }
        return made;
    }

    private static string CacheKey(params IEnumerable<string>[] parts) => string.Join("\u0001", parts.Select(p => string.Join("\u0000", p)));

    /// <summary>The names a node keeps in its <paramref name="setting"/>: its texts, in order.</summary>
    private static List<string> Names(Node node, string setting) =>
        node.Values.GetValueOrDefault(setting) is JsonArray items ? items.OfType<JsonString>().Select(s => s.Value).ToList() : [];

    /// <summary>
    /// The component a node's Component setting names (by its name or label; case, spaces, <c>_</c> and
    /// <c>-</c> do not matter), if visual scripts can read it.
    /// </summary>
    public static ComponentNodes? ComponentOf(Node node)
    {
        var named = node.Values.GetValueOrDefault("component") is JsonString s ? s.Value : "Point Light";
        var key = KindKey(named);
        return ComponentNodes.All.FirstOrDefault(c => KindKey(c.Name) == key || KindKey(c.Label) == key);
    }

    /// <summary>The property names a node keeps for the script it names, if it names a script.</summary>
    public static IReadOnlyList<string>? ScriptFields(Node node) =>
        ComponentOf(node) is null && node.Values.GetValueOrDefault("fields") is JsonArray ? Names(node, "fields") : null;

    /// <summary>A name for searches: without spaces, <c>_</c> and <c>-</c>, in lower case.</summary>
    public static string KindKey(string text)
    {
        var key = new StringBuilder();
        foreach (var rune in text.EnumerateRunes())
        {
            if (rune.Value is not (' ' or '_' or '-'))
            {
                key.Append(Rune.ToLowerInvariant(rune).ToString());
            }
        }
        return key.ToString();
    }

    /// <summary>A name as a label: <c>jumpHeight</c> and <c>jump_height</c> become <c>Jump Height</c>.</summary>
    public static string Title(string name)
    {
        var text = new StringBuilder();
        var upper = true;
        foreach (var rune in name.EnumerateRunes())
        {
            if (rune.Value is '_' or ' ')
            {
                upper = true;
                continue;
            }
            if ((upper || Rune.IsUpper(rune)) && text.Length > 0)
            {
                text.Append(' ');
            }
            text.Append((upper ? Rune.ToUpperInvariant(rune) : rune).ToString());
            upper = false;
        }
        return text.ToString();
    }

    /// <summary>
    /// The pins of a node that names a script: one pin per property, typed as <c>name:type</c> says
    /// (<c>speed</c>, a number, or <c>count:whole</c>, <c>on:bool</c>, <c>offset:vector</c>, <c>tint:color</c>).
    /// Text and object properties have no pin: fields are read and set as numbers.
    /// </summary>
    private static NodeDef? ScriptVariant(Node node, IReadOnlyList<string> fields) => Kept(CacheKey([node.Kind], fields), () =>
    {
        List<PinDef> Pins(PinDefault value)
        {
            var pins = new List<PinDef>();
            foreach (var field in fields)
            {
                var colon = field.IndexOf(':', StringComparison.Ordinal);
                var (name, code) = colon < 0 ? (field, "number") : (field[..colon], field[(colon + 1)..]);
                PinType? type = code switch
                {
                    "number" => PinType.Number,
                    "whole" => PinType.WholeNumber,
                    "bool" => PinType.Bool,
                    "vector" => PinType.Vec3,
                    "color" => PinType.Color,
                    _ => null,
                };
                if (type is { } t)
                {
                    pins.Add(new(name, Title(name), t, value));
                }
            }
            return pins;
        }
        var settings = new[] { ComponentSetting, FieldsSetting };
        return node.Kind switch
        {
            "getComponent" => new NodeDef("getComponent", "Get Component", "Find", GetText, [Target], [ComponentOut, .. Pins(PinDefault.None)], settings, false),
            "tryGetComponent" => new NodeDef(
                "tryGetComponent",
                "Try Get Component",
                "Find",
                TryGetText,
                [Target],
                [new PinDef("found", "Found", PinType.Bool), ComponentOut, .. Pins(PinDefault.None)],
                settings,
                false),
            "setComponent" => new NodeDef("setComponent", "Set Component", "Actions", SetText, [Run, Target, .. Pins(new PinDefault.Implicit("unchanged"))], [Then], settings, false),
            _ => null,
        };
    });

    /// <summary>The pin type a data field of type <paramref name="code"/> reads as; null for a list or map (no pin).</summary>
    internal static PinType? DataPinType(string code) => code switch
    {
        "number" => PinType.Number,
        "whole" => PinType.WholeNumber,
        "bool" => PinType.Bool,
        "vector" => PinType.Vec3,
        "color" => PinType.Color,
        "object" => PinType.SceneObject,
        "text" or "prefab" or "sound" or "material" or "data" => PinType.Text,
        _ => null,
    };

    /// <summary>
    /// The pins of a Data File node: one output per field of its shape, as the node keeps them in its
    /// <c>fields</c> setting (<c>name:type</c>; the editor keeps it in step with the shape).
    /// </summary>
    private static NodeDef? DataVariant(Node node)
    {
        var fields = Names(node, "fields");
        return Kept(CacheKey(["dataFile"], fields), () =>
        {
            var outputs = new List<PinDef>();
            foreach (var entry in fields)
            {
                var colon = entry.IndexOf(':', StringComparison.Ordinal);
                if (colon >= 0 && DataPinType(entry[(colon + 1)..]) is { } type)
                {
                    var name = entry[..colon];
                    outputs.Add(new(name, Title(name), type));
                }
            }
            return new NodeDef("dataFile", "Data File", "Values", DataText, [DataFile], outputs, [DataShape, FieldsSetting], false);
        });
    }

    /// <summary>The pins a list of <c>name:type</c> entries becomes; an entry of an unknown type is left out.</summary>
    private static List<PinDef> TypedPins(IEnumerable<string> entries, bool withDefaults)
    {
        var pins = new List<PinDef>();
        foreach (var entry in entries)
        {
            var colon = entry.IndexOf(':', StringComparison.Ordinal);
            if (colon < 0)
            {
                continue;
            }
            var (name, code) = (entry[..colon], entry[(colon + 1)..]);
            // A list or map: no default, it comes only from a link.
            if (VariableTypes.FromShapedCode(code) is { } shaped && shaped.Shape != Shape.One)
            {
                pins.Add(new(name, Title(name), shaped.Type.Pin(shaped.Shape, shaped.Key)));
                continue;
            }
            (PinType Type, PinDefault Default)? typed = code switch
            {
                "number" => (PinType.Number, new PinDefault.Number(0f)),
                "whole" => (PinType.WholeNumber, new PinDefault.WholeNumber(0)),
                "bool" => (PinType.Bool, new PinDefault.Bool(false)),
                "vector" => (PinType.Vec3, new PinDefault.Vec3(Vector3.Zero)),
                "color" => (PinType.Color, new PinDefault.Color(Vector3.One)),
                "text" => (PinType.Text, new PinDefault.Text("")),
                "object" => (PinType.SceneObject, PinDefault.None),
                _ => null,
            };
            if (typed is { } t)
            {
                pins.Add(new(name, Title(name), t.Type, withDefaults ? t.Default : PinDefault.None));
            }
        }
        return pins;
    }

    /// <summary>
    /// The pins of a Function, Return or Call Function node (the function's inputs and outputs as the
    /// node keeps them, <c>inputs</c> and <c>outputs</c>, each <c>name:type</c>), or of an event node
    /// (its values, <c>params</c>: the inputs of a Trigger Event, the outputs of an On Event).
    /// </summary>
    private static NodeDef? FunctionVariant(Node node)
    {
        var events = EventKinds.Contains(node.Kind) || ComponentEventKinds.Contains(node.Kind);
        var inputs = events ? Names(node, "params") : Names(node, "inputs");
        var outputs = events ? [] : Names(node, "outputs");
        return Kept(CacheKey([node.Kind], inputs, ["\u0002"], outputs), () =>
        {
            if (Catalog.Get(node.Kind) is not { } kind)
            {
                return null;
            }
            return node.Kind switch
            {
                "functionStart" or "eventStart" => kind with { Outputs = [Then, .. TypedPins(inputs, false)] },
                "functionReturn" => kind with { Inputs = [Run, .. TypedPins(outputs, true)] },
                "callFunction" => kind with { Inputs = [Run, .. TypedPins(inputs, true)], Outputs = [Then, .. TypedPins(outputs, false)] },
                "triggerEvent" => kind with { Inputs = [Run, Target, .. TypedPins(inputs, true)] },
                "triggerComponentEvent" => kind with { Inputs = [Run, ComponentIn, .. TypedPins(inputs, true)] },
                "componentEventStart" => kind with { Outputs = [Then, new PinDef("source", "Source", PinType.Component), .. TypedPins(inputs, false)] },
                _ => null,
            };
        });
    }

    /// <summary>
    /// What a list or map node holds, from its <c>item</c> setting (the type codes of the values, then of
    /// a map's keys; kept in step with the declaration by <c>graph.variable.set</c> and <c>graph.property.set</c>).
    /// </summary>
    private static VariableType? ItemOf(Node node) =>
        node.Values.GetValueOrDefault("item") is JsonArray { Count: > 0 } items && items[0] is JsonString code ? VariableTypes.FromCode(code.Value) : null;

    /// <summary>The type of a map node's keys, from its <c>item</c> setting; text when it says none.</summary>
    private static VariableType KeyTypeOf(Node node) =>
        node.Values.GetValueOrDefault("item") is JsonArray { Count: > 1 } items && items[1] is JsonString code && VariableTypes.FromCode(code.Value) is { } key ? key : VariableType.Text;

    /// <summary>A pin of one value of <paramref name="type"/>, with its type's default when it is an input.</summary>
    private static PinDef TypedPin(string name, string label, VariableType type, bool input)
    {
        var (pin, value) = type switch
        {
            VariableType.Number => (PinType.Number, (PinDefault)new PinDefault.Number(0f)),
            VariableType.WholeNumber => (PinType.WholeNumber, new PinDefault.WholeNumber(0)),
            VariableType.TrueFalse => (PinType.Bool, new PinDefault.Bool(false)),
            VariableType.Vector => (PinType.Vec3, new PinDefault.Vec3(Vector3.Zero)),
            VariableType.Color => (PinType.Color, new PinDefault.Color(Vector3.One)),
            VariableType.Text => (PinType.Text, new PinDefault.Text("")),
            _ => (PinType.SceneObject, PinDefault.None),
        };
        return new(name, label, pin, input ? value : PinDefault.None);
    }

    /// <summary>Whether a node kind works on a map (else on a list).</summary>
    private static bool OnMap(string kind) => kind.StartsWith("map", StringComparison.Ordinal) || kind == "getMap";

    /// <summary>
    /// The pins of a list or map node of the type it holds and, a map's, of the type of its keys (its
    /// <c>item</c>): the catalog's are for numbers under text keys, and here the list or map pin, the
    /// Value and the Key take the types it names.
    /// </summary>
    private static NodeDef? CollectionVariant(Node node)
    {
        var type = ItemOf(node) ?? VariableType.Number;
        var key = KeyTypeOf(node);
        if (Catalog.Get(node.Kind) is not { } kind)
        {
            return null;
        }
        if (type == VariableType.Number && key == VariableType.Text)
        {
            return kind;
        }
        return Kept(CacheKey([node.Kind, type.Code(), key.Code()]), () =>
        {
            List<PinDef> Retyped(IReadOnlyList<PinDef> pins, bool input) => pins.Select(p => p switch
            {
                { Name: "list", Type.Kind: PinKind.List } => p with { Type = PinType.List(type) },
                { Name: "map", Type.Kind: PinKind.Map } => p with { Type = PinType.Map(key, type) },
                { Name: "value" } => TypedPin("value", "Value", type, input),
                { Name: "key" } when OnMap(node.Kind) => TypedPin("key", "Key", key, input),
                _ => p,
            }).ToList();
            return kind with { Inputs = Retyped(kind.Inputs, true), Outputs = Retyped(kind.Outputs, false) };
        });
    }

    /// <summary>
    /// What a list or map node should hold for <paramref name="link"/> to fit: linking a list of text
    /// into a list node makes it a node for lists of text, and a map's keys' type comes along too. The
    /// node, the value type and the key type; null when the link goes elsewhere or fits already.
    /// </summary>
    public static (uint Node, VariableType Item, VariableType Key)? AdoptedItem(Graph.Graph graph, Link link)
    {
        if (graph.FindNode(link.To.Node) is not { } to || !CollectionKinds.Contains(to.Kind) || link.To.Pin is not ("list" or "map"))
        {
            return null;
        }
        if (!graph.NodeWithDef(Catalog, link.From.Node).TryGet(out var from, out _) || from.Def.Output(link.From.Pin) is not { } output)
        {
            return null;
        }
        (VariableType Item, VariableType Key) wanted;
        if (output.Type.Kind == PinKind.List && link.To.Pin == "list")
        {
            wanted = (output.Type.Item, VariableType.Text);
        }
        else if (output.Type.Kind == PinKind.Map && link.To.Pin == "map")
        {
            wanted = (output.Type.Item, output.Type.Key);
        }
        else
        {
            return null;
        }
        var now = (ItemOf(to) ?? VariableType.Number, link.To.Pin == "map" ? KeyTypeOf(to) : VariableType.Text);
        return now != wanted ? (to.Id, wanted.Item, wanted.Key) : null;
    }
}

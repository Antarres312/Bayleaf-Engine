using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Graph;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using Shape = Bayleaf.Graph.Shape;

namespace Bayleaf.Script;

/// <summary>Variables and properties of visual scripts: the nodes that read and set them, and how the scene and the Inspector call them.</summary>
public static partial class Visual
{
    /// <summary>The node kinds that read and set a variable of <paramref name="type"/>, by their <c>name</c> setting: the Get node first.</summary>
    public static (string Get, string Set) VariableKinds(VariableType type) => type switch
    {
        VariableType.Number => ("variable", "setVariable"),
        VariableType.Vector => ("vectorVariable", "setVectorVariable"),
        VariableType.TrueFalse => ("flag", "setFlag"),
        VariableType.WholeNumber => ("wholeVariable", "setWholeVariable"),
        VariableType.Text => ("textVariable", "setTextVariable"),
        VariableType.SceneObject => ("objectVariable", "setObjectVariable"),
        _ => ("colorVariable", "setColorVariable"),
    };

    /// <summary>The Property and Set Property node kinds for each property type.</summary>
    public static (string Get, string Set) PropertyKinds(VariableType type) => type switch
    {
        VariableType.Number => ("property", "setProperty"),
        VariableType.WholeNumber => ("wholeProperty", "setWholeProperty"),
        VariableType.TrueFalse => ("flagProperty", "setFlagProperty"),
        VariableType.Vector => ("vectorProperty", "setVectorProperty"),
        VariableType.Color => ("colorProperty", "setColorProperty"),
        VariableType.Text => ("textProperty", "setTextProperty"),
        _ => ("objectProperty", "setObjectProperty"),
    };

    /// <summary>The property type a Property or Set Property node kind reads or sets.</summary>
    public static VariableType? PropertyTypeOf(string kind)
    {
        foreach (var type in VariableTypes.All)
        {
            var (get, set) = PropertyKinds(type);
            if (kind == get || kind == set)
            {
                return type;
            }
        }
        return null;
    }

    /// <summary>How the scene and the Inspector call a property of this type.</summary>
    public static PropertyType ScenePropertyType(VariableType type) => type switch
    {
        VariableType.Number => PropertyType.Number,
        VariableType.WholeNumber => PropertyType.WholeNumber,
        VariableType.TrueFalse => PropertyType.TrueFalse,
        VariableType.Vector => PropertyType.Vector,
        VariableType.Color => PropertyType.Color,
        VariableType.Text => PropertyType.Text,
        _ => PropertyType.SceneObject,
    };

    /// <summary>How the scene and the Inspector call a shape.</summary>
    public static PropertyShape ScenePropertyShape(Shape shape, VariableType key) => shape switch
    {
        Shape.One => PropertyShape.One,
        Shape.List => PropertyShape.List,
        _ => PropertyShape.Map(ScenePropertyType(key)),
    };

    /// <summary>A declared default as a script's description lists it: an object property's none is <c>""</c>, a colour stays within 0 to 1.</summary>
    private static JsonValue DescribedDefault(VariableType type, JsonValue value)
    {
        switch (type)
        {
            case VariableType.SceneObject:
                return new JsonString("");
            case VariableType.Color:
            {
                var parts = new JsonValue[3];
                for (var i = 0; i < 3; i++)
                {
                    var part = value is JsonArray a && i < a.Count && Graph.Graph.AsF32(a[i]) is { } f ? f : 1f;
                    parts[i] = JsonNumber.Of((double)Math.Clamp(part, 0f, 1f));
                }
                return new JsonArray(parts);
            }
            default:
                return value;
        }
    }

    /// <summary>A declared default or start in <paramref name="shape"/> as a description lists it: one value, or a list or map of them.</summary>
    private static JsonValue DescribedValue(VariableType type, Shape shape, JsonValue value) => shape switch
    {
        Shape.One => DescribedDefault(type, value),
        Shape.List => value is JsonArray items ? new JsonArray(items.Select(v => DescribedDefault(type, v))) : new JsonArray(),
        _ => value is JsonObject entries ? Entries(entries.Select(e => (e.Key, DescribedDefault(type, e.Value)))) : new JsonObject(),
    };

    private static JsonObject Entries(IEnumerable<(string Key, JsonValue Value)> entries)
    {
        var o = new JsonObject();
        foreach (var (key, value) in entries)
        {
            o.Add(key, value);
        }
        return o;
    }

    /// <summary>What a Property node's name was before files said otherwise, by kind (graphs before version 5).</summary>
    private static string OldPropertyName(string kind) => kind switch
    {
        "wholeProperty" => "count",
        "flagProperty" => "enabled",
        "vectorProperty" => "offset",
        "colorProperty" => "tint",
        "textProperty" => "label",
        "objectProperty" => "target",
        _ => "speed",
    };

    /// <summary>
    /// Moves what visual scripts before version 5 kept on their Property nodes (each one's name and
    /// default) into the script's declared properties; the first node of a name gives its default.
    /// </summary>
    public static void Upgrade(Graph.Graph graph, uint read)
    {
        if (read >= 5)
        {
            return;
        }
        var declared = new List<(string Name, VariableType Type, JsonValue Default)>();
        foreach (var node in graph.Nodes)
        {
            if (PropertyTypeOf(node.Kind) is not { } type)
            {
                continue;
            }
            var name = node.Values.GetValueOrDefault("name") is JsonString s ? s.Value : OldPropertyName(node.Kind);
            node.Values["name"] = new JsonString(name);
            var old = node.Values.Remove("default", out var kept) ? kept : null;
            JsonValue value = (type, old) switch
            {
                (VariableType.Number, { } v) => JsonNumber.Of((double)(Graph.Graph.AsF32(v) ?? 1f)),
                (VariableType.Number, null) => JsonNumber.Of(1.0),
                (VariableType.WholeNumber, var v) => JsonNumber.Of((long)RoundToWhole(v is null ? null : Graph.Graph.AsF32(v))),
                (VariableType.TrueFalse, var v) => JsonValue.From(v is JsonString { Value: "True" }),
                (VariableType.Text, var v) => new JsonString(v is JsonString t ? t.Value : ""),
                (VariableType.SceneObject, _) => JsonValue.Null,
                (VariableType.Vector or VariableType.Color, { } v) when type.Fits(v) => v,
                _ => type.Zero(),
            };
            if (!declared.Any(d => d.Name == name))
            {
                declared.Add((name, type, value));
            }
        }
        foreach (var (name, type, value) in declared)
        {
            if (graph.FindProperty(name) is null && type.Fits(value))
            {
                // A name the graph refuses stays on its node, which the graph's check then refuses.
                _ = graph.AddProperty(name, type, value);
            }
        }
    }

    /// <summary>A number made whole as the Rust engine's <c>round() as i32</c> does: halves away from zero, stopping at the 32-bit ends.</summary>
    private static int RoundToWhole(float? value)
    {
        var rounded = MathF.Round(value ?? 0f, MidpointRounding.AwayFromZero);
        return float.IsNaN(rounded) ? 0 : rounded >= int.MaxValue ? int.MaxValue : rounded <= int.MinValue ? int.MinValue : (int)rounded;
    }

    /// <summary>
    /// How the graph's properties show in the Inspector, as a script's description lists it: each
    /// property with options in order, those of a group together in it (where the group first
    /// appears). Null when no property has options (one plain row each).
    /// </summary>
    public static JsonValue? InspectorLayout(Graph.Graph graph)
    {
        if (graph.Properties.All(p => p.Inspector.IsEmpty))
        {
            return null;
        }
        static JsonValue Item(Graph.DeclaredProperty p)
        {
            var o = p.Inspector;
            var item = new JsonObject { { "property", new JsonString(p.Name) } };
            if (o.Label.Length > 0)
            {
                item.Add("label", new JsonString(o.Label));
            }
            if (o.Hint.Length > 0)
            {
                item.Add("hint", new JsonString(o.Hint));
            }
            if (o.Range is { } range)
            {
                item.Add("range", new JsonArray([JsonNumber.Of(range.Low), JsonNumber.Of(range.High)]));
            }
            if (o.Slider)
            {
                item.Add("slider", JsonValue.From(true));
            }
            if (o.ReadOnly)
            {
                item.Add("readOnly", JsonValue.From(true));
            }
            if (o.Choices.Count > 0)
            {
                item.Add("choices", new JsonArray(o.Choices.Select(c => (JsonValue)new JsonString(c))));
            }
            if (o.ShowIf is { } showIf)
            {
                item.Add("showIf", new JsonArray([new JsonString(showIf.Property), showIf.Value]));
            }
            return item.Canonical();
        }
        var layout = new List<JsonValue>();
        // The items of each group, by its title, in the order the groups first appear.
        var groups = new Dictionary<string, List<JsonValue>>(StringComparer.Ordinal);
        var order = new List<(string? Group, JsonValue? Item)>();
        foreach (var p in graph.Properties.Where(p => !p.Inspector.IsEmpty))
        {
            if (p.Inspector.Group.Length == 0)
            {
                order.Add((null, Item(p)));
                continue;
            }
            if (!groups.TryGetValue(p.Inspector.Group, out var items))
            {
                items = [];
                groups.Add(p.Inspector.Group, items);
                order.Add((p.Inspector.Group, null));
            }
            items.Add(Item(p));
        }
        foreach (var (group, item) in order)
        {
            layout.Add(group is null ? item! : new JsonObject { { "group", new JsonString(group) }, { "items", new JsonArray(groups[group]) } }.Canonical());
        }
        return new JsonArray(layout);
    }

    /// <summary>
    /// Checks the graph's Inspector options as the script host will when it loads the script:
    /// properties named are declared, options fit their types, and so on.
    /// </summary>
    public static Result<Unit, string> CheckInspector(Graph.Graph graph)
    {
        if (InspectorLayout(graph) is not { } layout)
        {
            return Result.Ok<string>();
        }
        var parsed = InspectorItem.Parse(layout, DeclaredProperties(graph));
        return parsed.IsOk ? Result.Ok<string>() : Result.Fail(parsed.Error);
    }
}

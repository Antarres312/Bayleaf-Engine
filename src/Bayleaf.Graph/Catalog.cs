using System.Numerics;

using Bayleaf.Core.Json;

namespace Bayleaf.Graph;

/// <summary>Default of an input pin that has no link.</summary>
public abstract record PinDefault
{
    private PinDefault()
    {
    }

    public static readonly PinDefault None = new NoDefault();

    public sealed record NoDefault : PinDefault;

    public sealed record Number(float Value) : PinDefault;

    public sealed record Vec3(Vector3 Value) : PinDefault;

    public sealed record Color(Vector3 Value) : PinDefault;

    public sealed record Bool(bool Value) : PinDefault;

    public sealed record Text(string Value) : PinDefault;

    public sealed record WholeNumber(int Value) : PinDefault;

    /// <summary>Filled in by the domain (for example the surface position in a material).</summary>
    public sealed record Implicit(string What) : PinDefault;
}

/// <summary>One input or output of a node kind.</summary>
/// <param name="Name">Stable id used in files and commands.</param>
/// <param name="Label">English label. It is the translation key.</param>
public sealed record PinDef(string Name, string Label, PinType Type, PinDefault Default)
{
    public PinDef(string name, string label, PinType type)
        : this(name, label, type, PinDefault.None)
    {
    }
}

/// <summary>A value on a node that is not a pin, such as a key name or a constant.</summary>
public abstract record SettingKind
{
    private SettingKind()
    {
    }

    public sealed record Number(float Value) : SettingKind;

    public sealed record Color(Vector3 Value) : SettingKind;

    public sealed record Vec3(Vector3 Value) : SettingKind;

    /// <summary>Free text up to 256 characters.</summary>
    public sealed record Text(string Value) : SettingKind;

    /// <summary>Text that names something: letters, digits and <c>_</c> only.</summary>
    public sealed record Name(string Value) : SettingKind;

    /// <summary>One of a fixed list of words; the first is the default.</summary>
    public sealed record Choice(IReadOnlyList<string> Words) : SettingKind;

    /// <summary>
    /// A list of up to 64 names (1 to 64 characters each) the node keeps for itself, such as the
    /// properties of the script a Get Component names. Not shown as a row of the node.
    /// </summary>
    public sealed record Names : SettingKind;
}

public sealed record SettingDef(string Name, string Label, SettingKind Kind)
{
    /// <summary>Whether the node shows it as a row (a list of names is kept, not shown).</summary>
    public bool Shown => Kind is not SettingKind.Names;
}

/// <summary>One kind of node in a domain.</summary>
/// <param name="Kind">Stable id used in files and commands, e.g. <c>multiply</c>.</param>
/// <param name="Label">English label. It is the translation key.</param>
/// <param name="Category">Group in the add-node menu.</param>
/// <param name="Unique">At most one node of this kind per graph (for example the material output).</param>
public sealed record NodeDef(
    string Kind,
    string Label,
    string Category,
    string Description,
    IReadOnlyList<PinDef> Inputs,
    IReadOnlyList<PinDef> Outputs,
    IReadOnlyList<SettingDef> Settings,
    bool Unique)
{
    public PinDef? Input(string name) => Inputs.FirstOrDefault(p => Say.Equal(p.Name, name));

    public PinDef? Output(string name) => Outputs.FirstOrDefault(p => Say.Equal(p.Name, name));

    public SettingDef? Setting(string name) => Settings.FirstOrDefault(s => Say.Equal(s.Name, name));
}

/// <summary>The node kinds of one domain.</summary>
public sealed class Catalog
{
    /// <summary><c>material</c>, <c>script</c> or <c>animation</c>.</summary>
    public required string Domain { get; init; }

    /// <summary>English name of graphs in this domain, e.g. "Material".</summary>
    public required string Label { get; init; }

    /// <summary>File extension of this domain's graphs, without the dot.</summary>
    public required string Extension { get; init; }

    public IReadOnlyList<NodeDef> Nodes { get; init; } = [];

    /// <summary>Kinds a new graph starts with, and where.</summary>
    public IReadOnlyList<(string Kind, Vector2 Position)> Starter { get; init; } = [];

    /// <summary>
    /// A node's own pins when they depend on its settings (Get Component shows the fields of the
    /// component it names); null for the kind's pins.
    /// </summary>
    public Func<Node, NodeDef?> Variant { get; init; } = NoVariant;

    /// <summary>
    /// Moves what this domain's nodes kept in files older than <see cref="Graph.Version"/> (the
    /// version read is given) into where it belongs now.
    /// </summary>
    public Action<Graph, uint> Upgrade { get; init; } = NoUpgrade;

    /// <summary>
    /// Kinds whose number-or-vector output is a number even when every input is a whole number
    /// (Sine, Square Root). Other kinds keep whole numbers whole. See <see cref="Graph.OutputType"/>.
    /// </summary>
    public IReadOnlyList<string> Fractional { get; init; } = [];

    /// <summary>For catalogs whose files need nothing moved between versions.</summary>
    public static void NoUpgrade(Graph graph, uint version)
    {
    }

    /// <summary>For catalogs whose node kinds always have the same pins.</summary>
    public static NodeDef? NoVariant(Node node) => null;

    public NodeDef? Get(string kind) => Nodes.FirstOrDefault(n => Say.Equal(n.Kind, kind));

    /// <summary>The definition a node is drawn and compiled with: its variant, else its kind's.</summary>
    public NodeDef? DefOf(Node node) => Variant(node) ?? Get(node.Kind);

    /// <summary>The catalog's description as JSON (for <c>graph.catalog</c> and agents).</summary>
    public JsonValue ToJson()
    {
        static JsonValue Pin(PinDef p) => new JsonObject { { "name", J.S(p.Name) }, { "label", J.S(p.Label) }, { "type", p.Type.ToJson() } };
        var nodes = new JsonArray(Nodes.Select(n => (JsonValue)new JsonObject
        {
            { "kind", J.S(n.Kind) },
            { "label", J.S(n.Label) },
            { "category", J.S(n.Category) },
            { "description", J.S(n.Description) },
            { "inputs", new JsonArray(n.Inputs.Select(Pin)) },
            { "outputs", new JsonArray(n.Outputs.Select(Pin)) },
            { "settings", new JsonArray(n.Settings.Select(s => (JsonValue)new JsonObject { { "name", J.S(s.Name) }, { "label", J.S(s.Label) } })) },
        }));
        return new JsonObject { { "domain", J.S(Domain) }, { "label", J.S(Label) }, { "extension", J.S(Extension) }, { "nodes", nodes } }.Canonical();
    }
}

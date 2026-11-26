using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Scene;

/// <summary>One object. Its position among its siblings is its order in <see cref="SceneDoc.Objects"/>.</summary>
public sealed record ObjectDoc(StableId Id, StableId? Parent, SortedDictionary<string, JsonValue> Components)
{
    public bool Equals(ObjectDoc? other) => other is not null && Id == other.Id && Parent == other.Parent && ToJson().Equals(other.ToJson());

    public override int GetHashCode() => Id.GetHashCode();

    /// <summary>A copy whose component map can be changed without changing this one's.</summary>
    public ObjectDoc Copy() => this with { Components = new SortedDictionary<string, JsonValue>(Components, Utf8Order.Instance) };

    /// <summary>Reads an object as scene files and clipboard data hold it; throws <see cref="JsonShapeException"/> for another shape.</summary>
    public static ObjectDoc FromJson(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ObjectDoc", true, "id", "parent", "components");
        return new(f.Required("id", JsonRead.Id), f.NullableValue("parent", JsonRead.Id), f.Required("components", v => JsonRead.Map(v, c => c.Canonical())));
    }

    /// <summary>The object as scene files hold it.</summary>
    public JsonObject ToJson() => new()
    {
        { "id", J.S(Id.ToHex()) },
        { "parent", Parent is { } parent ? J.S(parent.ToHex()) : JsonValue.Null },
        { "components", new JsonObject(Components.Select(c => new KeyValuePair<string, JsonValue>(c.Key, c.Value))) },
    };
}

/// <summary>
/// A whole scene as plain data, as a <c>.bscene</c> file holds it: pretty JSON, objects listed parents
/// first in hierarchy order. Also used for undo snapshots.
/// </summary>
public sealed record SceneDoc(string Format, uint Version, SceneSettings Settings, IReadOnlyList<ObjectDoc> Objects)
{
    public const string FormatName = "bayleaf.scene";

    /// <summary>
    /// Version 2 replaced the single <c>script</c> component with the <c>scripts</c> list; version 3
    /// added <c>mesh.material</c>; ... version 26 skeletal animation. Older files still load. The
    /// version list is in the Rust engine's <c>crates/scene/src/doc.rs</c>; both engines read and
    /// write the same versions.
    /// </summary>
    public const uint CurrentVersion = 26;

    public const string Extension = "bscene";

    public bool Equals(SceneDoc? other) => other is not null && ToJson().Equals(other.ToJson());

    public override int GetHashCode() => Objects.Count;

    public static Result<SceneDoc, string> Read(JsonValue value) => JsonRead.Try(() =>
    {
        var f = JsonFields.Of(value, "struct SceneDoc", true, "format", "version", "settings", "objects");
        return new SceneDoc(f.Required("format", JsonRead.Str), f.Required("version", JsonRead.U32), f.Optional("settings", SceneSettings.Read, SceneSettings.Default),
            f.Required("objects", v => (IReadOnlyList<ObjectDoc>)JsonRead.List(v, ObjectDoc.FromJson)));
    });

    public static Result<SceneDoc, string> FromText(string text) => JsonReader.Parse(text).TryGet(out var value, out var error) ? Read(value) : Result.Fail(error);

    public JsonObject ToJson()
    {
        var o = new JsonObject { { "format", J.S(Format) }, { "version", J.U(Version) } };
        if (!Settings.IsDefault)
        {
            o.Add("settings", Settings.Write());
        }
        o.Add("objects", new JsonArray(Objects.Select(obj => obj.ToJson())));
        return o;
    }

    /// <summary>The file's text, as saving writes it.</summary>
    public string ToText() => ToJson().ToPretty() + "\n";
}

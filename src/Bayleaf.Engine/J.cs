using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Engine;

/// <summary>Short builders of command results.</summary>
internal static class J
{
    public static JsonObject Obj(params (string Key, JsonValue? Value)[] entries)
    {
        var o = new JsonObject();
        foreach (var (key, value) in entries)
        {
            o.Add(key, value ?? JsonValue.Null);
        }
        return o;
    }

    public static JsonValue S(string? value) => value is null ? JsonValue.Null : new JsonString(value);

    public static JsonValue B(bool value) => JsonValue.From(value);

    public static JsonValue N(long value) => JsonNumber.Of(value);

    public static JsonValue Id(StableId id) => new JsonString(id.ToHex());

    public static JsonValue Id(StableId? id) => id is { } some ? new JsonString(some.ToHex()) : JsonValue.Null;

    public static JsonArray Ids(IEnumerable<StableId> ids) => new(ids.Select(Id));

    public static JsonArray Strings(IEnumerable<string> values) => new(values.Select(v => (JsonValue)new JsonString(v)));
}

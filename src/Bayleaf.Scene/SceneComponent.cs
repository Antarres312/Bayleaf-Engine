using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Scene;

/// <summary>
/// A component type the scene can store, show in the inspector and write to scene files. Each one
/// is registered in <see cref="ComponentRegistry.Builtin"/>.
/// </summary>
public interface ISceneComponent<TSelf>
    where TSelf : ISceneComponent<TSelf>
{
    /// <summary>Key in scene files and command parameters.</summary>
    static abstract string Key { get; }

    /// <summary>English label, used as the translation key.</summary>
    static abstract string Label { get; }

    /// <summary>False for components every object always has.</summary>
    static abstract bool Removable { get; }

    /// <summary>False for components the inspector shows in its own way and the generic Add Component list leaves out.</summary>
    static virtual bool Listed => true;

    /// <summary>Where Add Component lists it: folders, then its name (<c>Light/Point Light</c>); empty for the top level.</summary>
    static virtual string Menu => "";

    /// <summary>The value a new component of this type starts with.</summary>
    static abstract TSelf Initial { get; }

    /// <summary>Reads a value as a scene file holds it; throws <see cref="JsonShapeException"/> for one of another shape.</summary>
    static abstract TSelf Read(JsonValue value);

    /// <summary>Allowed values of a text field, for fields that take one of a fixed set.</summary>
    static virtual IReadOnlyList<string> Options(string field) => [];

    /// <summary>The value as a scene file holds it.</summary>
    JsonValue Write();

    /// <summary>Checks a value that came from a file or a command.</summary>
    Result<Unit, string> Validate() => Unit.Value;
}

/// <summary>Checks the components share.</summary>
internal static class Checks
{
    public static Result<Unit, string> Ok => Unit.Value;

    public static Result<Unit, string> Fail(string message) => Result.Fail(message);

    public static Result<Unit, string> Finite(string what, IEnumerable<float> values) =>
        values.All(float.IsFinite) ? Ok : Fail($"{what} values must be finite numbers");

    public static Result<Unit, string> UnitRange(string what, IEnumerable<float> values) =>
        values.All(v => J.Within(v, 0f, 1f)) ? Ok : Fail($"{what} values must be between 0 and 1");

    /// <summary>An optional file path field: empty, or at most 512 bytes ending in one of <paramref name="extensions"/>.</summary>
    public static Result<Unit, string> FileField(string name, string value, params string[] extensions) =>
        value.Length == 0 || (Text.Bytes(value) <= 512 && extensions.Any(e => Text.AsciiLower(value).EndsWith($".{e}", StringComparison.Ordinal)))
            ? Ok
            : Fail($"{name} must be the path of a .{string.Join(" or .", extensions)} file (at most 512 characters)");

    public static Result<Unit, string> LightAmounts(float intensity, float range)
    {
        if (!(float.IsFinite(intensity) && intensity >= 0f))
        {
            return Fail("intensity must be 0 or more");
        }
        return float.IsFinite(range) && range > 0f && range <= 100_000f ? Ok : Fail("range must be more than 0 and at most 100000");
    }

    /// <summary>Runs the checks in turn and gives the first failure.</summary>
    public static Result<Unit, string> All(params Func<Result<Unit, string>>[] checks)
    {
        foreach (var check in checks)
        {
            var result = check();
            if (result.IsError)
            {
                return result;
            }
        }
        return Ok;
    }
}

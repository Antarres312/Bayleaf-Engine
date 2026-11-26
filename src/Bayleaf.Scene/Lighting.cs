using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;

using EnvironmentComponent = Bayleaf.Scene.Components.Environment;

namespace Bayleaf.Scene;

/// <summary>The sun of a lighting asset: a light from far away, as a Directional Light gives.</summary>
public sealed record Sun(bool Enabled, Vector3 Rotation, Vector3 Color, float Intensity, bool Shadows)
{
    public static Sun Default
    {
        get
        {
            var light = DirectionalLight.Initial;
            return new(true, new(-50f, 30f, 0f), light.Color, light.Intensity, light.Shadows);
        }
    }

    public static Sun Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Sun", true, "enabled", "rotation", "color", "intensity", "shadows");
        return new(f.Required("enabled", JsonRead.Bool), f.Required("rotation", J.ReadV3), f.Required("color", J.ReadV3), f.Required("intensity", JsonRead.F32), f.Required("shadows", JsonRead.Bool));
    }

    public JsonValue Write() => new JsonObject { { "enabled", J.B(Enabled) }, { "rotation", J.V3(Rotation) }, { "color", J.V3(Color) }, { "intensity", J.F(Intensity) }, { "shadows", J.B(Shadows) } };
}

/// <summary>
/// A lighting asset (<c>.blight</c>): what a scene's light looks like, kept in a file of its own so
/// several scenes can share it. It holds the sky's colours, the light around everything, fog, and a
/// sun that lights the scene where it has no Directional Light of its own.
/// </summary>
public sealed record Lighting(Vector3 SkyColor, Vector3 HorizonColor, Vector3 GroundColor, float Ambient, bool ShowSky, float FogDistance, Sun Sun)
{
    public const string Extension = "blight";
    public const string Format = "bayleaf.lighting";
    public const uint Version = 1;

    public static Lighting Default => FromEnvironment(EnvironmentComponent.Initial, Sun.Default);

    /// <summary>The light an Environment component gave, with <paramref name="sun"/>; how older scenes move to lighting assets.</summary>
    public static Lighting FromEnvironment(EnvironmentComponent env, Sun sun) => new(env.SkyColor, env.HorizonColor, env.GroundColor, env.Ambient, env.ShowSky, env.FogDistance, sun);

    /// <summary>The same light as an Environment component, for drawing (the sun aside).</summary>
    public EnvironmentComponent Environment() => new(SkyColor, HorizonColor, GroundColor, Ambient, ShowSky, FogDistance);

    private static Lighting ReadLighting(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Lighting", true, "skyColor", "horizonColor", "groundColor", "ambient", "showSky", "fogDistance", "sun");
        return new(f.Required("skyColor", J.ReadV3), f.Required("horizonColor", J.ReadV3), f.Required("groundColor", J.ReadV3), f.Required("ambient", JsonRead.F32),
            f.Required("showSky", JsonRead.Bool), f.Optional("fogDistance", JsonRead.F32, 0f), f.Optional("sun", Sun.Read, Sun.Default));
    }

    private JsonObject WriteLighting() => new()
    {
        { "skyColor", J.V3(SkyColor) },
        { "horizonColor", J.V3(HorizonColor) },
        { "groundColor", J.V3(GroundColor) },
        { "ambient", J.F(Ambient) },
        { "showSky", J.B(ShowSky) },
        { "fogDistance", J.F(FogDistance) },
        { "sun", Sun.Write() },
    };

    /// <summary>Checks every value is in its range.</summary>
    public Result<Unit, string> Validate()
    {
        static Result<Unit, string> UnitColor(string name, Vector3 values) => Checks.UnitRange(name, J.Of(values)).MapError(_ => $"{name} values must be between 0 and 1");
        return Checks.All(
            () => ((ISceneComponent<EnvironmentComponent>)Environment()).Validate(),
            () => UnitColor("sun.color", Sun.Color),
            () => J.Of(Sun.Rotation).All(float.IsFinite) ? Checks.Ok : Checks.Fail("sun.rotation values must be finite"),
            () => J.Within(Sun.Intensity, 0f, 100f) ? Checks.Ok : Checks.Fail("sun.intensity must be between 0 and 100"),
            () => UnitColor("skyColor", SkyColor));
    }

    /// <summary>The file's text.</summary>
    public string ToText() => new JsonObject { { "format", J.S(Format) }, { "version", J.U(Version) }, { "lighting", WriteLighting() } }.ToPretty() + "\n";

    /// <summary>Reads a lighting file's text.</summary>
    public static Result<Lighting, SceneError> FromText(string text)
    {
        var read = JsonReader.Parse(text).TryGet(out var value, out var parseError)
            ? JsonRead.Try(() =>
            {
                var f = JsonFields.Of(value, "struct LightingFile", true, "format", "version", "lighting");
                return (Format: f.Required("format", JsonRead.Str), Version: f.Required("version", JsonRead.U32), Lighting: f.Required("lighting", ReadLighting));
            })
            : Result.Fail(parseError);
        if (!read.TryGet(out var file, out var error))
        {
            return Result.Fail(SceneError.Format(error));
        }
        if (file.Format != Format)
        {
            return Result.Fail(SceneError.Format($"not a lighting file (format \"{file.Format}\")"));
        }
        if (file.Version is < 1 or > Version)
        {
            return Result.Fail(SceneError.Format($"unsupported lighting version {file.Version}"));
        }
        var valid = file.Lighting.Validate();
        return valid.IsOk ? file.Lighting : Result.Fail(SceneError.Format(valid.Error));
    }

    public static Result<Lighting, SceneError> Load(string path)
    {
        var text = Files.ReadText(path);
        return text.IsOk ? FromText(text.Value) : Result.Fail(SceneError.Io($"cannot read {path}: {text.Error}"));
    }

    /// <summary>Writes the file through a temporary one, so a failed write keeps the old file.</summary>
    public Result<Unit, SceneError> Save(string path)
    {
        var valid = Validate();
        if (valid.IsError)
        {
            return Result.Fail(SceneError.Format(valid.Error));
        }
        return Files.WriteReplacing(path, Path.ChangeExtension(path, "blight.tmp"), ToText()).MapError(SceneError.Io);
    }
}

/// <summary>A scene's own settings, kept in its file beside its objects.</summary>
public sealed record SceneSettings(string Lighting, string PostProcessing)
{
    public static readonly SceneSettings Default = new("", "");

    public bool IsDefault => Lighting.Length == 0 && PostProcessing.Length == 0;

    /// <summary>Whether <paramref name="path"/> is empty or a short path of a file with <paramref name="extension"/>.</summary>
    internal static Result<Unit, string> FilePath(string field, string path, string extension) =>
        path.Length == 0 || (Text.Bytes(path) <= 512 && Text.AsciiLower(path).EndsWith($".{extension}", StringComparison.Ordinal))
            ? Checks.Ok
            : Checks.Fail($"{field} must be the path of a .{extension} file");

    public Result<Unit, string> Validate() => Checks.All(
        () => FilePath("lighting", Lighting, Bayleaf.Scene.Lighting.Extension),
        () => FilePath("postProcessing", PostProcessing, Bayleaf.Scene.PostProcessing.Extension));

    public static SceneSettings Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct SceneSettings", true, "lighting", "postProcessing");
        return new(f.Optional("lighting", JsonRead.Str, ""), f.Optional("postProcessing", JsonRead.Str, ""));
    }

    public JsonObject Write()
    {
        var o = new JsonObject();
        if (Lighting.Length > 0)
        {
            o.Add("lighting", J.S(Lighting));
        }
        if (PostProcessing.Length > 0)
        {
            o.Add("postProcessing", J.S(PostProcessing));
        }
        return o;
    }
}

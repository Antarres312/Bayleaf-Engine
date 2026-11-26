using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Scene;

/// <summary>How a picture's remaining jagged edges are smoothed.</summary>
public enum Antialiasing
{
    Off,

    /// <summary>Each pixel blended with its four neighbours where they differ sharply: quick, a little soft.</summary>
    Fast,

    /// <summary>Along each edge's own direction, sub-pixel steps too: smoother edges and thin lines.</summary>
    Fine,
}

/// <summary>High-range colour onto the screen with a filmic curve, after exposure.</summary>
public sealed record ToneMapping(bool Enabled, float Exposure)
{
    public static readonly ToneMapping Default = new(true, 1f);

    public static ToneMapping Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ToneMapping", true, "enabled", "exposure");
        return new(f.Required("enabled", JsonRead.Bool), f.Required("exposure", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject { { "enabled", J.B(Enabled) }, { "exposure", J.F(Exposure) } };
}

/// <summary>Bright parts glow.</summary>
public sealed record Bloom(bool Enabled, float Strength, float Threshold)
{
    public static readonly Bloom Default = new(false, 0.6f, 1f);

    public static Bloom Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Bloom", true, "enabled", "strength", "threshold");
        return new(f.Required("enabled", JsonRead.Bool), f.Required("strength", JsonRead.F32), f.Required("threshold", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject { { "enabled", J.B(Enabled) }, { "strength", J.F(Strength) }, { "threshold", J.F(Threshold) } };
}

/// <summary>
/// Contrast, saturation and a lookup image that maps each colour to another: N squares of N by N
/// pixels side by side (N² wide and N high, N from 2 to 64), one square for each step of blue, red
/// across each square and green down it, all as sRGB.
/// </summary>
public sealed record ColorAdjustment(bool Enabled, float Contrast, float Saturation, string Lookup, float LookupStrength)
{
    /// <summary>Most steps per side of a lookup image.</summary>
    public const uint MaxLookupSteps = 64;

    public static readonly ColorAdjustment Default = new(false, 1f, 1f, "", 1f);

    /// <summary>Whether an image of this size is a lookup image: N squares of N by N side by side.</summary>
    public static bool IsLookupSize(uint width, uint height) => height is >= 2 and <= MaxLookupSteps && width == height * height;

    public static ColorAdjustment Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ColorAdjustment", true, "enabled", "contrast", "saturation", "lookup", "lookupStrength");
        return new(f.Required("enabled", JsonRead.Bool), f.Required("contrast", JsonRead.F32), f.Required("saturation", JsonRead.F32), f.Optional("lookup", JsonRead.Str, ""),
            f.Optional("lookupStrength", JsonRead.F32, 1f));
    }

    public JsonValue Write() => new JsonObject
    {
        { "enabled", J.B(Enabled) },
        { "contrast", J.F(Contrast) },
        { "saturation", J.F(Saturation) },
        { "lookup", J.S(Lookup) },
        { "lookupStrength", J.F(LookupStrength) },
    };
}

/// <summary>Darker corners.</summary>
public sealed record Vignette(bool Enabled, float Strength, float Smoothness)
{
    public static readonly Vignette Default = new(false, 0.4f, 0.5f);

    public static Vignette Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Vignette", true, "enabled", "strength", "smoothness");
        return new(f.Required("enabled", JsonRead.Bool), f.Required("strength", JsonRead.F32), f.Required("smoothness", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject { { "enabled", J.B(Enabled) }, { "strength", J.F(Strength) }, { "smoothness", J.F(Smoothness) } };
}

/// <summary>Fog by distance from the camera, from Start, hiding everything from End.</summary>
public sealed record DepthFog(bool Enabled, Vector3 Color, float Start, float End)
{
    public static readonly DepthFog Default = new(false, new(0.7f, 0.75f, 0.8f), 20f, 200f);

    public static DepthFog Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct DepthFog", true, "enabled", "color", "start", "end");
        return new(f.Required("enabled", JsonRead.Bool), f.Required("color", J.ReadV3), f.Required("start", JsonRead.F32), f.Required("end", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject { { "enabled", J.B(Enabled) }, { "color", J.V3(Color) }, { "start", J.F(Start) }, { "end", J.F(End) } };
}

/// <summary>Creases and corners go darker (ambient occlusion, from depth).</summary>
public sealed record Occlusion(bool Enabled, float Strength, float Radius)
{
    public static readonly Occlusion Default = new(false, 0.6f, 0.5f);

    public static Occlusion Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Occlusion", true, "enabled", "strength", "radius");
        return new(f.Required("enabled", JsonRead.Bool), f.Required("strength", JsonRead.F32), f.Required("radius", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject { { "enabled", J.B(Enabled) }, { "strength", J.F(Strength) }, { "radius", J.F(Radius) } };
}

/// <summary>
/// A post-processing asset (<c>.bpost</c>): what the camera does to the drawn picture. Each effect is
/// on or off with its settings, applied in a fixed order: occlusion, depth fog, bloom, exposure and
/// tone mapping, colour, antialiasing, vignette.
/// </summary>
public sealed record PostProcessing(ToneMapping ToneMapping, Bloom Bloom, ColorAdjustment Color, Vignette Vignette, DepthFog DepthFog, Occlusion Occlusion, Antialiasing Antialiasing)
{
    public const string Extension = "bpost";
    public const string Format = "bayleaf.post";

    /// <summary>Version 2 adds the colour lookup image; version 3 makes antialiasing a choice (true and false still read as fast and off).</summary>
    public const uint Version = 3;

    private static readonly string[] AntialiasingNames = ["off", "fast", "fine"];

    public static readonly PostProcessing Default = new(ToneMapping.Default, Bloom.Default, ColorAdjustment.Default, Vignette.Default, DepthFog.Default, Occlusion.Default, Antialiasing.Off);

    private static Antialiasing ReadAntialiasing(JsonValue value) => value switch
    {
        // Files before version 3 kept it on or off.
        JsonBool b => b.Value ? Antialiasing.Fast : Antialiasing.Off,
        JsonString s => Array.IndexOf(AntialiasingNames, s.Value) is var at and >= 0
            ? (Antialiasing)at
            : throw new JsonShapeException($"antialiasing is off, fast or fine, not {Text.Debug(s.Value)}"),
        _ => throw new JsonShapeException("data did not match any variant of untagged enum Stored"),
    };

    private static PostProcessing ReadPost(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct PostProcessing", true, "toneMapping", "bloom", "color", "vignette", "depthFog", "occlusion", "antialiasing");
        return new(f.Optional("toneMapping", ToneMapping.Read, ToneMapping.Default), f.Optional("bloom", Bloom.Read, Bloom.Default), f.Optional("color", ColorAdjustment.Read, ColorAdjustment.Default),
            f.Optional("vignette", Vignette.Read, Vignette.Default), f.Optional("depthFog", DepthFog.Read, DepthFog.Default), f.Optional("occlusion", Occlusion.Read, Occlusion.Default),
            f.Optional("antialiasing", ReadAntialiasing, Antialiasing.Off));
    }

    private JsonObject WritePost() => new()
    {
        { "toneMapping", ToneMapping.Write() },
        { "bloom", Bloom.Write() },
        { "color", Color.Write() },
        { "vignette", Vignette.Write() },
        { "depthFog", DepthFog.Write() },
        { "occlusion", Occlusion.Write() },
        { "antialiasing", J.S(AntialiasingNames[(int)Antialiasing]) },
    };

    /// <summary>The effects as commands give and take them (the file's <c>post</c>).</summary>
    public JsonObject ToJson() => WritePost();

    /// <summary>Reads the effects as commands take them; throws <see cref="JsonShapeException"/> for another shape.</summary>
    public static PostProcessing FromJson(JsonValue value) => ReadPost(value);

    private static Result<Unit, string> Within(string name, float value, float low, float high) =>
        float.IsFinite(value) && J.Within(value, low, high) ? Checks.Ok : Checks.Fail($"{name} must be between {Text.Number(low)} and {Text.Number(high)}");

    /// <summary>Checks every value is in its range.</summary>
    public Result<Unit, string> Validate() => Checks.All(
        () => Within("toneMapping.exposure", ToneMapping.Exposure, 0f, 16f),
        () => Within("bloom.strength", Bloom.Strength, 0f, 4f),
        () => Within("bloom.threshold", Bloom.Threshold, 0f, 16f),
        () => Within("color.contrast", Color.Contrast, 0f, 2f),
        () => Within("color.saturation", Color.Saturation, 0f, 2f),
        () => Within("color.lookupStrength", Color.LookupStrength, 0f, 1f),
        () => Text.Bytes(Color.Lookup) > 1024 ? Checks.Fail("color.lookup is longer than 1024 bytes") : Checks.Ok,
        () => Within("vignette.strength", Vignette.Strength, 0f, 1f),
        () => Within("vignette.smoothness", Vignette.Smoothness, 0f, 1f),
        () => Within("depthFog.color", DepthFog.Color.X, 0f, 1f),
        () => Within("depthFog.color", DepthFog.Color.Y, 0f, 1f),
        () => Within("depthFog.color", DepthFog.Color.Z, 0f, 1f),
        () => Within("depthFog.start", DepthFog.Start, 0f, 1_000_000f),
        () => Within("depthFog.end", DepthFog.End, 0f, 1_000_000f),
        () => DepthFog.End <= DepthFog.Start ? Checks.Fail("depthFog.end must be past depthFog.start") : Checks.Ok,
        () => Within("occlusion.strength", Occlusion.Strength, 0f, 1f),
        () => Within("occlusion.radius", Occlusion.Radius, 0.01f, 10f));

    public string ToText() => new JsonObject { { "format", J.S(Format) }, { "version", J.U(Version) }, { "post", WritePost() } }.ToPretty() + "\n";

    public static Result<PostProcessing, SceneError> FromText(string text)
    {
        var read = JsonReader.Parse(text).TryGet(out var value, out var parseError)
            ? JsonRead.Try(() =>
            {
                var f = JsonFields.Of(value, "struct PostFile", true, "format", "version", "post");
                return (Format: f.Required("format", JsonRead.Str), Version: f.Required("version", JsonRead.U32), Post: f.Required("post", ReadPost));
            })
            : Result.Fail(parseError);
        if (!read.TryGet(out var file, out var error))
        {
            return Result.Fail(SceneError.Format(error));
        }
        if (file.Format != Format)
        {
            return Result.Fail(SceneError.Format($"not a post-processing file (format \"{file.Format}\")"));
        }
        if (file.Version is < 1 or > Version)
        {
            return Result.Fail(SceneError.Format($"unsupported post-processing version {file.Version}"));
        }
        var valid = file.Post.Validate();
        return valid.IsOk ? file.Post : Result.Fail(SceneError.Format(valid.Error));
    }

    public static Result<PostProcessing, SceneError> Load(string path)
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
        return Files.WriteReplacing(path, Path.ChangeExtension(path, "bpost.tmp"), ToText()).MapError(SceneError.Io);
    }
}

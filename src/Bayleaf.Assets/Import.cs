using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Assets;

/// <summary>Which axis a model's maker used as up; the engine's up is Y.</summary>
public enum UpAxis
{
    Y,
    Z,
}

/// <summary>What an image's pixels are.</summary>
public enum ColorSpace
{
    /// <summary>Colours, stored as sRGB: shown as they look.</summary>
    Color,

    /// <summary>Plain numbers (normals, roughness, masks): read as stored.</summary>
    Data,
}

/// <summary>How an image is kept on the GPU.</summary>
public enum Compression
{
    /// <summary>Every pixel as it is: 4 bytes each.</summary>
    None,

    /// <summary>BC1: half a byte a pixel; transparency only on or off.</summary>
    Bc1,

    /// <summary>BC3: a byte a pixel, with smooth transparency.</summary>
    Bc3,
}

/// <summary>What kind of file settings are kept for, by its extension.</summary>
public enum Imported
{
    Model,
    Image,
    Sound,
}

/// <summary>How a model is read.</summary>
/// <param name="Scale">Every position times this: 0.01 for a model made in centimetres.</param>
/// <param name="UpAxis">Which axis the model's maker used as up; the engine's up is Y.</param>
public sealed record ModelImport(float Scale, UpAxis UpAxis)
{
    public static readonly ModelImport Default = new(1f, UpAxis.Y);

    internal static ModelImport Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ModelImport", true, "scale", "upAxis");
        return new(f.Required("scale", JsonRead.F32), (UpAxis)f.Required("upAxis", v => JsonRead.Variant(v, "y", "z")));
    }

    internal JsonObject Write() => new() { { "scale", JsonNumber.Of(Scale) }, { "upAxis", new JsonString(UpAxis == UpAxis.Y ? "y" : "z") } };
}

/// <summary>How an image is read.</summary>
/// <param name="Mipmaps">Smaller copies for far away, so it does not shimmer; off keeps every pixel sharp (pixel art).</param>
/// <param name="ColorSpace">Whether the pixels are colours (stored as sRGB) or plain numbers (a normal map, a mask).</param>
/// <param name="Compression">How it is kept on the GPU: as it is, or in 4 x 4 blocks that take an eighth or a quarter of the memory and lose a little detail.</param>
public sealed record ImageImport(bool Mipmaps, ColorSpace ColorSpace, Compression Compression)
{
    public static readonly ImageImport Default = new(true, ColorSpace.Color, Compression.None);

    internal static ImageImport Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ImageImport", true, "mipmaps", "colorSpace", "compression");
        return new(
            f.Required("mipmaps", JsonRead.Bool),
            (ColorSpace)f.Optional("colorSpace", v => JsonRead.Variant(v, "color", "data"), 0),
            (Compression)f.Optional("compression", v => JsonRead.Variant(v, "none", "bc1", "bc3"), 0));
    }

    internal JsonObject Write() => new()
    {
        { "mipmaps", JsonValue.From(Mipmaps) },
        { "colorSpace", new JsonString(ColorSpace == ColorSpace.Color ? "color" : "data") },
        { "compression", new JsonString(Compression switch { Compression.Bc1 => "bc1", Compression.Bc3 => "bc3", _ => "none" }) },
    };
}

/// <summary>How a sound is played.</summary>
/// <param name="Stream">Read from its file as it plays (long music), instead of decoded whole when first played.</param>
public sealed record SoundImport(bool Stream)
{
    public static readonly SoundImport Default = new(false);

    internal static SoundImport Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct SoundImport", true, "stream");
        return new(f.Required("stream", JsonRead.Bool));
    }

    internal JsonObject Write() => new() { { "stream", JsonValue.From(Stream) } };
}

/// <summary>
/// Import settings: a small JSON file beside a model or image brought into a project
/// (<c>robot.glb.bimport</c> beside <c>robot.glb</c>), saying how to read it: a model's scale and
/// which way is up, whether an image gets smaller copies for far away, holds colour or plain
/// numbers, and is kept block-compressed on the GPU, and whether a sound is played straight from
/// its file. The loaders apply them, and read the file again when the settings change. A file
/// without settings is read with the defaults.
/// </summary>
public sealed record ImportSettings(string Format, uint Version, ModelImport? Model, ImageImport? Image, SoundImport? Sound)
{
    /// <summary>The settings file's extension, added after the file's own.</summary>
    public const string SettingsExtension = "bimport";
    public const string FormatName = "bayleaf.import";
    public const uint CurrentVersion = 1;

    /// <summary>Largest settings file read, in bytes.</summary>
    private const long MaxSettingsBytes = 64 << 10;

    /// <summary>File extensions of sounds, as the scene's Audio Source takes them.</summary>
    public static readonly IReadOnlyList<string> SoundExtensions = ["wav", "ogg", "flac", "mp3", "aif", "aiff", "opus", "m4a", "aac"];

    /// <summary>What kind of file settings are kept for, by its extension; null for other files.</summary>
    public static Imported? KindOf(string path)
    {
        var extension = Paths.Extension(path);
        if (extension is null)
        {
            return null;
        }
        extension = Text.AsciiLower(extension);
        if (Loader.ModelExtensions.Contains(extension))
        {
            return Imported.Model;
        }
        if (Loader.ImageExtensions.Contains(extension))
        {
            return Imported.Image;
        }
        return SoundExtensions.Contains(extension) ? Imported.Sound : null;
    }

    /// <summary>The settings file beside <paramref name="file"/>.</summary>
    public static string SettingsPath(string file) => $"{file}.{SettingsExtension}";

    /// <summary>Whether <paramref name="path"/> is a settings file.</summary>
    public static bool IsSettingsFile(string path) => Paths.Extension(path) is { } e && string.Equals(Text.AsciiLower(e), SettingsExtension, StringComparison.Ordinal);

    /// <summary>The defaults for <paramref name="file"/>, or null for a kind of file without settings.</summary>
    public static ImportSettings? DefaultsFor(string file)
    {
        if (KindOf(file) is not { } kind)
        {
            return null;
        }
        return new(FormatName, CurrentVersion, kind == Imported.Model ? ModelImport.Default : null, kind == Imported.Image ? ImageImport.Default : null,
            kind == Imported.Sound ? SoundImport.Default : null);
    }

    /// <summary>Checks the format and every value.</summary>
    public Result<Unit, string> Validate()
    {
        if (Format != FormatName)
        {
            return Result.Fail($"not import settings: the format is {Text.Debug(Format)}, not {Text.Debug(FormatName)}");
        }
        if (Version > CurrentVersion)
        {
            return Result.Fail($"import settings version {Version} is newer than this engine reads ({CurrentVersion})");
        }
        if (Model is { } model && !(float.IsFinite(model.Scale) && model.Scale >= 0.0001f && model.Scale <= 10_000f))
        {
            return Result.Fail("a model's scale must be between 0.0001 and 10000");
        }
        return Unit.Value;
    }

    public static Result<ImportSettings, string> FromText(string text)
    {
        var read = JsonReader.Parse(text).TryGet(out var value, out var parseError)
            ? JsonRead.Try(() =>
            {
                var f = JsonFields.Of(value, "struct ImportSettings", true, "format", "version", "model", "image", "sound");
                return new ImportSettings(f.Required("format", JsonRead.Str), f.Required("version", JsonRead.U32), f.Nullable("model", ModelImport.Read),
                    f.Nullable("image", ImageImport.Read), f.Nullable("sound", SoundImport.Read));
            })
            : Result.Fail(parseError);
        if (!read.TryGet(out var settings, out var error))
        {
            return Result.Fail($"broken import settings: {error}");
        }
        var valid = settings.Validate();
        return valid.IsOk ? settings : valid.AsFailure();
    }

    public string ToText()
    {
        var o = new JsonObject { { "format", new JsonString(Format) }, { "version", JsonNumber.Of((ulong)Version) } };
        if (Model is not null)
        {
            o.Add("model", Model.Write());
        }
        if (Image is not null)
        {
            o.Add("image", Image.Write());
        }
        if (Sound is not null)
        {
            o.Add("sound", Sound.Write());
        }
        return o.ToPretty() + "\n";
    }

    /// <summary>
    /// The settings saved beside <paramref name="file"/>, or its defaults when there are none. An
    /// error when the settings file is there but broken, so a mistake there is not silently ignored.
    /// </summary>
    public static Result<ImportSettings?, string> ReadFor(string file)
    {
        var path = SettingsPath(file);
        var size = Files.Length(path);
        if (size.IsError)
        {
            return DefaultsFor(file);
        }
        if (size.Value > MaxSettingsBytes)
        {
            return Result.Fail($"{path} is too large for import settings");
        }
        var text = Files.ReadText(path);
        if (text.IsError)
        {
            return Result.Fail($"cannot read {path}: {text.Error}");
        }
        var settings = FromText(text.Value);
        return settings.IsOk ? settings.Value : Result.Fail($"{path}: {settings.Error}");
    }
}

/// <summary>Paths taken apart as the Rust engine's paths are.</summary>
internal static class Paths
{
    /// <summary>The file name: what follows the last separator.</summary>
    public static string FileName(string path)
    {
        var at = path.LastIndexOfAny(['/', '\\']);
        return at < 0 ? path : path[(at + 1)..];
    }

    /// <summary>The extension, as Rust's <c>Path::extension</c> gives it: none for a name without a dot or one that starts with its only dot.</summary>
    public static string? Extension(string path)
    {
        var name = FileName(path);
        if (name is "" or "." or "..")
        {
            return null;
        }
        var dot = name.LastIndexOf('.');
        return dot <= 0 ? null : name[(dot + 1)..];
    }

    /// <summary>The folder a file is in; "." for a bare name, as the Rust loaders read <c>parent()</c>.</summary>
    public static string Folder(string path)
    {
        var at = path.LastIndexOfAny(['/', '\\']);
        return at < 0 ? "" : path[..at];
    }

    /// <summary><paramref name="name"/> inside <paramref name="folder"/>.</summary>
    public static string Join(string folder, string name) => folder.Length == 0 ? name : Path.Join(folder, name);
}

using System.Numerics;
using System.Text;

using Bayleaf.Core;

namespace Bayleaf.Assets;

/// <summary>
/// L2: reading asset files: images (PNG, JPEG, OpenEXR) and 3D models (glTF 2.0, OBJ).
/// Loaders turn files into plain data the renderer uploads. Each result carries an id that changes
/// when the file changes, so caches know when to load again. Limits keep a broken or hostile file
/// from using all memory. Import settings beside a file (<see cref="ImportSettings"/>) say how to
/// read it, and are part of what makes its id change.
/// </summary>
public static class Loader
{
    /// <summary>Largest image side, in pixels.</summary>
    public const int MaxImageSide = 8192;

    /// <summary>Most vertices in one model.</summary>
    public const int MaxVertices = 2_000_000;

    /// <summary>Largest model or image file, in bytes.</summary>
    public const long MaxFileBytes = 256L << 20;

    /// <summary>File extensions of the images this crate reads.</summary>
    public static readonly IReadOnlyList<string> ImageExtensions = ["png", "jpg", "jpeg", "exr"];

    /// <summary>File extensions of the models this crate reads.</summary>
    public static readonly IReadOnlyList<string> ModelExtensions = ["glb", "gltf", "obj"];

    /// <summary>
    /// Block-compresses <paramref name="image"/> (and its smaller copies when it has mipmaps). Its
    /// sides must be multiples of 4.
    /// </summary>
    public static Result<Blocks, string> Compress(Image image, Compression compression)
    {
        if (compression == Compression.None)
        {
            return Result.Fail("no compression asked for");
        }
        if (image.Width % 4 != 0 || image.Height % 4 != 0)
        {
            return Result.Fail($"block compression needs sides that are multiples of 4; this image is {image.Width} x {image.Height}");
        }
        var sizes = new List<MipLevel> { new(image.Width, image.Height, image.Rgba) };
        if (image.Mipmaps)
        {
            sizes.AddRange(MipLevels(image));
        }
        var blockBytes = compression == Compression.Bc1 ? 8 : 16;
        var levels = new List<byte[]>(sizes.Count);
        foreach (var (width, height, pixels) in sizes)
        {
            if (pixels.Length != width * height * 4)
            {
                return Result.Fail($"{pixels.Length} bytes of pixels for a {width} x {height} image");
            }
            var blocks = BlockCompression.Encode(width, height, pixels, compression == Compression.Bc1);
            var expected = (width + 3) / 4 * ((height + 3) / 4) * blockBytes;
            if (blocks.Length != expected)
            {
                return Result.Fail($"the compressed image has {blocks.Length} bytes, not {expected}");
            }
            levels.Add(blocks);
        }
        return new Blocks(compression, levels);
    }

    /// <summary>When the import settings beside <paramref name="path"/> last changed, if it has them.</summary>
    internal static DateTime? SettingsModified(string path) => Files.Modified(ImportSettings.SettingsPath(path));

    /// <summary>An id for the file as it is now: its path and when it and its settings last changed. Never 0, which is kept for "no asset".</summary>
    internal static ulong FileId(string path, DateTime? modified)
    {
        var hash = 0xcbf29ce484222325UL;
        void Add(ReadOnlySpan<byte> bytes)
        {
            foreach (var b in bytes)
            {
                hash = (hash ^ b) * 0x100000001b3UL;
            }
        }
        void AddTime(DateTime? time)
        {
            Add(time is null ? [0] : [1]);
            Add(BitConverter.GetBytes(time?.Ticks ?? 0));
        }
        Add(Encoding.UTF8.GetBytes(path));
        AddTime(modified);
        AddTime(SettingsModified(path));
        return hash | 1;
    }

    /// <summary>The file's bytes and when it last changed, refusing files over <see cref="MaxFileBytes"/>.</summary>
    internal static Result<(byte[] Bytes, DateTime? Modified), string> ReadLimited(string path)
    {
        var size = Files.Length(path);
        if (size.IsError)
        {
            return Result.Fail($"cannot read {path}: {size.Error}");
        }
        if (size.Value > MaxFileBytes)
        {
            return Result.Fail($"{path} is larger than {MaxFileBytes >> 20} MiB");
        }
        var bytes = Files.Read(path);
        if (bytes.IsError)
        {
            return Result.Fail($"cannot read {path}: {bytes.Error}");
        }
        return (bytes.Value, Files.Modified(path));
    }

    /// <summary>Reads a PNG, JPEG or OpenEXR file. The kind comes from the file's first bytes, not its name. Its import settings apply.</summary>
    public static Result<Image, string> LoadImage(string path)
    {
        var settings = ImportSettings.ReadFor(path);
        if (settings.IsError)
        {
            return settings.AsFailure();
        }
        var read = ReadLimited(path);
        if (read.IsError)
        {
            return read.AsFailure();
        }
        var decoded = DecodeImage(read.Value.Bytes);
        if (decoded.IsError)
        {
            return Result.Fail($"{path}: {decoded.Error}");
        }
        var options = settings.Value?.Image ?? ImageImport.Default;
        var image = decoded.Value with { Id = FileId(path, read.Value.Modified), Mipmaps = options.Mipmaps, Data = options.ColorSpace == ColorSpace.Data };
        if (options.Compression != Compression.None)
        {
            // asset.importSettings.set refuses sides that cannot be compressed; a file changed since
            // keeps its pixels as they are (its asset info says so).
            var blocks = Compress(image, options.Compression);
            image = image with { Blocks = blocks.IsOk ? blocks.Value : null };
        }
        return image;
    }

    /// <summary>
    /// The smaller copies of an image, each half the size of the one before (rounded down, at least
    /// 1), down to 1 x 1, not counting the image itself. Colours are averaged as light (in linear
    /// space, not as sRGB numbers), so a fine black and white pattern turns light grey, not dark
    /// grey; a data image's numbers are averaged as they are.
    /// </summary>
    public static List<MipLevel> MipLevels(Image image)
    {
        var toLinear = new float[256];
        for (var v = 0; v < 256; v++)
        {
            toLinear[v] = image.Data ? v / 255f : FMath.SrgbToLinear(v / 255f);
        }
        float Back(float v) => image.Data ? FMath.Clamp(v, 0f, 1f) : FMath.LinearToSrgb(v);
        var levels = new List<MipLevel>();
        var (w, h, src) = (Math.Max(image.Width, 1), Math.Max(image.Height, 1), image.Rgba);
        Span<float> sum = stackalloc float[4];
        while (w > 1 || h > 1)
        {
            var (nw, nh) = (Math.Max(w / 2, 1), Math.Max(h / 2, 1));
            var output = new byte[nw * nh * 4];
            for (var y = 0; y < nh; y++)
            {
                for (var x = 0; x < nw; x++)
                {
                    sum.Clear();
                    for (var k = 0; k < 4; k++)
                    {
                        var (dx, dy) = (k & 1, k >> 1);
                        var (sx, sy) = (Math.Min(x * 2 + dx, w - 1), Math.Min(y * 2 + dy, h - 1));
                        var at = (sy * w + sx) * 4;
                        for (var c = 0; c < 3; c++)
                        {
                            sum[c] += toLinear[src[at + c]];
                        }
                        sum[3] += src[at + 3] / 255f;
                    }
                    var to = (y * nw + x) * 4;
                    for (var c = 0; c < 3; c++)
                    {
                        output[to + c] = FMath.ToByte(FMath.Round(Back(sum[c] / 4f) * 255f));
                    }
                    output[to + 3] = FMath.ToByte(FMath.Round(sum[3] / 4f * 255f));
                }
            }
            levels.Add(new(nw, nh, output));
            (w, h, src) = (nw, nh, output);
        }
        return levels;
    }

    /// <summary>The first bytes of an OpenEXR file.</summary>
    internal static ReadOnlySpan<byte> ExrMagic => [0x76, 0x2f, 0x31, 0x01];

    /// <summary>Decodes PNG, JPEG or OpenEXR bytes into RGBA. The id is left at 0.</summary>
    public static Result<Image, string> DecodeImage(ReadOnlySpan<byte> bytes)
    {
        if (bytes.StartsWith((ReadOnlySpan<byte>)[0xFF, 0xD8, 0xFF]))
        {
            return DecodeJpeg(bytes);
        }
        return bytes.StartsWith(ExrMagic) ? DecodeExr(bytes) : DecodePng(bytes);
    }

    /// <summary>Decodes PNG bytes into RGBA. The id is left at 0.</summary>
    public static Result<Image, string> DecodePng(ReadOnlySpan<byte> bytes) => Png.Decode(bytes);

    /// <summary>Decodes JPEG bytes into RGBA (JPEG has no transparency, so alpha is 255). The id is left at 0.</summary>
    public static Result<Image, string> DecodeJpeg(ReadOnlySpan<byte> bytes) => Jpeg.Decode(bytes);

    /// <summary>
    /// An OpenEXR picture's colour: the part with the most colour channels, its R, G, B and A (or Y
    /// as grey). Its values are linear light; like every image here it is kept as 8-bit sRGB, so
    /// values above 1 are clipped to white.
    /// </summary>
    public static Result<Image, string> DecodeExr(ReadOnlySpan<byte> bytes) => Exr.DecodeImage(bytes);

    internal static string ExtensionOf(string path) => Text.AsciiLower(Paths.Extension(path) ?? "");

    /// <summary>
    /// The files a model names beside it (glTF buffers and images, OBJ material libraries and
    /// images), relative to its folder, each once, in the order the model names them. A <c>.glb</c>
    /// with everything inside names none.
    /// </summary>
    public static Result<List<string>, string> ModelFiles(string path)
    {
        if (Neutral.Reads(ExtensionOf(path)))
        {
            return Neutral.ModelFiles(path);
        }
        var read = ReadLimited(path);
        if (read.IsError)
        {
            return read.AsFailure();
        }
        var parsed = Gltf.Document.Parse(read.Value.Bytes);
        if (parsed.IsError)
        {
            return Result.Fail($"{path}: not a glTF 2.0 model: {parsed.Error}");
        }
        // An image inside a buffer view names no file, even when it also gives a uri.
        var uris = parsed.Value.Buffers.Select(b => b.Uri).Concat(parsed.Value.Images.Where(i => i.View is null).Select(i => i.Uri)).OfType<string>();
        var files = new List<string>();
        foreach (var uri in uris.Where(u => !u.StartsWith("data:", StringComparison.Ordinal)))
        {
            if (BufferPath("", uri) is not { } file)
            {
                return Result.Fail($"{path}: {uri} must be a file next to the model");
            }
            if (!files.Contains(file, StringComparer.Ordinal))
            {
                files.Add(file);
            }
        }
        return files;
    }

    /// <summary>
    /// Reads a model: glTF 2.0 (a <c>.glb</c> file, or a <c>.gltf</c> file with its buffers in files
    /// beside it), or Wavefront OBJ with its material libraries. Its import settings apply: the
    /// scale, and turning a Z-up model to Y up.
    /// </summary>
    public static Result<MeshData, string> LoadModel(string path) => Neutral.Reads(ExtensionOf(path)) ? Neutral.LoadModel(path) : Gltf.Loader.Load(path);

    /// <summary>A file named by <paramref name="uri"/>, only directly beside the model or below it.</summary>
    internal static string? BufferPath(string folder, string uri)
    {
        if (uri.Length > 0 && (uri[0] is '/' or '\\' || (uri.Length >= 2 && uri[1] == ':')))
        {
            return null;
        }
        var parts = uri.Split('/', '\\');
        for (var i = 0; i < parts.Length; i++)
        {
            // Rust's path components: a leading "." and any ".." are not plain names; empty and inner "." parts are dropped.
            if (parts[i] == ".." || (i == 0 && parts[i] == "."))
            {
                return null;
            }
        }
        return Paths.Join(folder, uri);
    }

    /// <summary>The same image read as plain numbers (a model's normal or metallic and roughness image).</summary>
    internal static Image AsData(Image image) =>
        // Its own id: the same file may also be shown as a colour.
        image.Data ? image : image with { Data = true, Id = image.Id ^ 0x2 };

    /// <summary>The id of a model's image <paramref name="index"/>: distinct from the model's id and from its other images.</summary>
    internal static ulong ImageId(ulong modelId, int index) => (modelId ^ unchecked((ulong)(index + 1) * 0x9E3779B97F4A7C15UL)) | 1;

    /// <summary>Each vertex gets the normal of the last triangle that uses it; for models without normals.</summary>
    internal static Vector3[] FlatNormals(IReadOnlyList<Vector3> positions, IReadOnlyList<uint> indices)
    {
        var normals = new Vector3[positions.Count];
        Array.Fill(normals, Vector3.UnitY);
        for (var t = 0; t + 2 < indices.Count; t += 3)
        {
            var (a, b, c) = (positions[(int)indices[t]], positions[(int)indices[t + 1]], positions[(int)indices[t + 2]]);
            var n = FMath.NormalizeOr(Vector3.Cross(b - a, c - a), Vector3.UnitY);
            normals[indices[t]] = n;
            normals[indices[t + 1]] = n;
            normals[indices[t + 2]] = n;
        }
        return normals;
    }
}

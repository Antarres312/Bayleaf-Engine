using System.Globalization;
using System.Numerics;
using System.Text;

namespace Bayleaf.Assets.Tests;

/// <summary>
/// Every model and image of the examples and of <c>tests/files</c> reads as the Rust engine reads
/// it. The Rust engine's test (<c>rust/crates/assets/src/shared_files.rs</c>) writes a summary of
/// what it reads from each into <c>tests/files/expected/&lt;name&gt;.summary</c>; this test writes
/// the same summary from the C# reader and compares, numbers within a small tolerance and the
/// pixels of JPEG and OpenEXR images within the tolerance the line names. Import settings files
/// are read and written back and compared byte for byte.
/// </summary>
public class SharedFilesTests
{
    internal static string RepositoryRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    internal static string Fnv(ReadOnlySpan<byte> bytes)
    {
        var hash = 0xcbf29ce484222325UL;
        foreach (var b in bytes)
        {
            hash = (hash ^ b) * 0x100000001b3UL;
        }
        return hash.ToString("x16", CultureInfo.InvariantCulture);
    }

    private static string Num(float v) =>
        float.IsNaN(v) ? "NaN" : float.IsInfinity(v) ? (v > 0 ? "inf" : "-inf") : v.ToString("F4", CultureInfo.InvariantCulture);

    private static string Nums(params float[] values) => string.Join(' ', values.Select(Num));

    private static string Nums(Vector2 v) => Nums(v.X, v.Y);

    private static string Nums(Vector3 v) => Nums(v.X, v.Y, v.Z);

    private static string Nums(Vector4 v) => Nums(v.X, v.Y, v.Z, v.W);

    private static string Nums(Matrix4x4 m) => Nums(m.M11, m.M12, m.M13, m.M14, m.M21, m.M22, m.M23, m.M24, m.M31, m.M32, m.M33, m.M34, m.M41, m.M42, m.M43, m.M44);

    private static string B(bool v) => v ? "true" : "false";

    private static string KindOf(byte[] bytes) =>
        bytes.AsSpan().StartsWith((ReadOnlySpan<byte>)[0xFF, 0xD8, 0xFF]) ? "jpeg" : bytes.AsSpan().StartsWith((ReadOnlySpan<byte>)[0x76, 0x2f, 0x31, 0x01]) ? "exr" : "png";

    private static string Picture(Image? i) => i is null ? "none" : $"{i.Width}x{i.Height} data={B(i.Data)} {Fnv(i.Rgba)}";

    private static string Material(ModelMaterial? m) => m is null
        ? "none"
        : $"base {Nums(m.BaseColor)} metallic {Num(m.Metallic)} roughness {Num(m.Roughness)} image {Picture(m.Image)} metal-roughness {Picture(m.MetalRoughness)} normal {Picture(m.Normal)}";

    private static string ImageSummary(string path)
    {
        var loaded = Loader.LoadImage(path);
        if (loaded.IsError)
        {
            return "refused\n";
        }
        var image = loaded.Value;
        var kind = KindOf(File.ReadAllBytes(path));
        var s = new StringBuilder();
        s.Append(CultureInfo.InvariantCulture, $"image\nformat {kind}\nsize {image.Width} {image.Height}\nmipmaps {B(image.Mipmaps)}\ndata {B(image.Data)}\n");
        if (kind == "png")
        {
            s.Append(CultureInfo.InvariantCulture, $"rgba {Fnv(image.Rgba)}\n");
        }
        else
        {
            s.Append(CultureInfo.InvariantCulture, $"rgba~{(kind == "jpeg" ? 3 : 1)} {Convert.ToHexStringLower(image.Rgba)}\n");
        }
        if (image.Blocks is { } blocks)
        {
            s.Append(CultureInfo.InvariantCulture, $"blocks {blocks.Compression.ToString().ToLowerInvariant()} {blocks.Levels.Count} {Fnv([.. blocks.Levels.SelectMany(l => l)])}\n");
        }
        else
        {
            s.Append("blocks none\n");
        }
        return s.ToString();
    }

    /// <summary>A rotation with the sign that makes w positive (q and -q are the same turn).</summary>
    private static Vector4 Turn(Quaternion q)
    {
        var first = new[] { q.X, q.Y, q.Z }.FirstOrDefault(v => v != 0);
        var flip = q.W < 0 || (q.W == 0 && first < 0);
        return flip ? new(-q.X, -q.Y, -q.Z, -q.W) : new(q.X, q.Y, q.Z, q.W);
    }

    private static void SkeletonSummary(StringBuilder s, MeshData mesh, Skeleton skeleton)
    {
        var c = CultureInfo.InvariantCulture;
        s.Append(c, $"skeleton nodes {skeleton.Nodes.Count} bindings {skeleton.Bindings.Count} animations {skeleton.Animations.Count}\n");
        for (var i = 0; i < skeleton.Nodes.Count; i++)
        {
            var n = skeleton.Nodes[i];
            s.Append(c, $"node {i} {skeleton.Names[i]} parent {(n.Parent is { } p ? p.ToString(c) : "-")} t {Nums(n.Translation)} r {Nums(Turn(n.Rotation))} s {Nums(n.Scale)}\n");
        }
        s.Append(c, $"order {string.Join(' ', skeleton.Order)}\n");
        s.Append(c, $"root {Nums(skeleton.Root)}\n");
        for (var i = 0; i < skeleton.Bindings.Count; i++)
        {
            s.Append(c, $"binding {i} node {skeleton.Bindings[i].Node} inverse {Nums(skeleton.Bindings[i].InverseBind)}\n");
        }
        for (var i = 0; i < skeleton.BindingBounds.Count; i++)
        {
            s.Append(c, $"bounds {i} {Nums(skeleton.BindingBounds[i].Min)} {Nums(skeleton.BindingBounds[i].Max)}\n");
        }
        for (var i = 0; i < skeleton.Positions.Count; i++)
        {
            var b = skeleton.VertexBindings[i];
            s.Append(c, $"bound {i} {b.B0} {b.B1} {b.B2} {b.B3} {Nums(skeleton.VertexWeights[i])} {Nums(skeleton.Positions[i])} {Nums(skeleton.Normals[i])}\n");
        }
        for (var index = 0; index < skeleton.Animations.Count; index++)
        {
            var a = skeleton.Animations[index];
            s.Append(c, $"animation {a.Name} duration {Num(a.Duration)} channels {a.Channels.Count}\n");
            foreach (var ch in a.Channels)
            {
                s.Append(c, $"channel node {ch.Node} {ch.Property.ToString().ToLowerInvariant()} {(ch.Step ? "step" : "linear")} keys {ch.Times.Count}\n");
                for (var k = 0; k < ch.Times.Count; k++)
                {
                    s.Append(c, $"key {Num(ch.Times[k])} {Nums(ch.Values[k])}\n");
                }
            }
            var time = a.Duration * 0.5f;
            var posed = skeleton.Posed(mesh, index, time, false, mesh.Id, 1);
            s.Append(c, $"posed {a.Name} {Num(time)} min {Nums(posed.Min)} max {Nums(posed.Max)}\n");
            for (var i = 0; i < posed.Positions.Count; i++)
            {
                s.Append(c, $"pv {i} {Nums(posed.Positions[i])} {Nums(posed.Normals[i])}\n");
            }
        }
    }

    internal static string ModelSummary(string path)
    {
        var loaded = Loader.LoadModel(path);
        if (loaded.IsError)
        {
            return "refused\n";
        }
        var mesh = loaded.Value;
        var c = CultureInfo.InvariantCulture;
        var s = new StringBuilder("model\n");
        var files = Loader.ModelFiles(path).ValueOr([]);
        s.Append(c, $"files {files.Count}\n");
        foreach (var f in files)
        {
            s.Append(c, $"file {f.Replace('\\', '/')}\n");
        }
        var indexBytes = mesh.Indices.SelectMany(BitConverter.GetBytes).ToArray();
        s.Append(c, $"vertices {mesh.Positions.Count}\nindices {mesh.Indices.Count} {Fnv(indexBytes)}\n");
        s.Append(c, $"min {Nums(mesh.Min)}\nmax {Nums(mesh.Max)}\n");
        for (var i = 0; i < mesh.Positions.Count; i++)
        {
            s.Append(c, $"v {i} {Nums(mesh.Positions[i])} {Nums(mesh.Normals[i])} {Nums(mesh.Uvs[i])}\n");
        }
        s.Append(c, $"parts {mesh.Parts.Count}\n");
        foreach (var p in mesh.Parts)
        {
            s.Append(c, $"part {p.First} {p.Count} {Material(p.Material)}\n");
        }
        s.Append(c, $"material {Material(mesh.Material)}\n");
        s.Append(c, $"warnings {mesh.Warnings.Count}\n");
        foreach (var w in mesh.Warnings)
        {
            s.Append(c, $"warning {w}\n");
        }
        if (mesh.Skeleton is { } skeleton)
        {
            SkeletonSummary(s, mesh, skeleton);
        }
        else
        {
            s.Append("skeleton none\n");
        }
        return s.ToString();
    }

    /// <summary>What the C# engine reads from <paramref name="path"/>, or null for a kind of file it is not asked about.</summary>
    private static string? Written(string path)
    {
        var extension = (Path.GetExtension(path).TrimStart('.')).ToLowerInvariant();
        if (extension == ImportSettings.SettingsExtension)
        {
            var settings = ImportSettings.FromText(File.ReadAllText(path));
            return settings.IsOk ? settings.Value.ToText() : "refused\n";
        }
        if (Loader.ModelExtensions.Contains(extension))
        {
            return ModelSummary(path);
        }
        return Loader.ImageExtensions.Contains(extension) ? ImageSummary(path) : null;
    }

    /// <summary>Whether two summaries say the same: word by word, numbers within a small tolerance, an <c>rgba~N</c> line's bytes each within N.</summary>
    internal static string? Differences(string written, string expected)
    {
        var (a, b) = (written.Split('\n'), expected.Split('\n'));
        if (a.Length != b.Length)
        {
            return $"{a.Length} lines, not {b.Length}";
        }
        for (var n = 0; n < a.Length; n++)
        {
            var (xs, ys) = (a[n].Split(' '), b[n].Split(' '));
            if (xs.Length == 2 && ys.Length == 2 && xs[0] == ys[0] && xs[0].StartsWith("rgba~", StringComparison.Ordinal))
            {
                var tolerance = int.Parse(xs[0]["rgba~".Length..], CultureInfo.InvariantCulture);
                var (p, q) = (Convert.FromHexString(xs[1]), Convert.FromHexString(ys[1]));
                var worst = p.Zip(q).Select(t => Math.Abs(t.First - t.Second)).DefaultIfEmpty(0).Max();
                if (p.Length != q.Length || worst > tolerance)
                {
                    return $"line {n + 1}: pixels differ by up to {worst} (or in number), more than {tolerance}";
                }
                continue;
            }
            static bool Close(string p, string q) =>
                p == q || ((p.Contains('.', StringComparison.Ordinal) || q.Contains('.', StringComparison.Ordinal))
                    && double.TryParse(p, NumberStyles.Float, CultureInfo.InvariantCulture, out var u) && double.TryParse(q, NumberStyles.Float, CultureInfo.InvariantCulture, out var v)
                    && Math.Abs(u - v) <= 2e-4 + 2e-5 * Math.Abs(u));
            if (xs.Length != ys.Length || xs.Zip(ys).Any(t => !Close(t.First, t.Second)))
            {
                return $"line {n + 1}: `{a[n]}` is not `{b[n]}`";
            }
        }
        return null;
    }

    public static TheoryData<string> SharedFiles()
    {
        var root = RepositoryRoot();
        var files = new TheoryData<string>();
        foreach (var dir in new[] { Path.Join(root, "examples"), Path.Join(root, "tests", "files") })
        {
            foreach (var file in Directory.EnumerateFiles(dir, "*", SearchOption.AllDirectories).Order(StringComparer.Ordinal))
            {
                var relative = Path.GetRelativePath(root, file);
                var extension = Path.GetExtension(file).TrimStart('.').ToLowerInvariant();
                var asked = extension == ImportSettings.SettingsExtension || Loader.ModelExtensions.Contains(extension) || Loader.ImageExtensions.Contains(extension);
                if (asked && !relative.Split(Path.DirectorySeparatorChar).Contains("expected"))
                {
                    files.Add(relative.Replace(Path.DirectorySeparatorChar, '/'));
                }
            }
        }
        return files;
    }

    private static string Expected(string name) => File.ReadAllText(Path.Join(RepositoryRoot(), "tests", "files", "expected", name)).ReplaceLineEndings("\n");

    [Fact]
    public void TheSharedFilesAreThere() => Assert.True(SharedFiles().Count > 100);

    [Theory]
    [MemberData(nameof(SharedFiles))]
    public void EachFileReadsAsTheRustEngineReadsIt(string relative)
    {
        var root = RepositoryRoot();
        var name = string.Join("__", relative.Split('/').SkipWhile(p => p is "tests" or "files"));
        var written = Written(Path.Join(root, relative))!;
        if (ImportSettings.IsSettingsFile(relative))
        {
            Assert.Equal(Expected(name), written);
            return;
        }
        var differences = Differences(written, Expected($"{name}.summary"));
        Assert.True(differences is null, $"{relative}: {differences}\n{written}");
    }

    [Fact]
    public void TheFixtureModelIsWrittenAsTheRustEngineWritesIt()
    {
        var dir = TestFiles.TempDir("shared-arm");
        var arm = Fixtures.SkinnedArm(dir).Value;
        Assert.Equal(Expected("fixtures__arm.gltf"), File.ReadAllText(arm));
        var summary = $"bin {Fnv(File.ReadAllBytes(Path.Join(dir, "arm.bin")))}\n{ModelSummary(arm)}";
        TestFiles.Remove(dir);
        var differences = Differences(summary, Expected("fixtures__arm.gltf.summary"));
        Assert.True(differences is null, differences);
    }

    [Theory]
    [InlineData("model", "a.glb")]
    [InlineData("image", "a.png")]
    [InlineData("sound", "a.ogg")]
    public void DefaultImportSettingsAreWrittenAsTheRustEngineWritesThem(string kind, string file) =>
        Assert.Equal(Expected($"defaults__{kind}.bimport"), ImportSettings.DefaultsFor(file)!.ToText());
}

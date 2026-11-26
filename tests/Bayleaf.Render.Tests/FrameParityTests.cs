using System.Numerics;

using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Render.Tests;

/// <summary>
/// Stage 3 of docs/csharp-redesign.md: each scene of the examples and <c>tests/files</c> extracts to the
/// frame the Rust engine extracts, seen as <c>scene.render</c> sees it (its first camera, else a view
/// framing it, at 16:9, with its files). The Rust test (<c>rust/crates/render/src/parity_tests.rs</c>)
/// writes <c>tests/files/expected/&lt;name&gt;.frame.json</c>; here every count, flag and name must be the
/// same and every number within a millionth of its size (or of 1, for small ones): both engines work in
/// single precision, but not always in the same order of operations.
/// </summary>
public class FrameParityTests
{
    private const string Refused = "refused\n";

    public static TheoryData<string> Scenes()
    {
        var root = SharedFilesTests.RepositoryRoot();
        var files = new TheoryData<string>();
        foreach (var dir in new[] { Path.Join(root, "examples"), Path.Join(root, "tests", "files") })
        {
            foreach (var file in Directory.EnumerateFiles(dir, "*.bscene", SearchOption.AllDirectories).Order(StringComparer.Ordinal))
            {
                var relative = Path.GetRelativePath(root, file);
                if (!relative.Split(Path.DirectorySeparatorChar).Contains("expected"))
                {
                    files.Add(relative.Replace(Path.DirectorySeparatorChar, '/'));
                }
            }
        }
        return files;
    }

    /// <summary>The folder a scene's paths start from: the project folder above it, else its own folder.</summary>
    internal static string AssetBase(string scene)
    {
        var own = Path.GetDirectoryName(scene) ?? ".";
        for (var dir = new DirectoryInfo(own); dir is not null; dir = dir.Parent)
        {
            if (dir.EnumerateFiles("*.bproject").Any())
            {
                return dir.FullName;
            }
        }
        return own;
    }

    private static JsonNumber N(float v) => JsonNumber.Of((double)v);

    private static JsonArray Floats(params float[] values) => new(values.Select(v => (JsonValue)N(v)));

    private static JsonArray V3(Vector3 v) => Floats(v.X, v.Y, v.Z);

    private static JsonArray V4(Vector4 v) => Floats(v.X, v.Y, v.Z, v.W);

    private static JsonArray M(Matrix4x4 m)
    {
        var values = new float[16];
        FMat.Write(values, m);
        return Floats(values);
    }

    private static JsonValue B(bool b) => JsonValue.From(b);

    private static JsonNumber U(long n) => JsonNumber.Of(n);

    private static JsonValue OrNull(JsonValue? value) => value ?? JsonValue.Null;

    /// <summary>The frame of a scene file, laid out as the Rust test lays it out; null for a file that does not load.</summary>
    private static JsonObject? Summary(string path)
    {
        if (!SceneModel.Load(path).TryGet(out var scene, out _))
        {
            return null;
        }
        const float Aspect = 16f / 9f;
        var camera = Extraction.SceneCamera(scene, Aspect) ?? Framing(scene, Aspect);
        var assets = FileAssets.Unconfined(AssetBase(path));
        var options = FrameOptions.Default with { Effects = Effects.Default with { Camera = Extraction.CameraObject(scene) } };
        var frame = Extraction.Extract(scene, camera, options, assets);
        var items = new JsonArray(frame.Items.Select(i => (JsonValue)new JsonObject
        {
            { "object", new JsonString(i.ObjectId.ToHex()) },
            { "shape", new JsonString(i.Shape.Code()) },
            { "model", M(i.Model) },
            { "color", V4(i.Color) },
            { "metallic", N(i.Metallic) },
            { "roughness", N(i.Roughness) },
            { "unlit", B(i.Unlit) },
            { "transparent", B(i.IsTransparent) },
            { "material", B(i.Material is not null) },
            { "mesh", OrNull(i.Mesh is { } m ? new JsonObject { { "vertices", U(m.Positions.Count) }, { "indices", U(m.Indices.Count) }, { "min", V3(m.Min) }, { "max", V3(m.Max) } } : null) },
            { "part", OrNull(i.Part is { } part ? new JsonArray([U(part.First), U(part.Count)]) : null) },
            { "textures", new JsonArray(i.Textures.Select(t => OrNull(t is null ? null : new JsonArray([U(t.Width), U(t.Height)])))) },
            { "additive", B(i.Additive) },
            { "uvRect", V4(i.UvRect) },
            { "skin", OrNull(i.Skin is { } s ? new JsonObject { { "matrices", U(s.Matrices.Count) }, { "min", V3(s.Min) }, { "max", V3(s.Max) } } : null) },
        }));
        var lights = new JsonArray(frame.Lights.Select(l => (JsonValue)new JsonObject
        {
            { "kind", new JsonString(l.Kind.ToString().ToLowerInvariant()) },
            { "position", V3(l.Position) },
            { "direction", V3(l.Direction) },
            { "radiance", V3(l.Radiance) },
            { "range", N(l.Range) },
            { "cosOuter", N(l.CosOuter) },
            { "cosInner", N(l.CosInner) },
            { "shadows", B(l.Shadows) },
        }));
        var p = frame.Post;
        var origin = frame.Camera.Origin;
        return new JsonObject
        {
            { "camera", new JsonObject { { "view", M(frame.Camera.View) }, { "proj", M(frame.Camera.Proj) }, { "position", V3(frame.Camera.Position) }, { "origin", new JsonArray([JsonNumber.Of(origin.X), JsonNumber.Of(origin.Y), JsonNumber.Of(origin.Z)]) } } },
            { "items", items },
            { "lights", lights },
            { "ambient", V3(frame.Ambient) },
            { "ambientGround", V3(frame.AmbientGround) },
            { "clearColor", V4(frame.ClearColor) },
            { "sky", OrNull(frame.Sky is { } sky ? new JsonObject { { "zenith", V3(sky.Zenith) }, { "horizon", V3(sky.Horizon) }, { "ground", V3(sky.Ground) } } : null) },
            { "background", B(frame.Background) },
            { "fog", Floats(frame.Fog.Color.X, frame.Fog.Color.Y, frame.Fog.Color.Z, frame.Fog.Density) },
            {
                "post", new JsonObject
                {
                    { "exposure", N(p.Exposure) }, { "toneMapping", B(p.ToneMapping) }, { "bloom", N(p.Bloom) }, { "bloomThreshold", N(p.BloomThreshold) },
                    { "contrast", N(p.Contrast) }, { "saturation", N(p.Saturation) }, { "lookup", OrNull(p.Lookup is { } l ? new JsonArray([U(l.Width), U(l.Height)]) : null) },
                    { "lookupStrength", N(p.LookupStrength) }, { "vignette", N(p.Vignette) }, { "vignetteSmoothness", N(p.VignetteSmoothness) },
                    { "fog", OrNull(p.Fog is { } f ? Floats(f.Color.X, f.Color.Y, f.Color.Z, f.Start, f.End) : null) },
                    { "occlusion", N(p.Occlusion) }, { "occlusionRadius", N(p.OcclusionRadius) }, { "antialiasing", new JsonString(p.Antialiasing.ToString().ToLowerInvariant()) },
                }
            },
            { "warnings", U(assets.Warnings.Count) },
        };
    }

    private static CameraView Framing(SceneModel scene, float aspect)
    {
        var (center, radius) = Extraction.Bounds(scene) ?? (Double3.Zero, 2f);
        return CameraView.Framing(center, radius, aspect);
    }

    /// <summary>Where <paramref name="actual"/> differs from <paramref name="expected"/>, numbers within the tolerance.</summary>
    private static void Compare(JsonValue expected, JsonValue actual, string at, List<string> differences)
    {
        switch (expected, actual)
        {
            case (JsonNumber e, JsonNumber a):
                var (x, y) = (e.ToDouble(), a.ToDouble());
                if (Math.Abs(x - y) > 1e-6 * Math.Max(1, Math.Max(Math.Abs(x), Math.Abs(y))))
                {
                    differences.Add($"{at}: {x} against {y}");
                }
                break;
            case (JsonArray e, JsonArray a) when e.Count == a.Count:
                for (var i = 0; i < e.Count; i++)
                {
                    Compare(e[i], a[i], $"{at}[{i}]", differences);
                }
                break;
            case (JsonObject e, JsonObject a) when e.Keys.Order(StringComparer.Ordinal).SequenceEqual(a.Keys.Order(StringComparer.Ordinal)):
                foreach (var (key, value) in e.Entries)
                {
                    Compare(value, a.Get(key)!, $"{at}.{key}", differences);
                }
                break;
            default:
                if (expected.ToCompact() != actual.ToCompact())
                {
                    differences.Add($"{at}: {expected.ToCompact()} against {actual.ToCompact()}");
                }
                break;
        }
    }

    [Fact]
    public void TheSharedScenesAreThere() => Assert.True(Scenes().Count > 5);

    [Theory]
    [MemberData(nameof(Scenes))]
    public void EachSceneExtractsToTheFrameTheRustEngineExtracts(string relative)
    {
        var name = string.Join("__", relative.Split('/').SkipWhile(p => p is "tests" or "files")) + ".frame.json";
        var expected = File.ReadAllText(Path.Join(SharedFilesTests.RepositoryRoot(), "tests", "files", "expected", name)).ReplaceLineEndings("\n");
        var summary = Summary(Path.Join(SharedFilesTests.RepositoryRoot(), relative));
        if (expected == Refused || summary is null)
        {
            Assert.Equal(expected, summary is null ? Refused : summary.ToPretty() + "\n");
            return;
        }
        var differences = new List<string>();
        Compare(JsonReader.Parse(expected).Value, summary, "frame", differences);
        Assert.True(differences.Count == 0, string.Join("\n", differences.Take(20)));
    }
}

using System.Globalization;

using Bayleaf.Assets;
using Bayleaf.Core.Json;

namespace Bayleaf.Engine.Tests;

/// <summary>
/// Stage 3's gate (docs/csharp-redesign.md): <c>scene.render</c> draws the shared scenes close to how the
/// Rust engine draws them. The Rust test (<c>the_shared_scenes_render_as_the_expected_pictures</c> in
/// <c>rust/crates/engine/src/parity_tests.rs</c>) writes its pictures to
/// <c>tests/files/expected/renders</c>; this one draws the same scenes, opened the same way, at the same
/// size, and compares each colour channel: its mean difference over every pixel and its largest one.
/// Without a GPU it skips, unless <c>BAYLEAF_GPU_REQUIRED=1</c>.
/// </summary>
public class RenderParityTests
{
    /// <summary>The scenes, each with the project it opens in (the Rust test's list).</summary>
    public static TheoryData<string, string?> Rendered() => new()
    {
        { "examples/demo/demo.bscene", null },
        { "examples/showcase/scenes/main.bscene", "examples/showcase" },
        { "tests/files/every-component.bscene", null },
        { "tests/files/old-environment.bscene", null },
        { "tests/files/version-1.bscene", null },
    };

    /// <summary>Pixels across and down of the compared pictures.</summary>
    private const int Width = 192;

    private const int Height = 108;

    /// <summary>
    /// How far the two engines' pictures of a scene may differ, per channel out of 255: on average over
    /// every pixel, and in the pixel that differs most (why these, in docs/csharp-redesign.md, stage 3).
    /// </summary>
    internal const double MeanLimit = 1.0;

    internal const int MaxLimit = 64;

    private static string Root()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    /// <summary>The mean and the largest difference of each colour channel between two RGBA pictures of one size.</summary>
    internal static (double[] Mean, int[] Max) Difference(byte[] a, byte[] b)
    {
        var (sum, max) = (new long[3], new int[3]);
        for (var p = 0; p + 3 < Math.Min(a.Length, b.Length); p += 4)
        {
            for (var c = 0; c < 3; c++)
            {
                var d = Math.Abs(a[p + c] - b[p + c]);
                sum[c] += d;
                max[c] = Math.Max(max[c], d);
            }
        }
        var pixels = Math.Max(a.Length / 4, 1);
        return ([.. sum.Select(s => (double)s / pixels)], max);
    }

    [Theory]
    [MemberData(nameof(Rendered))]
    public void EachSceneRendersCloseToTheRustEnginesPicture(string scene, string? project)
    {
        var root = Root();
        var registry = CommandRegistry.WithBuiltins();
        var session = new Session();
        if (project is not null)
        {
            Assert.True(registry.Execute(session, "project.open", new JsonObject { { "path", new JsonString(Path.Join(root, project)) } }).IsOk);
        }
        var name = scene.Replace("/", "__", StringComparison.Ordinal) + ".png";
        var output = Path.Join(Path.GetTempPath(), $"bayleaf-render-parity-cs-{System.Environment.ProcessId}-{Guid.NewGuid():N}");
        Directory.CreateDirectory(output);
        // A test file is opened from a copy: opening an old one moves its environment into a lighting
        // file beside it.
        var path = Path.Join(root, scene);
        if (scene.StartsWith("tests/", StringComparison.Ordinal))
        {
            File.Copy(path, Path.Join(output, Path.GetFileName(path)));
            path = Path.Join(output, Path.GetFileName(path));
        }
        var opened = registry.Execute(session, "scene.open", new JsonObject { { "path", new JsonString(path) } });
        Assert.True(opened.IsOk, opened.Error?.Message);
        var drawn = Path.Join(output, name);
        var result = registry.Execute(session, "scene.render", new JsonObject
        {
            { "path", new JsonString(drawn) },
            { "width", JsonNumber.Of((long)Width) },
            { "height", JsonNumber.Of((long)Height) },
        });
        if (result.IsError && result.Error.Message.Contains("no usable graphics adapter", StringComparison.Ordinal)
            && System.Environment.GetEnvironmentVariable("BAYLEAF_GPU_REQUIRED") is not { Length: > 0 })
        {
            Assert.Skip(result.Error.Message);
        }
        Assert.True(result.IsOk, result.Error?.Message);
        var now = Loader.LoadImage(drawn).Value;
        Directory.Delete(output, true);
        var expected = Loader.LoadImage(Path.Join(root, "tests", "files", "expected", "renders", name)).Value;
        Assert.Equal((expected.Width, expected.Height), (now.Width, now.Height));
        var (mean, max) = Difference(now.Rgba, expected.Rgba);
        var measured = string.Create(CultureInfo.InvariantCulture, $"{scene}: mean {string.Join(", ", mean.Select(m => m.ToString("0.000", CultureInfo.InvariantCulture)))}; largest {string.Join(", ", max)}");
        TestContext.Current.TestOutputHelper?.WriteLine(measured);
        Assert.True(mean.All(m => m <= MeanLimit) && max.All(m => m <= MaxLimit), measured);
    }
}

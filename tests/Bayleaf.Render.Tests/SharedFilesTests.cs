using Bayleaf.Core;

namespace Bayleaf.Render.Tests;

/// <summary>
/// Every material file of the examples, the benchmark and <c>tests/files</c> reads and writes back
/// as the Rust engine reads and writes it, with the material catalog, and compiles to the same WGSL
/// byte for byte (docs/csharp-redesign.md). The Rust engine's test
/// (<c>rust/crates/render/src/parity_tests.rs</c>) writes what it gives into <c>tests/files/expected</c>:
/// <c>&lt;name&gt;</c> the file written back, <c>&lt;name&gt;.wgsl</c> its surface function, and
/// <c>material.catalog.json</c> the catalog in every detail.
/// </summary>
public class SharedFilesTests
{
    private const string Refused = "refused\n";

    internal static string RepositoryRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    private static string Expected(string name) => File.ReadAllText(Path.Join(RepositoryRoot(), "tests", "files", "expected", name)).ReplaceLineEndings("\n");

    /// <summary>The file as the C# engine writes it back after reading it, and its surface function; <c>refused</c> for either when it does not read or compile.</summary>
    private static (string File, string Surface) Rewrite(string path)
    {
        if (!Files.ReadText(path).TryGet(out var text, out _) || !Graph.Graph.FromJson(text, Material.Catalog).TryGet(out var graph, out _))
        {
            return (Refused, Refused);
        }
        return (graph.ToJson(), Material.Compile(graph).TryGet(out var compiled, out _) ? compiled.Surface : Refused);
    }

    public static TheoryData<string> SharedFiles()
    {
        var root = RepositoryRoot();
        var files = new TheoryData<string>();
        foreach (var dir in new[] { Path.Join(root, "examples"), Path.Join(root, "rust", "bench", "graphs"), Path.Join(root, "tests", "files") })
        {
            foreach (var file in Directory.EnumerateFiles(dir, "*.bmaterial", SearchOption.AllDirectories).Order(StringComparer.Ordinal))
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

    [Fact]
    public void TheSharedFilesAreThere() => Assert.True(SharedFiles().Count > 10);

    [Theory]
    [MemberData(nameof(SharedFiles))]
    public void EachFileRewritesAndCompilesAsTheRustEngineDoes(string relative)
    {
        var name = string.Join("__", relative.Split('/').SkipWhile(p => p is "tests" or "files"));
        var (file, surface) = Rewrite(Path.Join(RepositoryRoot(), relative));
        Assert.Equal(Expected(name), file);
        Assert.Equal(Expected(name + ".wgsl"), surface);
    }

    [Fact]
    public void TheCatalogIsTheRustEnginesInEveryDetail() => Assert.Equal(Expected("material.catalog.json"), Material.Catalog.Spec().ToPretty() + "\n");
}

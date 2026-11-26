using Bayleaf.Core;

namespace Bayleaf.Graph.Tests;

/// <summary>
/// Every graph file of the examples, the benchmark and <c>tests/files</c> reads and writes back as
/// the Rust engine reads and writes it (docs/csharp-redesign.md). The Rust engine's test
/// (<c>rust/crates/graph/src/tests.rs</c>) writes what it gives into <c>tests/files/expected</c>;
/// this test checks the C# engine gives the same. Both read a file by the graph format's own rules,
/// with none of its domain's node kinds (those come with the projects that compile each domain),
/// so nodes and links are read and written but not checked against a catalog.
/// </summary>
public class SharedFilesTests
{
    private static readonly string[] Kinds = ["bmaterial", "bscript", "banim"];

    internal static string RepositoryRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    /// <summary>The catalog of a graph file's domain, by its extension, with none of the domain's node kinds.</summary>
    private static Catalog FormatCatalog(string extension) => extension switch
    {
        "bmaterial" => new() { Domain = "material", Label = "Material", Extension = extension },
        "bscript" => new() { Domain = "script", Label = "Visual Script", Extension = extension },
        _ => new() { Domain = "animation", Label = "Animation States", Extension = extension },
    };

    /// <summary>What the C# engine writes back after reading <paramref name="path"/> as <see cref="Graph.FromJson"/> does, its nodes and links aside, or <c>refused</c>.</summary>
    private static string Rewrite(string path)
    {
        const string refused = "refused\n";
        var catalog = FormatCatalog(Path.GetExtension(path).TrimStart('.'));
        if (!Files.ReadText(path).TryGet(out var text, out _) || !Graph.Parse(text).TryGet(out var graph, out _))
        {
            return refused;
        }
        var read = graph.Version;
        graph.Upgrade();
        if (read < Graph.CurrentVersion)
        {
            catalog.Upgrade(graph, read);
            graph.Version = Graph.CurrentVersion;
        }
        var declarations = graph.Clone();
        declarations.Nodes.Clear();
        declarations.Links.Clear();
        return declarations.Check(catalog).IsOk ? graph.ToJson() : refused;
    }

    public static TheoryData<string> SharedFiles()
    {
        var root = RepositoryRoot();
        var files = new TheoryData<string>();
        foreach (var dir in new[] { Path.Join(root, "examples"), Path.Join(root, "rust", "bench", "graphs"), Path.Join(root, "tests", "files") })
        {
            foreach (var file in Directory.EnumerateFiles(dir, "*", SearchOption.AllDirectories).Order(StringComparer.Ordinal))
            {
                var relative = Path.GetRelativePath(root, file);
                if (Kinds.Contains(Path.GetExtension(file).TrimStart('.')) && !relative.Split(Path.DirectorySeparatorChar).Contains("expected"))
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
    public void EachFileRewritesAsTheRustEngineRewritesIt(string relative)
    {
        var root = RepositoryRoot();
        var name = string.Join("__", relative.Split('/').SkipWhile(p => p is "tests" or "files"));
        var expected = File.ReadAllText(Path.Join(root, "tests", "files", "expected", name)).ReplaceLineEndings("\n");
        Assert.Equal(expected, Rewrite(Path.Join(root, relative)));
    }
}

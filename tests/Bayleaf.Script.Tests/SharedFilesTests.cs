using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Script.Tests;

/// <summary>
/// Every visual script file of the examples, the benchmark and <c>tests/files</c> reads and writes
/// back as the Rust engine reads and writes it, with the visual script catalog: node kinds, the pins
/// their settings give them, and the upgrade of older files (docs/csharp-redesign.md). The Rust
/// engine's test (<c>rust/crates/script/src/parity_tests.rs</c>) writes what it gives into
/// <c>tests/files/expected</c>, and <c>script.catalog.json</c>: the catalog in every detail, the
/// components visual scripts read, and the pins of sample nodes whose pins follow their settings.
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

    private static string Expected(string name) => File.ReadAllText(Path.Join(RepositoryRoot(), "tests", "files", "expected", name)).ReplaceLineEndings("\n");

    /// <summary>What the C# engine writes back after reading <paramref name="path"/> with the visual script catalog, or <c>refused</c>.</summary>
    private static string Rewrite(string path) =>
        Files.ReadText(path).TryGet(out var text, out _) && Graph.Graph.FromJson(text, Visual.Catalog).TryGet(out var graph, out _) ? graph.ToJson() : "refused\n";

    public static TheoryData<string> SharedFiles()
    {
        var root = RepositoryRoot();
        var files = new TheoryData<string>();
        foreach (var dir in new[] { Path.Join(root, "examples"), Path.Join(root, "rust", "bench", "graphs"), Path.Join(root, "tests", "files") })
        {
            foreach (var file in Directory.EnumerateFiles(dir, "*.bscript", SearchOption.AllDirectories).Order(StringComparer.Ordinal))
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
    public void EachFileRewritesAsTheRustEngineRewritesIt(string relative)
    {
        var name = string.Join("__", relative.Split('/').SkipWhile(p => p is "tests" or "files"));
        Assert.Equal(Expected(name), Rewrite(Path.Join(RepositoryRoot(), relative)));
    }

    /// <summary>The nodes whose pins follow their settings that the Rust engine's test describes, by its file.</summary>
    private static JsonArray Variants(JsonValue spec)
    {
        var made = new List<JsonValue>();
        foreach (var sample in (JsonArray)((JsonObject)spec).Find("variants")!)
        {
            var fields = (JsonObject)sample;
            var kind = ((JsonString)fields.Find("kind")!).Value;
            var values = (JsonObject)fields.Find("values")!;
            var graph = Graph.Graph.Empty(Visual.Catalog);
            var id = graph.AddNode(Visual.Catalog, kind, System.Numerics.Vector2.Zero).Value;
            foreach (var (name, value) in values)
            {
                Assert.True(graph.SetValue(Visual.Catalog, id, name, value).IsOk, $"{kind} takes {name}");
            }
            var def = graph.NodeWithDef(Visual.Catalog, id).TryGet(out var found, out _) ? found.Def.Spec() : JsonValue.Null;
            made.Add(new JsonObject { { "kind", new JsonString(kind) }, { "values", values }, { "def", def } });
        }
        return new JsonArray(made);
    }

    [Fact]
    public void TheCatalogIsTheRustEnginesInEveryDetail()
    {
        var expected = Expected("script.catalog.json");
        Assert.True(JsonReader.Parse(expected).TryGet(out var spec, out var error), error?.ToString());
        var components = ComponentNodes.All.Select(c => (JsonValue)new JsonObject
        {
            { "name", new JsonString(c.Name) },
            { "label", new JsonString(c.Label) },
            { "settable", JsonValue.From(c.Settable) },
            { "fields", new JsonArray(c.Fields.Select(f => (JsonValue)new JsonString(f.Name))) },
        });
        var made = new JsonObject { { "catalog", Visual.Catalog.Spec() }, { "components", new JsonArray(components) }, { "variants", Variants(spec) } };
        Assert.Equal(expected, made.Canonical().ToPretty() + "\n");
    }
}

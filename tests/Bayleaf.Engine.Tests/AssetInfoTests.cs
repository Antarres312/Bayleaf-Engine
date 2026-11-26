using Bayleaf.Core.Json;

namespace Bayleaf.Engine.Tests;

/// <summary><c>asset.info</c> and the graph kinds of <c>asset.create</c>, as the Rust engine's behaviour tests check them.</summary>
public class AssetInfoTests
{
    private readonly CommandRegistry registry = CommandRegistry.WithBuiltins();
    private readonly Session session = new();

    private static JsonObject Params(params (string Key, JsonValue Value)[] fields) => new(fields.Select(f => new KeyValuePair<string, JsonValue>(f.Key, f.Value)));

    private JsonValue Info(string path) => registry.Execute(session, "asset.info", Params(("path", new JsonString(path)))).Value;

    private static string? Row(JsonValue info, string label) =>
        info.Get("rows")!.AsArray()!.FirstOrDefault(r => r.AsArray()![0].AsString() == label)?.AsArray()![1].AsString();

    private static string RepositoryRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    private static string TempDir(string name)
    {
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-{name}-cs-{Environment.ProcessId}");
        if (Directory.Exists(dir))
        {
            Directory.Delete(dir, true);
        }
        return dir;
    }

    [Fact]
    public void FileInfoDescribesProjectFilesAndStaysInsideTheRoots()
    {
        var dir = TempDir("file-info");
        try
        {
            registry.Execute(session, "project.new", Params(("path", new JsonString(dir)))).OrThrow();
            var scene = Info("scenes/main.bscene");
            Assert.Equal("Scene", scene.Get("kind")!.AsString());
            Assert.Equal("4", Row(scene, "Objects"));
            var script = Info("scripts/spin.bscript");
            Assert.Equal("Visual Script", script.Get("kind")!.AsString());
            // Not compiled here, which it says, rather than that nothing is wrong.
            Assert.Contains("not compiled", Assert.Single(script.Get("problems")!.AsArray()!).AsString(), StringComparison.Ordinal);
            Assert.Contains("speed", Row(script, "Properties"), StringComparison.Ordinal);

            // A long text file is cut, and never inside a character.
            var longText = string.Concat(Enumerable.Repeat("ż", AssetInfoCommands.TextPreviewBytes));
            File.WriteAllText(Path.Join(dir, "notes.txt"), longText);
            var text = Info("notes.txt").Get("text")!.AsString()!;
            Assert.True(text.Length < longText.Length && text.EndsWith('…'));

            Assert.NotEmpty(Info("missing.png").Get("problems")!.AsArray()!);

            var inside = Path.Join(dir, "scenes");
            session.Confinement = Confinement.Create([inside], [inside]).Value;
            Assert.True(registry.Execute(session, "asset.info", Params(("path", new JsonString(Path.Join(dir, "README.md"))))).IsError, "outside the read roots");
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }

    [Fact]
    public void AWebAssemblyModuleIsNotDescribedAndSaysWhy()
    {
        var dir = TempDir("file-info-wasm");
        Directory.CreateDirectory(dir);
        try
        {
            File.WriteAllBytes(Path.Join(dir, "mover.wasm"), [0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
            var info = Info(Path.Join(dir, "mover.wasm"));
            Assert.Equal("Script", info.Get("kind")!.AsString());
            Assert.Contains("C#", Assert.Single(info.Get("problems")!.AsArray()!).AsString(), StringComparison.Ordinal);
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }

    [Fact]
    public void TheShowcasesModelsAndMaterialReadWithoutProblems()
    {
        var project = Path.Join(RepositoryRoot(), "examples", "showcase", "Showcase.bproject");
        registry.Execute(session, "project.open", Params(("path", new JsonString(project)))).OrThrow();
        foreach (var model in new[] { "models/windmill.gltf", "models/crate.obj", "materials/ground.bmaterial" })
        {
            Assert.Empty(Info(model).Get("problems")!.AsArray()!);
        }
        Assert.Contains("Spin", Info("models/windmill.gltf").ToCompact(), StringComparison.Ordinal);
    }

    [Fact]
    public void AssetCreateMakesMaterialsAndVisualScriptsThatOpenAndNeverWritesOverAFile()
    {
        var dir = TempDir("asset-create-graphs");
        try
        {
            registry.Execute(session, "project.new", Params(("path", new JsonString(dir)))).OrThrow();
            JsonValue Create(string kind, string folder, string name) =>
                registry.Execute(session, "asset.create", Params(("kind", new JsonString(kind)), ("folder", new JsonString(folder)), ("name", new JsonString(name)))).OrThrow();
            foreach (var (kind, name) in new[] { ("material", "Rock"), ("visualScript", "Door") })
            {
                var made = Create(kind, "assets", name);
                registry.Execute(session, "graph.open", Params(("path", made.Get("path")!))).OrThrow();
            }
            Assert.True(registry.Execute(session, "asset.create", Params(("kind", new JsonString("material")), ("folder", new JsonString("assets")), ("name", new JsonString("Rock")))).IsError, "an existing file is never written over");

            // Agents create only inside their write folders.
            var scenes = Path.Join(dir, "scenes");
            session.Confinement = Confinement.Create([dir], [scenes]).Value;
            Assert.True(registry.Execute(session, "asset.create", Params(("kind", new JsonString("material")), ("folder", new JsonString("assets")), ("name", new JsonString("Outside")))).IsError);
            Assert.False(File.Exists(Path.Join(dir, "assets", "Outside.bmaterial")));
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }
}

internal static class ResultExtensions
{
    public static JsonValue OrThrow(this Core.Result<JsonValue, CommandError> result) =>
        result.IsOk ? result.Value : throw new InvalidOperationException(result.Error.Message);
}

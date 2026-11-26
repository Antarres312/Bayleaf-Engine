namespace Bayleaf.Scene.Tests;

/// <summary>
/// Stage 1 of docs/csharp-redesign.md: every file of the examples and of <c>dotnet/tests/files</c> reads
/// and writes back as the Rust engine reads and writes it. The Rust engine's test
/// (<c>crates/scene/src/tests.rs</c>) writes what it gives into <c>dotnet/tests/files/expected</c> and
/// checks it still gives that; this test checks the C# engine gives the same.
/// </summary>
public class SharedFilesTests
{
    private static readonly string[] Kinds = ["bscene", "blight", "bpost", "bshape", "bdata", "btimeline"];

    internal static string RepositoryRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !(File.Exists(Path.Join(dir.FullName, "Cargo.toml")) && Directory.Exists(Path.Join(dir.FullName, "dotnet"))))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    /// <summary>What the C# engine writes back after reading <paramref name="path"/>, or <c>refused</c>.</summary>
    private static string Rewrite(string path)
    {
        const string refused = "refused\n";
        return Path.GetExtension(path).TrimStart('.') switch
        {
            "bscene" => Scene.Load(path).TryGet(out var scene, out _) ? scene.ToDoc().ToText() : refused,
            "blight" => Lighting.Load(path).TryGet(out var lighting, out _) ? lighting.ToText() : refused,
            "bpost" => PostProcessing.Load(path).TryGet(out var post, out _) ? post.ToText() : refused,
            "bshape" => DataShape.Load(path).TryGet(out var shape, out _) ? shape.ToText() : refused,
            "bdata" => DataFile.Load(path).TryGet(out var data, out _) ? data.ToText() : refused,
            _ => Timeline.FromJson(File.ReadAllText(path)).TryGet(out var timeline, out _) ? timeline.ToJson() + "\n" : refused,
        };
    }

    public static TheoryData<string> SharedFiles()
    {
        var root = RepositoryRoot();
        var files = new TheoryData<string>();
        foreach (var dir in new[] { Path.Join(root, "examples"), Path.Join(root, "dotnet", "tests", "files") })
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
    public void TheSharedFilesAreThere() => Assert.True(SharedFiles().Count > 20);

    [Theory]
    [MemberData(nameof(SharedFiles))]
    public void EachFileRewritesAsTheRustEngineRewritesIt(string relative)
    {
        var root = RepositoryRoot();
        var name = string.Join("__", relative.Split('/').SkipWhile(p => p is "dotnet" or "tests" or "files"));
        var expected = File.ReadAllText(Path.Join(root, "dotnet", "tests", "files", "expected", name)).ReplaceLineEndings("\n");
        Assert.Equal(expected, Rewrite(Path.Join(root, relative)));
    }
}

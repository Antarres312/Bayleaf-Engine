namespace Bayleaf.GameInterface.Tests;

/// <summary>
/// Every layout and style file of the examples and of <c>tests/files</c> reads and writes back as
/// the Rust engine reads and writes it (docs/csharp-redesign.md). The Rust engine's test
/// (<c>rust/crates/interface/src/tests.rs</c>) writes what it gives into <c>tests/files/expected</c>;
/// this test checks the C# engine gives the same.
/// </summary>
public class SharedFilesTests
{
    private static readonly string[] Kinds = ["bui", "bstyle"];

    internal static string RepositoryRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    /// <summary>What the C# engine writes back after reading <paramref name="path"/>, or <c>refused</c>.</summary>
    private static string Rewrite(string path)
    {
        const string refused = "refused\n";
        return Path.GetExtension(path) == ".bui"
            ? Layout.Load(path).TryGet(out var layout, out _) ? layout.ToText() : refused
            : StyleSheet.Load(path).TryGet(out var sheet, out _) ? sheet.ToText() : refused;
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

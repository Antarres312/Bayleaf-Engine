namespace Bayleaf.Assets.Tests;

/// <summary>
/// The readers' hostile-input sweep: every shared image and model, cut short and with bytes
/// changed, is read or refused with a reason, never an exception.
/// </summary>
public class HostileInputTests
{
    private static IEnumerable<string> Shared(params string[] extensions)
    {
        var root = SharedFilesTests.RepositoryRoot();
        return new[] { Path.Join(root, "examples"), Path.Join(root, "tests", "files") }
            .SelectMany(d => Directory.EnumerateFiles(d, "*", SearchOption.AllDirectories))
            .Where(f => !f.Contains($"{Path.DirectorySeparatorChar}expected{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            .Where(f => extensions.Contains(Path.GetExtension(f).TrimStart('.').ToLowerInvariant()))
            .Order(StringComparer.Ordinal);
    }

    /// <summary>The bytes cut at many places, and with one byte changed at many places.</summary>
    private static IEnumerable<byte[]> Broken(byte[] bytes, int cuts, int changes)
    {
        var step = Math.Max(1, bytes.Length / cuts);
        for (var cut = 0; cut < bytes.Length; cut += step)
        {
            yield return bytes[..cut];
        }
        var seed = 0x2545F491u;
        for (var i = 0; i < changes && bytes.Length > 0; i++)
        {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            var changed = bytes.ToArray();
            changed[(int)(seed % (uint)changed.Length)] ^= (byte)(seed >> 24 | 1);
            yield return changed;
        }
    }

    [Fact]
    public void BrokenImagesAreReadOrRefusedWithoutAnException()
    {
        var files = Shared("png", "jpg", "jpeg", "exr").ToList();
        Assert.True(files.Count > 40);
        foreach (var file in files)
        {
            foreach (var bytes in Broken(File.ReadAllBytes(file), 40, 60))
            {
                var image = Loader.DecodeImage(bytes);
                Assert.True(image.IsError || image.Value.Rgba.Length == image.Value.Width * image.Value.Height * 4, file);
            }
        }
    }

    [Fact]
    public void BrokenModelsAreReadOrRefusedWithoutAnException()
    {
        var dir = TestFiles.TempDir("hostile-models");
        var files = Shared("gltf", "glb", "obj", "bin", "mtl").ToList();
        foreach (var file in files)
        {
            File.Copy(file, Path.Join(dir, Path.GetFileName(file)), true);
        }
        foreach (var file in files.Where(f => Path.GetExtension(f) is ".gltf" or ".glb" or ".obj" or ".bin" or ".mtl"))
        {
            var name = Path.GetFileName(file);
            var original = File.ReadAllBytes(file);
            // A broken buffer or library is read through the models that name it.
            var models = Path.GetExtension(file) is ".bin" or ".mtl"
                ? files.Where(m => Path.GetExtension(m) is ".gltf" or ".obj" && File.ReadAllText(m).Contains(name, StringComparison.Ordinal)).Select(Path.GetFileName).ToList()
                : [name];
            foreach (var bytes in Broken(original, 12, 20))
            {
                File.WriteAllBytes(Path.Join(dir, name), bytes);
                foreach (var model in models)
                {
                    var read = Loader.LoadModel(Path.Join(dir, model!));
                    Assert.True(read.IsError || read.Value.Indices.All(i => i < read.Value.Positions.Count), $"{name} in {model}");
                    _ = Loader.ModelFiles(Path.Join(dir, model!));
                }
            }
            File.WriteAllBytes(Path.Join(dir, name), original);
        }
        TestFiles.Remove(dir);
    }

    [Fact]
    public void BrokenImportSettingsAreRefusedWithAReason()
    {
        var text = "{\"format\":\"bayleaf.import\",\"version\":1,\"model\":{\"scale\":2.5,\"upAxis\":\"z\"},\"image\":{\"mipmaps\":true,\"colorSpace\":\"data\",\"compression\":\"bc3\"}}";
        Assert.True(ImportSettings.FromText(text).IsOk);
        foreach (var bytes in Broken(System.Text.Encoding.UTF8.GetBytes(text), 97, 160))
        {
            var read = ImportSettings.FromText(System.Text.Encoding.UTF8.GetString(bytes));
            Assert.True(read.IsOk || read.Error.Length > 0);
        }
    }
}

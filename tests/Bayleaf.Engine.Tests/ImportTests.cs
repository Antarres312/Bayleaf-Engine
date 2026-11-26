using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Engine.Tests;

public class ImportTests
{
    private static string RepositoryRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    private static JsonObject Params(params (string Key, JsonValue Value)[] fields) => new(fields.Select(f => new KeyValuePair<string, JsonValue>(f.Key, f.Value)));

    private static JsonArray Strings(params string[] items) => new(items.Select(i => (JsonValue)new JsonString(i)));

    private static List<string> FilesUnder(string dir) =>
        [.. Directory.EnumerateFiles(dir, "*", SearchOption.AllDirectories).Select(f => Path.GetRelativePath(dir, f)).Order(StringComparer.Ordinal)];

    private static byte[] WavBytes()
    {
        const int samples = 800;
        using var b = new MemoryStream();
        using var w = new BinaryWriter(b);
        w.Write("RIFF"u8);
        w.Write(36 + (samples * 2));
        w.Write("WAVEfmt "u8);
        w.Write(16);
        w.Write((short)1);
        w.Write((short)1);
        w.Write(8000);
        w.Write(16000);
        w.Write((short)2);
        w.Write((short)16);
        w.Write("data"u8);
        w.Write(samples * 2);
        w.Write(new byte[samples * 2]);
        return b.ToArray();
    }

    /// <summary>A glTF model with its buffer and an image in a folder beside it.</summary>
    private static void WriteGltfWithFiles(string dir, byte[] image)
    {
        var bin = new List<byte>();
        foreach (var f in new float[] { 0, 0, 0, 1, 0, 0, 0, 0, -1 })
        {
            bin.AddRange(BitConverter.GetBytes(f));
        }
        bin.AddRange([0, 0, 1, 0, 2, 0, 0, 0]);
        File.WriteAllBytes(Path.Join(dir, "tri.bin"), [.. bin]);
        Directory.CreateDirectory(Path.Join(dir, "tex"));
        File.WriteAllBytes(Path.Join(dir, "tex", "logo.png"), image);
        File.WriteAllText(Path.Join(dir, "tri.gltf"), $$"""
            {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
             "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
             "buffers": [{"uri": "tri.bin", "byteLength": {{bin.Count}}}],
             "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 6}],
             "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, -1], "max": [1, 0, 0]},
                           {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}],
             "images": [{"uri": "tex/logo.png"}]}
            """);
    }

    [Fact]
    public void FilesBroughtInAreCheckedCopiedWithTheirSettingsAndPlaced()
    {
        var registry = CommandRegistry.WithBuiltins();
        var session = new Session();
        var root = RepositoryRoot();
        var logo = File.ReadAllBytes(Path.Join(root, "assets", "brand", "bayleaf-logo.png"));
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-import-cs-{Environment.ProcessId}");
        if (Directory.Exists(dir))
        {
            Directory.Delete(dir, true);
        }
        var (project, outside) = (Path.Join(dir, "Game"), Path.Join(dir, "outside"));
        try
        {
            JsonValue Run(string cmd, JsonValue p) => registry.Execute(session, cmd, p).Value;
            string Refused(string cmd, JsonValue p) => registry.Execute(session, cmd, p).Error.ToString();
            Run("project.new", Params(("path", new JsonString(project))));
            Directory.CreateDirectory(outside);
            File.WriteAllBytes(Path.Join(outside, "logo.png"), logo);
            File.WriteAllText(Path.Join(outside, "broken.png"), "not an image");
            File.WriteAllText(Path.Join(outside, "crate.obj"), "v 0 0 0");
            File.WriteAllText(Path.Join(outside, "crate.fbx"), "Kaydara FBX Binary");
            // A model with its material library and image beside it.
            File.WriteAllText(Path.Join(outside, "box.obj"), "mtllib box.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl Lit\nf 1 2 3\n");
            File.WriteAllText(Path.Join(outside, "box.mtl"), "newmtl Lit\nKd 1 1 1\nmap_Kd logo.png\n");
            File.WriteAllBytes(Path.Join(outside, "tone.wav"), WavBytes());
            WriteGltfWithFiles(outside, logo);
            var assets = Path.Join(project, "assets");
            var before = FilesUnder(assets);
            string In(params string[] names) => Path.Join([outside, .. names]);

            // One unreadable file stops the whole import, and nothing is left behind.
            Assert.Contains("broken.png", Refused("asset.import", Params(("files", Strings(In("logo.png"), In("broken.png"))), ("folder", new JsonString("assets")))), StringComparison.Ordinal);
            Assert.Contains("crate.obj", Refused("asset.import", Params(("files", Strings(In("crate.obj"))), ("folder", new JsonString("assets")))), StringComparison.Ordinal);
            Assert.Contains("glTF", Refused("asset.import", Params(("files", Strings(In("crate.fbx"))), ("folder", new JsonString("assets")))), StringComparison.Ordinal);
            Assert.Equal(before, FilesUnder(assets));

            var done = Run("asset.import", Params(("files", Strings(In("logo.png"), In("tri.gltf"), In("tone.wav"), In("box.obj"))), ("folder", new JsonString("assets"))));
            Assert.Equal(["assets/logo.png", "assets/tri/tri.gltf", "assets/tone.wav", "assets/box/box.obj"], done.Get("imported")!.AsArray()!.Select(i => i.Get("path")!.AsString()));
            foreach (var file in new[] { "assets/logo.png.bimport", "assets/tri/tri.bin", "assets/tri/tex/logo.png", "assets/tri/tri.gltf.bimport", "assets/box/box.mtl", "assets/box/logo.png", "assets/tone.wav.bimport" })
            {
                Assert.True(File.Exists(Path.Join(project, file)), $"{file} came in");
            }
            Run("asset.importSettings.set", Params(("path", new JsonString("assets/tone.wav")), ("settings", Params(("sound", Params(("stream", JsonBool.True)))))));
            Assert.Equal(true, Run("asset.importSettings.get", Params(("path", new JsonString("assets/tone.wav")))).Get("settings")!.Get("sound")!.Get("stream")!.AsBool());
            var again = Run("asset.import", Params(("files", Strings(In("logo.png"))), ("folder", new JsonString("assets"))));
            Assert.NotEqual("assets/logo.png", again.Get("imported")!.AsArray()![0].Get("path")!.AsString());
            var listed = Run("asset.list", Params(("path", new JsonString("assets")), ("recursive", JsonBool.True)));
            Assert.DoesNotContain(".bimport", listed.ToCompact(), StringComparison.Ordinal);

            // Settings: only the file's own kind, checked, saved beside it.
            var model = new JsonString("assets/tri/tri.gltf");
            Run("asset.importSettings.set", Params(("path", model), ("settings", Params(("model", Params(("scale", JsonNumber.Of(2.0))))))));
            var got = Run("asset.importSettings.get", Params(("path", model)));
            Assert.Equal(2.0, got.Get("settings")!.Get("model")!.Get("scale")!.AsDouble());
            Assert.Equal(true, got.Get("saved")!.AsBool());
            Assert.True(registry.Execute(session, "asset.importSettings.set", Params(("path", model), ("settings", Params(("image", Params(("mipmaps", JsonBool.False))))))).IsError);
            Assert.True(registry.Execute(session, "asset.importSettings.set", Params(("path", model), ("settings", Params(("model", Params(("scale", JsonNumber.Of(-1.0)))))))).IsError);

            // Placing a model and a sound; each one undo step.
            var placed = Run("scene.file.place", Params(("path", model), ("position", new JsonArray([JsonNumber.Of(1.0), JsonNumber.Of(0.0), JsonNumber.Of(2.0)]))));
            var id = StableId.ParseHex(placed.Get("id")!.AsString()!).Value;
            Assert.Equal(model, session.Scene.Component(id, "mesh").Value.Get("model"));
            Assert.Equal(1.0, session.Scene.Component(id, "transform").Value.Get("position")!.AsArray()![0].AsDouble());
            var sound = StableId.ParseHex(Run("scene.file.place", Params(("path", new JsonString("assets/tone.wav")))).Get("id")!.AsString()!).Value;
            Assert.Equal("assets/tone.wav", session.Scene.Component(sound, "audioSource").Value.Get("clip")!.AsString());
            Run("edit.undo", JsonValue.Null);
            Assert.True(session.Scene.Component(sound, "audioSource").IsError, "undone");

            // Confined: a file outside the read roots is refused.
            session.Confinement = Confinement.Create([project], [project]).Value;
            Assert.True(registry.Execute(session, "asset.import", Params(("files", Strings(In("logo.png"))), ("folder", new JsonString("assets")))).IsError);
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }
}

using Bayleaf.Assets;
using Bayleaf.Core.Json;

namespace Bayleaf.Engine.Tests;

/// <summary><c>scene.render</c>, as the Rust engine's tests check it.</summary>
public class RenderTests
{
    private static JsonValue Json(string text) => JsonReader.Parse(text).Value;

    private static JsonString Text(string text) => new(text);

    private static string Folder(string name)
    {
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-render-cs-{name}-{System.Environment.ProcessId}-{Guid.NewGuid():N}");
        Directory.CreateDirectory(dir);
        return dir;
    }

    /// <summary>The render's result, or null when there is no GPU here (a failure with <c>BAYLEAF_GPU_REQUIRED=1</c>).</summary>
    private static JsonValue? Rendered(CommandRegistry registry, Session session, JsonValue p)
    {
        var result = registry.Execute(session, "scene.render", p);
        if (result.IsOk)
        {
            return result.Value;
        }
        Assert.Contains("no usable graphics adapter", result.Error.Message, StringComparison.Ordinal);
        Assert.True(System.Environment.GetEnvironmentVariable("BAYLEAF_GPU_REQUIRED") is not { Length: > 0 }, result.Error.Message);
        return null;
    }

    [Fact]
    public void ARenderRefusesBadSizesAndNonCameraObjectsBeforeWriting()
    {
        var registry = CommandRegistry.WithBuiltins();
        var session = new Session();
        var id = registry.Execute(session, "scene.object.create", Json("{}")).Value.Get("id")!.AsString()!;
        var dir = Folder("refused");
        var path = Path.Join(dir, "x.png");
        foreach (var p in new[] { $"{{\"width\": 0}}", "{\"height\": 100000}", $"{{\"camera\": \"{id}\"}}" })
        {
            var parameters = (JsonObject)Json(p);
            parameters.Add("path", Text(path));
            Assert.True(registry.Execute(session, "scene.render", parameters).IsError, $"accepted {p}");
        }
        Assert.False(File.Exists(path));
        // Inline renders stay small.
        Assert.Equal(CommandErrorKind.BadParams, registry.Execute(session, "scene.render", Json("{\"width\": 4096, \"height\": 8}")).Error.Kind);
        // Agents write only inside their folders.
        var inside = Path.Join(dir, "inside");
        Directory.CreateDirectory(inside);
        session.Confinement = Confinement.Create([inside], [inside]).Value;
        Assert.True(registry.Execute(session, "scene.render", new JsonObject { { "path", Text(path) } }).IsError);
        Assert.False(File.Exists(path));
        Directory.Delete(dir, true);
    }

    [Fact]
    public void ARenderWithoutAPathComesBackAsBase64Png()
    {
        var registry = CommandRegistry.WithBuiltins();
        var session = new Session();
        registry.Execute(session, "scene.object.create", Json("{}"));
        if (Rendered(registry, session, Json("{\"width\": 8, \"height\": 8}")) is not { } result)
        {
            return;
        }
        // The PNG signature, in base64.
        Assert.StartsWith("iVBORw0KGgo", result.Get("png")!.AsString(), StringComparison.Ordinal);
        Assert.Equal("framing", result.Get("camera")!.AsString());
        var image = Loader.DecodeImage(Convert.FromBase64String(result.Get("png")!.AsString()!)).Value;
        Assert.Equal((8, 8), (image.Width, image.Height));
    }

    [Fact]
    public void AScenesPostProcessingAssetChangesItsRenderAsFarAsTheProjectAllows()
    {
        var registry = CommandRegistry.WithBuiltins();
        var session = new Session();
        var dir = Folder("post");
        var project = Path.Join(dir, "Looks");
        Assert.True(registry.Execute(session, "project.new", new JsonObject { { "path", Text(project) } }).IsOk);
        var made = registry.Execute(session, "asset.create", new JsonObject { { "kind", Text("postProcessing") }, { "folder", Text(project) }, { "name", Text("grey") } }).Value;
        var path = made.Get("path")!.AsString()!;
        var post = (JsonObject)registry.Execute(session, "post.get", new JsonObject { { "path", Text(path) } }).Value.Get("post")!;
        JsonObject With(JsonObject value, string field, JsonValue replaced) => new(value.Entries.Select(e => new KeyValuePair<string, JsonValue>(e.Key, e.Key == field ? replaced : e.Value)));
        var grey = With(With(post, "color", Json("""{"enabled": true, "contrast": 1.0, "saturation": 0.0}""")), "toneMapping", Json("""{"enabled": false, "exposure": 1.0}"""));
        Assert.True(registry.Execute(session, "post.set", new JsonObject { { "path", Text(path) }, { "post", grey } }).IsOk);
        registry.Execute(session, "scene.object.create", Json("""{"name": "Red", "components": {"mesh": {"shape": "cube", "color": [0.9, 0.1, 0.1, 1], "metallic": 0, "roughness": 0.5}}}"""));
        (byte R, byte G, byte B)? Centre(string name)
        {
            var png = Path.Join(dir, name);
            if (Rendered(registry, session, new JsonObject { { "path", Text(png) }, { "width", JsonNumber.Of(32L) }, { "height", JsonNumber.Of(32L) } }) is null)
            {
                return null;
            }
            var image = Loader.LoadImage(png).Value;
            var at = (16 * image.Width + 16) * 4;
            return (image.Rgba[at], image.Rgba[at + 1], image.Rgba[at + 2]);
        }
        if (Centre("plain.png") is not { } plain)
        {
            return;
        }
        Assert.True(plain.R > plain.G + 40, $"red as drawn: {plain}");
        Assert.True(registry.Execute(session, "scene.settings.set", Json("""{"postProcessing": "grey.bpost"}""")).IsOk);
        var taken = Centre("grey.png")!.Value;
        Assert.True(Math.Abs(taken.R - taken.G) < 8, $"the scene's post-processing takes the colour out: {taken}");

        // A colour lookup image that swaps red and blue: 8 squares of 8 by 8.
        const int Steps = 8;
        var pixels = new List<byte>();
        for (var g = 0; g < Steps; g++)
        {
            for (var b = 0; b < Steps; b++)
            {
                for (var r = 0; r < Steps; r++)
                {
                    static byte Level(int v) => (byte)(v * 255 / (Steps - 1));
                    pixels.AddRange([Level(b), Level(g), Level(r), 255]);
                }
            }
        }
        File.WriteAllBytes(Path.Join(project, "swap.png"), Png.Encode(Steps * Steps, Steps, pixels.ToArray()));
        var swapped = With(grey, "color", Json("""{"enabled": true, "contrast": 1.0, "saturation": 1.0, "lookup": "swap.png", "lookupStrength": 1.0}"""));
        Assert.True(registry.Execute(session, "post.set", new JsonObject { { "path", Text(path) }, { "post", swapped } }).IsOk);
        var blue = Centre("swapped.png")!.Value;
        Assert.True(blue.B > blue.R + 40, $"the lookup turns the red cube blue: {blue}");
        Assert.True(registry.Execute(session, "post.set", new JsonObject { { "path", Text(path) }, { "post", grey } }).IsOk);

        // A project that does not allow colour adjustment shows the colour again.
        var settings = (JsonObject)registry.Execute(session, "project.settings", JsonValue.Null).Value.Get("settings")!;
        var graphics = With((JsonObject)settings.Get("graphics")!, "effects", Json("""{"color": false}"""));
        Assert.True(registry.Execute(session, "project.settings.set", new JsonObject { { "settings", With(settings, "graphics", graphics) } }).IsOk);
        var allowed = Centre("allowed.png")!.Value;
        Assert.True(allowed.R > allowed.G + 40, $"colour adjustment is not allowed: {allowed}");
        Directory.Delete(dir, true);
    }
}

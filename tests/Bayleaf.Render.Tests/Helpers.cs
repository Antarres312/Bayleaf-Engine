using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Render.Tests;

/// <summary>Scenes built by hand for the renderer's tests.</summary>
internal static class Scenes
{
    public static JsonValue J(string json) => JsonReader.Parse(json).Value;

    private static string V(Vector3 v) => FormattableString.Invariant($"[{v.X}, {v.Y}, {v.Z}]");

    /// <summary>An object with <paramref name="component"/> set to <paramref name="value"/>, placed at <paramref name="position"/> turned by <paramref name="rotation"/> degrees.</summary>
    public static StableId Object(SceneModel scene, string component, string value, Vector3 position, Vector3 rotation = default)
    {
        var id = scene.CreateObject(null, component).Value;
        Assert.True(scene.AddComponent(id, component).IsOk);
        var set = scene.SetComponent(id, component, J(value));
        Assert.True(set.IsOk, set.Error?.Message);
        SetTransform(scene, id, position, rotation, Vector3.One);
        return id;
    }

    public static void SetTransform(SceneModel scene, StableId id, Vector3 position, Vector3 rotation, Vector3 scale) =>
        Assert.True(scene.SetComponent(id, "transform", J($"{{\"position\": {V(position)}, \"rotation\": {V(rotation)}, \"scale\": {V(scale)}}}")).IsOk);

    public static StableId Mesh(SceneModel scene, string shape, Vector3 position) =>
        Object(scene, "mesh", $"{{\"shape\": \"{shape}\", \"color\": [0.8, 0.8, 0.8, 1], \"metallic\": 0, \"roughness\": 0.5}}", position);

    /// <summary>A wide floor seen from above at an angle, with the given lights.</summary>
    public static SceneModel Floor(params (string Component, string Value, Vector3 Position, Vector3 Rotation)[] lights)
    {
        var scene = new SceneModel();
        var floor = Object(scene, "mesh", """{"shape": "plane", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}""", Vector3.Zero);
        SetTransform(scene, floor, Vector3.Zero, Vector3.Zero, new Vector3(20, 1, 20));
        foreach (var (component, value, position, rotation) in lights)
        {
            Object(scene, component, value, position, rotation);
        }
        return scene;
    }

    public static CameraView TopDown() => CameraView.LookAt(new Vector3(0, 12, 0.01f), Vector3.Zero, 60, 1);

    /// <summary>The pixel a world point lands on in a square image of <paramref name="size"/>.</summary>
    public static (uint X, uint Y) CameraPixel(CameraView camera, Vector3 point, uint size)
    {
        var ndc = FMat.ProjectPoint(camera.ViewProj, point);
        var x = (uint)Math.Max(0, (ndc.X * 0.5f + 0.5f) * size);
        var y = (uint)Math.Max(0, (0.5f - ndc.Y * 0.5f) * size);
        return (Math.Min(x, size - 1), Math.Min(y, size - 1));
    }

    public static uint Brightness((byte R, byte G, byte B, byte A) p) => (uint)p.R + p.G + p.B;

    public static (byte R, byte G, byte B, byte A) At(RenderedImage image, (uint X, uint Y) p) => image.Pixel(p.X, p.Y)!.Value;

    public static (byte R, byte G, byte B, byte A) At(RenderedImage image, uint x, uint y) => image.Pixel(x, y)!.Value;
}

/// <summary>
/// The GPU the tests share. A test that needs one skips when there is no adapter; with
/// <c>BAYLEAF_GPU_REQUIRED=1</c> it fails instead, as the Rust tests do.
/// </summary>
internal static class Gpu
{
    private static string why = "";

    private static readonly Lazy<HeadlessGpu?> Shared = new(() =>
    {
        var made = HeadlessGpu.Create();
        if (made.TryGet(out var gpu, out var error))
        {
            return gpu;
        }
        why = error.Message;
        return null;
    });

    /// <summary>Renders go one at a time: the device's error scopes are its own.</summary>
    public static readonly Lock Gate = new();

    public static HeadlessGpu Get()
    {
        if (Shared.Value is { } gpu)
        {
            return gpu;
        }
        if (System.Environment.GetEnvironmentVariable("BAYLEAF_GPU_REQUIRED") is { Length: > 0 })
        {
            Assert.Fail($"graphics adapter required: {why}");
        }
        Assert.Skip($"no graphics adapter ({why})");
        return null!;
    }

    public static RenderedImage Render(Frame frame, uint width, uint height, uint samples = RenderTarget.DefaultSamples)
    {
        var gpu = Get();
        lock (Gate)
        {
            var image = gpu.RenderWithSamples(frame, width, height, samples);
            Assert.True(image.IsOk, image.Error?.Message);
            return image.Value;
        }
    }
}

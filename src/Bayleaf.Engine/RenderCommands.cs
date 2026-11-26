using Bayleaf.Assets;
using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Render;

using RenderEffects = Bayleaf.Render.EffectsAllowed;

namespace Bayleaf.Engine;

/// <summary><c>scene.render</c>: the scene drawn without a window, as a PNG file or returned inline.</summary>
internal static class RenderCommands
{
    /// <summary>Largest image side <c>scene.render</c> accepts.</summary>
    private const uint MaxRenderSide = 8192;

    /// <summary>Largest side of a render returned inline, without a file.</summary>
    private const uint MaxInlineRenderSide = 2048;

    /// <summary>The effects the project's graphics settings allow, as the renderer takes them.</summary>
    private static RenderEffects Allowed(EffectsAllowed e) => new(e.ToneMapping, e.Bloom, e.Color, e.Vignette, e.DepthFog, e.Occlusion, e.Antialiasing);

    public static JsonObject Render(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "width", "height", "camera", "grid");
        var given = f.Nullable("path", JsonRead.Str);
        var width = f.Optional("width", JsonRead.U32, 1280u);
        var height = f.Optional("height", JsonRead.U32, 720u);
        var cameraId = Params.OptionalId(f, "camera");
        var grid = f.Optional("grid", JsonRead.Bool, false);
        // Checked before any rendering work.
        var path = given is null ? null : session.Allowed(given, Access.Write);
        var largest = path is not null ? MaxRenderSide : MaxInlineRenderSide;
        if (width is < 1 || width > largest || height is < 1 || height > largest)
        {
            throw Stop.BadParams($"width and height must be between 1 and {largest}");
        }
        var scene = session.Scene;
        var aspect = (float)width / height;
        CameraView camera;
        string source;
        if (cameraId is { } id)
        {
            camera = Extraction.ObjectCamera(scene, id, aspect) ?? throw Stop.Failed($"object {id.ToHex()} has no camera component");
            source = "object";
        }
        else if (Extraction.SceneCamera(scene, aspect) is { } view)
        {
            (camera, source) = (view, "scene");
        }
        else
        {
            var (center, radius) = Extraction.Bounds(scene) ?? (Core.Geometry.Double3.Zero, 2f);
            (camera, source) = (CameraView.Framing(center, radius, aspect), "framing");
        }
        Result<string, string> Readable(string file)
        {
            try
            {
                return session.Allowed(file, Access.Read);
            }
            catch (CommandException e)
            {
                return Result.Fail(e.Error.Message);
            }
        }
        var assets = new FileAssets(session.ScriptDir, session.MaterialLibrary(), new AssetCache(), Readable);
        var graphics = ProjectCommands.SettingsOf(session).Graphics;
        // Through a camera object, its own post-processing first; the scene's otherwise.
        var effects = new Effects(true, Allowed(graphics.Effects), cameraId ?? Extraction.CameraObject(scene));
        var shadows = new ShadowSettings(graphics.ShadowResolution, graphics.ShadowCascades, graphics.ShadowDistance, graphics.ShadowLights).Clamped();
        var options = new FrameOptions(grid, null, 0, shadows, effects);
        var frame = Extraction.Extract(scene, camera, options, new PosedAssets(assets, session.AnimationPoses));
        var warnings = J.Strings(assets.Warnings);
        var made = HeadlessGpu.Create();
        if (!made.TryGet(out var gpu, out var noGpu))
        {
            throw Stop.Failed(noGpu.Message);
        }
        using (gpu)
        {
            var image = gpu.RenderWithSamples(frame, width, height, graphics.Multisampling ? RenderTarget.DefaultSamples : 1);
            if (!image.TryGet(out var picture, out var failed))
            {
                throw Stop.Failed(failed.Message);
            }
            var png = Png.Encode((int)picture.Width, (int)picture.Height, picture.Pixels, srgb: true);
            if (path is not null)
            {
                Files.Write(path, png).MapError(e => $"cannot create {path}: {e}").OrFail();
                return J.Obj(("path", J.S(path)), ("width", J.N(width)), ("height", J.N(height)), ("camera", J.S(source)), ("warnings", warnings));
            }
            return J.Obj(("png", J.S(Convert.ToBase64String(png))), ("width", J.N(width)), ("height", J.N(height)), ("camera", J.S(source)), ("warnings", warnings));
        }
    }
}

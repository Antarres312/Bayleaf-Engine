using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene;

using EnvironmentComponent = Bayleaf.Scene.Components.Environment;

namespace Bayleaf.Engine;

/// <summary>
/// A scene's settings (<c>scene.settings</c>, <c>scene.settings.set</c>), lighting and post-processing
/// assets, and moving an older scene's Environment component into a lighting asset when the scene opens.
/// </summary>
internal static class LightingCommands
{
    public static IEnumerable<CommandSpec> All() =>
    [
        Spec("scene.settings", "Scene Settings", "none; returns the open scene's own settings: {lighting}", Mutates.No, SettingsGet),
        Spec("scene.settings.set", "Change Scene Settings",
            "{lighting?, postProcessing?}: the lighting asset (.blight, relative to the project folder; empty for the engine's light) and the post-processing asset (.bpost; empty for none) the scene uses; returns the settings",
            Mutates.Scene, SettingsSet),
        Spec("lighting.get", "Read Lighting",
            "{path}: a lighting asset (.blight); returns {lighting: {skyColor, horizonColor, groundColor, ambient, showSky, fogDistance, sun: {enabled, rotation, color, intensity, shadows}}}",
            Mutates.No, LightingGet),
        Spec("lighting.set", "Change Lighting",
            "{path, lighting}: writes a lighting asset (the whole lighting, as lighting.get returns it); every scene that names it changes", Mutates.No, LightingSet),
        Spec("post.get", "Read Post-processing",
            "{path}: a post-processing asset (.bpost); returns {post: {toneMapping, bloom, color (with lookup, lookupStrength), vignette, depthFog, occlusion, antialiasing}}",
            Mutates.No, PostGet),
        Spec("post.set", "Change Post-processing",
            "{path, post}: writes a post-processing asset (all of it, as post.get returns it); every scene and camera that names it changes. A colour lookup image (color.lookup) of the wrong size is refused; one not there yet is a warning",
            Mutates.No, PostSet),
        Spec("post.lookup.neutral", "Make Neutral Lookup",
            "{path, steps?}: writes a colour lookup image (.png) that changes no colour, steps (2 to 64, default 16) squares of steps by steps side by side, to recolour in an image editor and name in color.lookup; returns {path, relative} (relative to the project). Refuses a file that is already there",
            Mutates.No, LookupNeutral),
    ];

    private static CommandSpec Spec(string id, string label, string parameters, Mutates mutates, Func<Session, JsonValue, JsonValue> run) =>
        new() { Id = id, Label = label, Params = parameters, Mutates = mutates, Run = run };

    /// <summary>A file's path of the given kind, relative to the project folder or absolute, checked for <paramref name="access"/>.</summary>
    internal static string KindPath(Session session, string path, string extension, Access access)
    {
        if (!Text.AsciiLower(path).EndsWith($".{extension}", StringComparison.Ordinal))
        {
            throw Stop.BadParams($"give a .{extension} file");
        }
        return session.Allowed(Path.Combine(session.ScriptDir, path), access);
    }

    private static JsonValue SettingsGet(Session session, JsonValue p)
    {
        Params.None(p);
        return session.Scene.Settings.Write();
    }

    private static JsonValue SettingsSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "lighting", "postProcessing");
        var settings = session.Scene.Settings;
        if (f.Nullable("lighting", JsonRead.Str) is { } lighting)
        {
            settings = settings with { Lighting = lighting.Trim().Replace('\\', '/') };
        }
        if (f.Nullable("postProcessing", JsonRead.Str) is { } post)
        {
            settings = settings with { PostProcessing = post.Trim().Replace('\\', '/') };
        }
        settings.Validate().OrBadParams();
        session.Scene.SetSettings(settings).OrFail();
        return session.Scene.Settings.Write();
    }

    private static string PathOf(JsonValue p) => Params.Of(p, "path").Required("path", JsonRead.Str);

    private static JsonValue LightingGet(Session session, JsonValue p)
    {
        var path = KindPath(session, PathOf(p), Lighting.Extension, Access.Read);
        return J.Obj(("lighting", Lighting.Load(path).OrFail().ToJson()));
    }

    private static JsonValue LightingSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "lighting");
        var given = f.Required("path", JsonRead.Str);
        var lighting = f.Required("lighting", Lighting.FromJson);
        lighting.Validate().OrBadParams();
        var path = KindPath(session, given, Lighting.Extension, Access.Write);
        lighting.Save(path).OrFail();
        return J.Obj(("path", J.S(path)));
    }

    private static JsonValue PostGet(Session session, JsonValue p)
    {
        var path = KindPath(session, PathOf(p), PostProcessing.Extension, Access.Read);
        return J.Obj(("post", PostProcessing.Load(path).OrFail().ToJson()));
    }

    private static JsonValue PostSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "post");
        var given = f.Required("path", JsonRead.Str);
        var post = f.Required("post", PostProcessing.FromJson);
        post.Validate().OrBadParams();
        var path = KindPath(session, given, PostProcessing.Extension, Access.Write);
        var warnings = LookupCheck(session, post);
        post.Save(path).OrFail();
        return J.Obj(("path", J.S(path)), ("warnings", J.Strings(warnings)));
    }

    /// <summary>Refuses a colour lookup image of the wrong size; one that is not there yet is a warning.</summary>
    private static List<string> LookupCheck(Session session, PostProcessing post)
    {
        var name = post.Color.Lookup;
        if (name.Length == 0)
        {
            return [];
        }
        var imagePath = session.Allowed(Path.Combine(session.ScriptDir, name), Access.Read);
        if (!File.Exists(imagePath))
        {
            return [$"color.lookup {name} is not there yet; colours change once it is"];
        }
        var size = Png.Size(imagePath);
        if (size.IsError)
        {
            throw Stop.BadParams($"color.lookup {name}: {size.Error}");
        }
        var (width, height) = size.Value;
        if (!ColorAdjustment.IsLookupSize(width, height))
        {
            throw Stop.BadParams($"color.lookup {name} is {width} by {height}; a lookup image is N squares of N by N side by side (N times N wide, N high, N from 2 to {ColorAdjustment.MaxLookupSteps})");
        }
        return [];
    }

    private static JsonValue LookupNeutral(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "steps");
        var given = f.Required("path", JsonRead.Str);
        var steps = f.Optional("steps", JsonRead.U32, 16u);
        if (steps is < 2 or > ColorAdjustment.MaxLookupSteps)
        {
            throw Stop.BadParams($"steps must be between 2 and {ColorAdjustment.MaxLookupSteps}");
        }
        if (!Text.AsciiLower(given).EndsWith(".png", StringComparison.Ordinal))
        {
            throw Stop.BadParams("give a .png file");
        }
        var path = session.Allowed(Path.Combine(session.ScriptDir, given), Access.Write);
        if (File.Exists(path) || Directory.Exists(path))
        {
            throw Stop.BadParams($"{path} is already there");
        }
        Png.Write(path, (int)(steps * steps), (int)steps, NeutralLookup(steps)).OrFail();
        return J.Obj(("path", J.S(path)), ("relative", J.S(Relative(session, path))));
    }

    /// <summary>The pixels of a lookup image that maps every colour to itself.</summary>
    private static byte[] NeutralLookup(uint steps)
    {
        byte Level(uint v) => (byte)((v * 255 + (steps - 1) / 2) / (steps - 1));
        var pixels = new List<byte>((int)(steps * steps * steps * 4));
        for (uint green = 0; green < steps; green++)
        {
            for (uint blue = 0; blue < steps; blue++)
            {
                for (uint red = 0; red < steps; red++)
                {
                    pixels.AddRange([Level(red), Level(green), Level(blue), 255]);
                }
            }
        }
        return [.. pixels];
    }

    /// <summary><paramref name="path"/> relative to the script folder when it starts with it, with <c>/</c>.</summary>
    internal static string Relative(Session session, string path)
    {
        var (dirRoot, dirNames) = Files.Parts(session.ScriptDir);
        var (root, names) = Files.Parts(path);
        var inside = dirRoot == root && names.Count >= dirNames.Count && dirNames.SequenceEqual(names.Take(dirNames.Count));
        return (inside ? string.Join('/', names.Skip(dirNames.Count)) : path).Replace('\\', '/');
    }

    /// <summary>A file name in <paramref name="dir"/> from <paramref name="stem"/> that is not taken yet: <c>stem.blight</c>, <c>stem 2.blight</c>, …</summary>
    private static string FreePath(string dir, string stem)
    {
        for (var n = 1; n < 100_000; n++)
        {
            var candidate = Path.Combine(dir, n == 1 ? $"{stem}.{Lighting.Extension}" : $"{stem} {n}.{Lighting.Extension}");
            if (!File.Exists(candidate) && !Directory.Exists(candidate))
            {
                return candidate;
            }
        }
        return Path.Combine(dir, $"{stem}.{Lighting.Extension}");
    }

    /// <summary>
    /// Moves an older scene's Environment component into a lighting asset beside the scene, which the
    /// scene then names, and takes the component away; the scene is left changed, for saving. A scene
    /// that already names a lighting asset only loses its Environment. Says what it did, or why it left
    /// the scene as it was; null when the scene has no Environment.
    /// </summary>
    internal static (Level Level, string Message, bool Changed)? MoveEnvironment(Session session)
    {
        var scene = session.Scene;
        var found = scene.WorldMatrices().Select(m => m.Id).Where(id => scene.TryGet<EnvironmentComponent>(id, out _)).ToList();
        if (found.Count == 0 || !scene.TryGet<EnvironmentComponent>(found[0], out var first))
        {
            return null;
        }
        if (scene.Settings.Lighting.Length == 0)
        {
            if (session.ScenePath is not { } scenePath || Path.GetDirectoryName(scenePath) is not { } dir)
            {
                return null;
            }
            var stem = Path.GetFileNameWithoutExtension(scenePath);
            var target = FreePath(dir, $"{(stem.Length > 0 ? stem : "Scene")} lighting");
            try
            {
                target = session.Allowed(target, Access.Write);
            }
            catch (CommandException e)
            {
                return (Level.Info, $"The scene's Environment stays as it is: its lighting asset cannot be written ({e.Error.Message}).", false);
            }
            // The old scene's own lights stay its light: the asset's sun is off.
            var moved = Lighting.FromEnvironment(first, Sun.Default with { Enabled = false });
            var saved = moved.Save(target);
            if (saved.IsError)
            {
                return (Level.Error, $"The scene's Environment stays as it is: {saved.Error.Message}", false);
            }
            var relative = Relative(session, target);
            if (scene.SetSettings(scene.Settings with { Lighting = relative }).IsError)
            {
                return null;
            }
            foreach (var id in found)
            {
                scene.RemoveComponent(id, "environment");
            }
            return (Level.Info, $"Moved the scene's Environment into the lighting asset {relative}; save the scene to keep it.", true);
        }
        foreach (var id in found)
        {
            scene.RemoveComponent(id, "environment");
        }
        return (Level.Info, "The scene names a lighting asset, so its old Environment was taken away; save the scene to keep it.", true);
    }
}

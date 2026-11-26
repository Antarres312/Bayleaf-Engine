using System.Numerics;
using System.Text;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Engine;

/// <summary>The post-processing effects a project lets its scenes use; each on unless turned off.</summary>
public sealed record EffectsAllowed(bool ToneMapping = true, bool Bloom = true, bool Color = true, bool Vignette = true, bool DepthFog = true, bool Occlusion = true, bool Antialiasing = true)
{
    private static readonly string[] Names = ["toneMapping", "bloom", "color", "vignette", "depthFog", "occlusion", "antialiasing"];

    internal static EffectsAllowed Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct EffectsAllowed", true, Names);
        bool On(string name) => f.Optional(name, JsonRead.Bool, true);
        return new(On("toneMapping"), On("bloom"), On("color"), On("vignette"), On("depthFog"), On("occlusion"), On("antialiasing"));
    }

    internal JsonObject Write() => new()
    {
        { "toneMapping", JsonValue.From(ToneMapping) },
        { "bloom", JsonValue.From(Bloom) },
        { "color", JsonValue.From(Color) },
        { "vignette", JsonValue.From(Vignette) },
        { "depthFog", JsonValue.From(DepthFog) },
        { "occlusion", JsonValue.From(Occlusion) },
        { "antialiasing", JsonValue.From(Antialiasing) },
    };
}

/// <summary>How the editor and the game draw every scene of the project.</summary>
public sealed record GraphicsSettings(uint ShadowResolution, uint ShadowCascades, float ShadowDistance, uint ShadowLights, EffectsAllowed Effects, bool Multisampling)
{
    /// <summary>The shadow-map sizes on offer.</summary>
    public static readonly IReadOnlyList<uint> ShadowResolutions = [512, 1024, 2048, 4096];

    /// <summary>Largest shadow distance, in world units.</summary>
    public const float MaxShadowDistance = 100_000f;

    public static GraphicsSettings Default => new(2048, 3, 0f, 1, new EffectsAllowed(), true);

    internal static GraphicsSettings Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct GraphicsSettings", true, "shadowResolution", "shadowCascades", "shadowDistance", "shadowLights", "effects", "multisampling");
        var d = Default;
        return new(f.Optional("shadowResolution", JsonRead.U32, d.ShadowResolution), f.Optional("shadowCascades", JsonRead.U32, d.ShadowCascades),
            f.Optional("shadowDistance", JsonRead.F32, d.ShadowDistance), f.Optional("shadowLights", JsonRead.U32, d.ShadowLights),
            f.Optional("effects", EffectsAllowed.Read, new EffectsAllowed()), f.Optional("multisampling", JsonRead.Bool, true));
    }

    internal JsonObject Write() => new()
    {
        { "shadowResolution", JsonNumber.Of((ulong)ShadowResolution) },
        { "shadowCascades", JsonNumber.Of((ulong)ShadowCascades) },
        { "shadowDistance", JsonNumber.Of(ShadowDistance) },
        { "shadowLights", JsonNumber.Of((ulong)ShadowLights) },
        { "effects", Effects.Write() },
        { "multisampling", JsonValue.From(Multisampling) },
    };

    internal void Check()
    {
        if (!ShadowResolutions.Contains(ShadowResolution))
        {
            throw Stop.BadParams("graphics.shadowResolution must be 512, 1024, 2048 or 4096");
        }
        if (ShadowCascades is < 1 or > 4)
        {
            throw Stop.BadParams("graphics.shadowCascades must be between 1 and 4");
        }
        if (!(float.IsFinite(ShadowDistance) && ShadowDistance >= 0f && ShadowDistance <= MaxShadowDistance))
        {
            throw Stop.BadParams($"graphics.shadowDistance must be between 0 and {Text.Number(MaxShadowDistance)}");
        }
        if (ShadowLights is < 1 or > 4)
        {
            throw Stop.BadParams("graphics.shadowLights must be between 1 and 4");
        }
    }
}

/// <summary>How the exported game shows itself, and how it runs its scripts per platform.</summary>
public sealed record GameSettings(string Icon, SortedDictionary<int, string> ScriptModes)
{
    /// <summary>The platforms, in the order the Rust engine sorts them.</summary>
    internal static readonly string[] Platforms = ["windows", "linux", "mac", "web"];

    internal static readonly string[] Modes = ["interpreted", "compiled", "native"];

    public static GameSettings Default => new("", []);

    public bool Equals(GameSettings? other) => other is not null && Icon == other.Icon && ScriptModes.SequenceEqual(other.ScriptModes);

    public override int GetHashCode() => StringComparer.Ordinal.GetHashCode(Icon);

    internal static GameSettings Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct GameSettings", true, "icon", "scriptModes");
        var modes = new SortedDictionary<int, string>();
        if (f.Has("scriptModes"))
        {
            foreach (var (platform, mode) in f.Required("scriptModes", v => JsonRead.Map(v, m => Modes[JsonRead.Variant(m, Modes)])))
            {
                modes[JsonRead.Variant(new JsonString(platform), Platforms)] = mode;
            }
        }
        return new(f.Optional("icon", JsonRead.Str, ""), modes);
    }

    internal JsonObject Write()
    {
        var o = new JsonObject { { "icon", new JsonString(Icon) } };
        if (ScriptModes.Count > 0)
        {
            o.Add("scriptModes", new JsonObject(ScriptModes.Select(m => new KeyValuePair<string, JsonValue>(Platforms[m.Key], new JsonString(m.Value)))));
        }
        return o;
    }
}

/// <summary>An action as project files before format 3 kept it, with its bindings.</summary>
public sealed record ActionSetting(string Name, IReadOnlyList<string> Bindings);

/// <summary>The project's input: an Input Map asset, and the actions older project files kept inline.</summary>
public sealed record InputSettings(string Map, IReadOnlyList<ActionSetting>? Actions)
{
    internal static InputSettings Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct InputSettings", true, "map", "actions");
        var actions = f.Nullable("actions", v => (IReadOnlyList<ActionSetting>)JsonRead.List(v, a =>
        {
            var af = JsonFields.Of(a, "struct ActionSetting", true, "name", "bindings");
            return new ActionSetting(af.Required("name", JsonRead.Str), af.Required("bindings", b => JsonRead.List(b, JsonRead.Str)));
        }));
        return new(f.Optional("map", JsonRead.Str, ""), actions);
    }

    internal JsonObject Write()
    {
        var o = new JsonObject { { "map", new JsonString(Map) } };
        if (Actions is not null)
        {
            o.Add("actions", new JsonArray(Actions.Select(a => (JsonValue)new JsonObject { { "name", new JsonString(a.Name) }, { "bindings", J.Strings(a.Bindings) } })));
        }
        return o;
    }

    /// <summary>Old inline actions as an Input Map asset: map <c>Default</c>, every action a value.</summary>
    internal InputAsset? LegacyAsset() =>
        Actions is null ? null : new InputAsset("Default", [new InputMapDef("Default", Actions.Select(a => new InputAction(a.Name, ActionKind.Value, a.Bindings)).ToList())]);
}

/// <summary>Physics in play mode.</summary>
public sealed record PhysicsSettings(bool Enabled, Vector3 Gravity, string Precision)
{
    /// <summary>Largest gravity along any axis.</summary>
    public const float MaxGravity = 1000f;

    public static PhysicsSettings Default => new(true, new(0f, -9.81f, 0f), "single");

    internal static PhysicsSettings Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct PhysicsSettings", true, "enabled", "gravity", "precision");
        var gravity = f.Required("gravity", v => JsonRead.Fixed(v, 3, JsonRead.F32));
        string[] precisions = ["single", "double"];
        return new(f.Required("enabled", JsonRead.Bool), new(gravity[0], gravity[1], gravity[2]), f.Optional("precision", v => precisions[JsonRead.Variant(v, precisions)], "single"));
    }

    internal JsonObject Write() => new()
    {
        { "enabled", JsonValue.From(Enabled) },
        { "gravity", new JsonArray([JsonNumber.Of(Gravity.X), JsonNumber.Of(Gravity.Y), JsonNumber.Of(Gravity.Z)]) },
        { "precision", new JsonString(Precision) },
    };
}

/// <summary>Settings for the whole project. Play mode reads them when it starts.</summary>
public sealed record ProjectSettings(PhysicsSettings Physics, InputSettings Input, bool HideExtensions, GameSettings Game, GraphicsSettings Graphics)
{
    public static ProjectSettings Default => new(PhysicsSettings.Default, new InputSettings("", null), true, GameSettings.Default, GraphicsSettings.Default);

    internal static ProjectSettings Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ProjectSettings", true, "physics", "input", "editor", "game", "graphics");
        var hide = f.Optional("editor", v => JsonFields.Of(v, "struct EditorSettings", true, "hideExtensions").Optional("hideExtensions", JsonRead.Bool, true), true);
        return new(f.Optional("physics", PhysicsSettings.Read, PhysicsSettings.Default), f.Optional("input", InputSettings.Read, new InputSettings("", null)), hide,
            f.Optional("game", GameSettings.Read, GameSettings.Default), f.Optional("graphics", GraphicsSettings.Read, GraphicsSettings.Default));
    }

    internal JsonObject Write() => new()
    {
        { "physics", Physics.Write() },
        { "input", Input.Write() },
        { "editor", new JsonObject { { "hideExtensions", JsonValue.From(HideExtensions) } } },
        { "game", Game.Write() },
        { "graphics", Graphics.Write() },
    };

    internal void Check()
    {
        if (!J3(Physics.Gravity).All(g => float.IsFinite(g) && Math.Abs(g) <= PhysicsSettings.MaxGravity))
        {
            throw Stop.BadParams($"gravity values must be finite and at most {Text.Number(PhysicsSettings.MaxGravity)} in size");
        }
        var icon = Game.Icon;
        if (!(icon.Length == 0 || (Text.Bytes(icon) <= 512 && Text.AsciiLower(icon).EndsWith(".png", StringComparison.Ordinal))))
        {
            throw Stop.BadParams("game.icon must be the path of a .png file");
        }
        var map = Input.Map;
        if (!(map.Length == 0 || (Text.Bytes(map) <= 512 && map.EndsWith($".{InputAsset.Extension}", StringComparison.Ordinal))))
        {
            throw Stop.BadParams($"input.map must be the path of a .{InputAsset.Extension} file");
        }
        Graphics.Check();
    }

    private static float[] J3(Vector3 v) => [v.X, v.Y, v.Z];
}

/// <summary>A project file (<c>.bproject</c>).</summary>
public sealed record ProjectFile(string Format, uint Version, string Name, string MainScene, ProjectSettings Settings)
{
    public const string FormatName = "bayleaf.project";
    public const string Extension = "bproject";

    /// <summary>Version 9 added multisampling on or off; older files still open, and opening one moves its inline actions.</summary>
    public const uint CurrentVersion = 9;

    internal static ProjectFile ReadFile(string path)
    {
        var text = Files.ReadText(path);
        if (text.IsError)
        {
            throw Stop.Failed($"cannot read {path}: {text.Error}");
        }
        var parsed = JsonReader.Parse(text.Value);
        var file = parsed.IsOk ? JsonRead.Try(() => Read(parsed.Value)) : Result.Fail(parsed.Error);
        if (file.IsError)
        {
            throw Stop.Failed($"{path} is not a project file: {file.Error}");
        }
        if (file.Value.Format != FormatName || file.Value.Version is < 1 or > CurrentVersion)
        {
            throw Stop.Failed($"{path} is not a version {CurrentVersion} Bayleaf project");
        }
        return file.Value;
    }

    internal static ProjectFile Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ProjectFile", true, "format", "version", "name", "mainScene", "settings");
        return new(f.Required("format", JsonRead.Str), f.Required("version", JsonRead.U32), f.Required("name", JsonRead.Str), f.Required("mainScene", JsonRead.Str),
            f.Optional("settings", ProjectSettings.Read, ProjectSettings.Default));
    }

    internal string ToText() => new JsonObject
    {
        { "format", new JsonString(Format) },
        { "version", JsonNumber.Of((ulong)Version) },
        { "name", new JsonString(Name) },
        { "mainScene", new JsonString(MainScene) },
        { "settings", Settings.Write() },
    }.ToPretty();
}

/// <summary>
/// Projects: a folder with a <c>.bproject</c> file. Paths in the project's scenes start at that
/// folder, so scenes can live in subfolders. <c>project.new</c> makes a new project folder with a small
/// starting point to build on.
/// </summary>
internal static class ProjectCommands
{
    /// <summary>Where a new project keeps its Input Map, relative to the project folder.</summary>
    private const string StarterInputMap = "input/game.binput";

    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "project.new", Label = "New Project…", Menu = "File/New Project…",
            Params = "{path, name?}: makes the folder `path` (it must not exist or be empty) with a starting scene, a material and a visual script, then opens the scene",
            Prompt = Prompt.NewProject, Enabled = Session.NotPlaying, Run = New,
        },
        new()
        {
            Id = "project.open", Label = "Open Project…", Menu = "File/Open Project…", Params = "{path}: a .bproject file or its folder; opens the project's main scene",
            Prompt = Prompt.OpenProject, Enabled = Session.NotPlaying, Run = Open,
        },
        new() { Id = "project.info", Label = "Project Info", Params = "none; the open project's folder, name and main scene, or null", Run = Info },
        new()
        {
            Id = "project.settings", Label = "Project Settings",
            Params = "none; returns {project, settings}: the open project's settings (physics: {enabled, gravity, precision: \"single\" | \"double\"}, input: {map: the project's Input Map, a .binput file (see input.asset)}), or the defaults and project: false outside a project",
            Run = Settings,
        },
        new()
        {
            Id = "project.settings.set", Label = "Change Project Settings",
            Params = "{settings}: the whole settings object, written to the project file; play mode uses it from its next start",
            Enabled = static s => s.ProjectDir is not null ? null : "open a project first; scenes outside a project use the default settings", Run = SettingsSet,
        },
    ];

    /// <summary>The open project's settings, or the defaults for a scene outside any project (or an unreadable file).</summary>
    internal static ProjectSettings SettingsOf(Session session)
    {
        if (session.ProjectDir is not { } dir || ProjectFileIn(dir) is not { } file)
        {
            return ProjectSettings.Default;
        }
        try
        {
            return ProjectFile.ReadFile(file).Settings;
        }
        catch (CommandException)
        {
            return ProjectSettings.Default;
        }
    }

    /// <summary><paramref name="relative"/> (with <c>/</c> separators) inside <paramref name="dir"/>, spelled with the platform's separators.</summary>
    private static string Inside(string dir, string relative) => relative.Split('/', StringSplitOptions.RemoveEmptyEntries).Aggregate(dir, Path.Combine);

    /// <summary>The project file in <paramref name="dir"/>, if it has one.</summary>
    internal static string? ProjectFileIn(string dir)
    {
        var listed = Files.ReadDirectory(dir);
        return listed.IsOk ? listed.Value.FirstOrDefault(p => Files.IsFile(p) && Path.GetExtension(p) == $".{ProjectFile.Extension}") : null;
    }

    /// <summary>The project folder a scene file belongs to: the nearest folder above it with a project file.</summary>
    internal static string? ProjectDirOf(string scene)
    {
        var dir = Path.GetDirectoryName(scene);
        while (dir is not null)
        {
            if (ProjectFileIn(dir) is not null)
            {
                return dir;
            }
            dir = dir.Length == 0 ? null : Path.GetDirectoryName(dir);
        }
        return null;
    }

    private static JsonObject Settings(Session session, JsonValue p)
    {
        Params.None(p);
        return J.Obj(("project", J.B(session.ProjectDir is not null)), ("settings", SettingsOf(session).Write()));
    }

    private static JsonObject SettingsSet(Session session, JsonValue p)
    {
        var settings = Params.Of(p, "settings").Required("settings", ProjectSettings.Read);
        settings.Check();
        var dir = session.ProjectDir ?? throw Stop.Failed("open a project first");
        var file = ProjectFileIn(dir) ?? throw Stop.Failed("the project file is gone");
        var path = session.Allowed(file, Access.Write);
        var project = ProjectFile.ReadFile(path) with { Settings = settings, Version = ProjectFile.CurrentVersion };
        Files.Write(path, System.Text.Encoding.UTF8.GetBytes(project.ToText())).MapError(e => $"cannot write {path}: {e}").OrFail();
        return J.Obj(("settings", project.Settings.Write()));
    }

    private static JsonObject New(Session session, JsonValue p)
    {
        session.NoPrefabOpen("opening a project");
        var f = Params.Of(p, "path", "name");
        var dir = session.Allowed(f.Required("path", JsonRead.Str), Access.Write);
        var given = f.Nullable("name", JsonRead.Str);
        if ((File.Exists(dir) || Directory.Exists(dir)) && !(Directory.Exists(dir) && !Directory.EnumerateFileSystemEntries(dir).Any()))
        {
            throw Stop.Failed($"{dir} already exists and is not empty; pick a new folder");
        }
        var name = (given ?? (Path.GetFileName(Path.TrimEndingDirectorySeparator(dir)) is { Length: > 0 } folder ? folder : "New Game")).Trim();
        if (name.Length == 0 || Text.Chars(name) > 100)
        {
            throw Stop.BadParams("the project name must have 1 to 100 characters");
        }
        var fileStem = string.Concat(name.EnumerateRunes().Select(r => Rune.IsLetterOrDigit(r) || r.Value is '-' or '_' ? r.ToString() : "_"));
        void Write(string relative, string text)
        {
            var path = Path.Combine(dir, relative);
            if (Path.GetDirectoryName(path) is { Length: > 0 } parent)
            {
                Files.CreateDirectory(parent).MapError(e => $"cannot make {parent}: {e}").OrFail();
            }
            Files.Write(path, System.Text.Encoding.UTF8.GetBytes(text)).MapError(e => $"cannot write {path}: {e}").OrFail();
        }
        var project = new ProjectFile(ProjectFile.FormatName, ProjectFile.CurrentVersion, name, "scenes/main.bscene",
            ProjectSettings.Default with { Input = new InputSettings(StarterInputMap, null) });
        Write($"{fileStem}.{ProjectFile.Extension}", project.ToText());
        Write("scenes/main.bscene", Starter.MainScene);
        // The sky and light around everything; the Sun object is the scene's own sun.
        Write("scenes/daylight.blight", (Lighting.Default with { Sun = Sun.Default with { Enabled = false } }).ToText());
        Write("materials/ground.bmaterial", Starter.GroundMaterial);
        Write("scripts/spin.bscript", Starter.SpinScript);
        Write(StarterInputMap, InputAsset.Starter().ToText());
        Write("assets/README.md", "Images, sounds and other files the game uses go here.\n");
        Write(".gitignore", "# Files the editor or build tools make again.\n/build/\n/.build/\n*.wasm\n");
        Write("README.md",
            $"# {name}\n\nA Bayleaf Engine project. Open `{fileStem}.{ProjectFile.Extension}` in the editor.\n\n- `scenes/`: scenes; `main.bscene` opens first\n- `materials/`: material graphs\n- `scripts/`: visual scripts and WebAssembly scripts\n- `assets/`: images, sounds and other files\n- `saves/`: values the game keeps between plays, such as a best score (safe to delete)\n");
        var scene = Inside(dir, project.MainScene);
        OpenSceneAt(session, scene);
        return J.Obj(("path", J.S(dir)), ("name", J.S(name)), ("scene", J.S(scene)));
    }

    private static JsonObject Open(Session session, JsonValue p)
    {
        session.NoPrefabOpen("opening a project");
        var path = session.Allowed(Params.Of(p, "path").Required("path", JsonRead.Str), Access.Read);
        var file = Files.IsDirectory(path) ? ProjectFileIn(path) ?? throw Stop.Failed($"{path} has no .{ProjectFile.Extension} file") : path;
        var project = ProjectFile.ReadFile(file);
        var dir = Path.GetDirectoryName(file) ?? "";
        MoveInlineActions(session, file, dir, project);
        var scene = Inside(dir, project.MainScene);
        OpenSceneAt(session, scene);
        return J.Obj(("path", J.S(dir)), ("name", J.S(project.Name)), ("scene", J.S(scene)));
    }

    /// <summary>
    /// A project from before format 3 keeps its input actions in the project file: they move into an
    /// Input Map asset that the settings then name. When that cannot be written, nothing changes.
    /// </summary>
    private static void MoveInlineActions(Session session, string file, string dir, ProjectFile project)
    {
        if (project.Settings.Input.Map.Length != 0 || project.Settings.Input.LegacyAsset() is not { } asset)
        {
            return;
        }
        var relative = Enumerable.Range(1, 99).Select(n => n == 1 ? "input/actions.binput" : $"input/actions_{n}.binput")
            .FirstOrDefault(r => !File.Exists(Inside(dir, r)) && !Directory.Exists(Inside(dir, r))) ?? "input/actions.binput";
        try
        {
            var path = session.Allowed(file, Access.Write);
            InputCommands.WriteAsset(session, Inside(dir, relative), asset);
            var updated = project with { Settings = project.Settings with { Input = new InputSettings(relative, null) }, Version = ProjectFile.CurrentVersion };
            Files.Write(path, System.Text.Encoding.UTF8.GetBytes(updated.ToText())).MapError(e => $"cannot write {path}: {e}").OrFail();
            session.Log(Level.Info, $"The project's input actions moved into {relative} (map Default), chosen in Project Settings.");
        }
        catch (CommandException e)
        {
            session.Log(Level.Info, $"The project's input actions stay in the project file for now: {e.Error.Message}");
        }
    }

    private static void OpenSceneAt(Session session, string scene)
    {
        var path = session.Allowed(scene, Access.Read);
        var loaded = SceneModel.Load(path).OrFail();
        // A prefab file that cannot be read leaves its copies as the scene holds them.
        session.ReplaceScene(loaded, path);
    }

    private static JsonValue Info(Session session, JsonValue p)
    {
        if (!(p is JsonNull || p is JsonObject { Count: 0 }))
        {
            throw Stop.BadParams("project.info takes no params");
        }
        if (session.ProjectDir is not { } dir)
        {
            return JsonValue.Null;
        }
        var file = ProjectFileIn(dir) ?? throw Stop.Failed("the project file is gone");
        var project = ProjectFile.ReadFile(file);
        return J.Obj(("path", J.S(dir)), ("name", J.S(project.Name)), ("mainScene", J.S(project.MainScene)));
    }
}

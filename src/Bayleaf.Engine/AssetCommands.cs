using Bayleaf.Assets;
using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.GameInterface;
using Bayleaf.Render;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;
using Bayleaf.Script;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Engine;

/// <summary>
/// The project's files: <c>asset.create</c> makes new folders and asset files (never over a file),
/// <c>asset.list</c> lists a folder as the Project panel does, <c>asset.rename</c> and <c>asset.delete</c>
/// rename and delete them with their import settings. A script is a C# file (docs/script-sdk.md);
/// Rust script crates are not special here.
/// </summary>
internal static class AssetCommands
{
    /// <summary>Most entries one <c>asset.list</c> returns.</summary>
    private const int MaxListed = 5000;

    /// <summary>Folders never listed: build output and package caches.</summary>
    private static readonly string[] NotListed = ["target", "node_modules"];

    private const string NameRule = "a name has 1 to 100 characters and none of / \\ : * ? \" < > |";

    private static readonly string[] Kinds =
        ["folder", "scene", "material", "visualScript", "script", "inputMap", "lighting", "postProcessing", "interface", "interfaceStyle", "dataShape", "dataFile"];

    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "asset.create", Label = "Create Asset",
            Params = "{kind: \"folder\" | \"scene\" | \"material\" | \"visualScript\" | \"script\" | \"inputMap\" | \"lighting\" | \"postProcessing\" | \"interface\" | \"interfaceStyle\" | \"dataShape\" | \"dataFile\", folder, name}: makes it in the folder (relative to the project folder or absolute); returns {path}, and for a script {path, module}",
            Run = Create,
        },
        new()
        {
            Id = "asset.list", Label = "List Assets",
            Params = "{path?, recursive?}: what a folder of the project holds (the project folder when left out), as the Project panel lists it: hidden and build folders left out. Returns {entries: [{path, kind}], truncated}, paths relative to the project folder",
            Run = List,
        },
        new()
        {
            Id = "asset.delete", Label = "Delete Asset",
            Params = "{path, permanent?}: moves a file or folder of the project (relative to the project folder or absolute) to the Recycle Bin; permanent: true deletes it for good, for where there is no Recycle Bin. Never the project folder itself. A Rust script leaves the project's workspace with it",
            Run = Delete,
        },
        new()
        {
            Id = "asset.rename", Label = "Rename Asset",
            Params = "{path, name}: renames a file or folder of the project (relative to the project folder or absolute); a file keeps its extension when the name leaves it out; a Rust script's folder renames its crate, its built module and its workspace entry too. Never over another file. Returns {path}",
            Run = Rename,
        },
    ];

    /// <summary>Whether a name can name a file on every system: 1 to 100 characters, none of <c>/ \ : * ? " &lt; &gt; |</c> or control characters, and not only dots or spaces.</summary>
    internal static bool ValidFileName(string name)
    {
        var trimmed = name.Trim();
        return trimmed.Length > 0 && Text.Chars(trimmed) <= 100 && !trimmed.All(c => c is '.' or ' ')
            && !trimmed.EnumerateRunes().Any(r => r.Value is '/' or '\\' or ':' or '*' or '?' or '"' or '<' or '>' or '|' || System.Text.Rune.IsControl(r));
    }

    internal static bool Exists(string path) => File.Exists(path) || Directory.Exists(path);

    internal static string Canonical(string path) => Exists(path) ? Path.GetFullPath(path).TrimEnd(Path.DirectorySeparatorChar) : path;

    /// <summary>Whether <paramref name="inner"/> is <paramref name="outer"/> or inside it, by path parts.</summary>
    internal static bool StartsWith(string inner, string outer)
    {
        var (innerRoot, innerNames) = Files.Parts(inner);
        var (outerRoot, outerNames) = Files.Parts(outer);
        return innerRoot == outerRoot && innerNames.Count >= outerNames.Count && outerNames.SequenceEqual(innerNames.Take(outerNames.Count));
    }

    /// <summary>What a file or folder is, as the Project panel names it.</summary>
    private static string Kind(string path)
    {
        if (Directory.Exists(path))
        {
            return File.Exists(Path.Join(path, "Cargo.toml")) ? "Rust Script" : "Folder";
        }
        return Text.AsciiLower(Path.GetExtension(path).TrimStart('.')) switch
        {
            "bscene" => "Scene",
            "bprefab" => "Prefab",
            "bmaterial" => "Material",
            "bscript" => "Visual Script",
            "banim" => "Animation States",
            "wasm" => "Script",
            "binput" => "Input Map",
            "blight" => "Lighting",
            "bpost" => "Post-processing",
            "bui" => "Interface",
            "bstyle" => "Interface Style",
            "bshape" => "Data Shape",
            "bdata" => "Data File",
            "bproject" => "Project",
            var e when Loader.ModelExtensions.Contains(e) => "Model",
            var e when Loader.ImageExtensions.Contains(e) => "Image",
            _ => "File",
        };
    }

    private static JsonObject List(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "recursive");
        var given = f.Optional("path", JsonRead.Str, "");
        var recursive = f.Optional("recursive", JsonRead.Bool, false);
        var baseDir = session.ScriptDir;
        var top = session.Allowed(Path.Combine(baseDir, given), Access.Read);
        if (!Directory.Exists(top))
        {
            throw Stop.BadParams($"{top} is not a folder");
        }
        string Shown(string path) => LightingCommands.Relative(session, path);
        var entries = new JsonArray();
        var truncated = false;
        var pending = new Stack<string>([top]);
        while (pending.TryPop(out var dir))
        {
            List<string> here;
            try
            {
                here = Directory.EnumerateFileSystemEntries(dir)
                    .Where(e => !Path.GetFileName(e).StartsWith('.') && !(Directory.Exists(e) && NotListed.Contains(Path.GetFileName(e))))
                    .Where(e => !ImportSettings.IsSettingsFile(e))
                    .Order(StringComparer.Ordinal).ToList();
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                continue;
            }
            foreach (var path in here)
            {
                if (entries.Count >= MaxListed)
                {
                    truncated = true;
                    break;
                }
                var kind = Kind(path);
                if (recursive && kind == "Folder")
                {
                    pending.Push(path);
                }
                entries.Add(J.Obj(("path", J.S(Shown(path))), ("kind", J.S(kind))));
            }
        }
        return J.Obj(("entries", entries), ("truncated", J.B(truncated)));
    }

    private static void RefuseProject(Session session, string path, string what)
    {
        if (session.ProjectDir is { } project && StartsWith(Canonical(project), Canonical(path)))
        {
            throw Stop.BadParams($"the project folder itself, or a folder holding it, is {what}");
        }
    }

    private static JsonObject Delete(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "permanent");
        var given = f.Required("path", JsonRead.Str);
        var permanent = f.Optional("permanent", JsonRead.Bool, false);
        var path = session.Allowed(Path.Combine(session.ScriptDir, given), Access.Write);
        if (!Exists(path))
        {
            throw Stop.BadParams($"{path} does not exist");
        }
        RefuseProject(session, path, "never deleted");
        if (!permanent)
        {
            throw Stop.Failed($"cannot delete {path}: the C# engine has no Recycle Bin yet; with no Recycle Bin, delete with permanent: true");
        }
        try
        {
            if (Directory.Exists(path))
            {
                Directory.Delete(path, true);
            }
            else
            {
                File.Delete(path);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            throw Stop.Failed($"cannot delete {path}: {e.Message}");
        }
        // Its import settings go with it.
        var settings = ImportSettings.SettingsPath(path);
        if (File.Exists(settings))
        {
            try
            {
                File.Delete(session.Allowed(settings, Access.Write));
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException or CommandException)
            {
                // The settings stay; the asset itself is gone.
            }
        }
        return J.Obj(("path", J.S(path)), ("recycled", J.B(false)));
    }

    private static JsonObject Rename(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "name");
        var given = f.Required("path", JsonRead.Str);
        var name = f.Required("name", JsonRead.Str).Trim();
        var path = session.Allowed(Path.Combine(session.ScriptDir, given), Access.Write);
        if (!Exists(path))
        {
            throw Stop.BadParams($"{path} does not exist");
        }
        RefuseProject(session, path, "not renamed here");
        if (!ValidFileName(name))
        {
            throw Stop.BadParams(NameRule);
        }
        var extension = File.Exists(path) ? Path.GetExtension(path).TrimStart('.') : "";
        var file = extension.Length > 0 && !Text.AsciiLower(name).EndsWith($".{Text.AsciiLower(extension)}", StringComparison.Ordinal) ? $"{name}.{extension}" : name;
        var renamed = Path.Combine(Path.GetDirectoryName(path) ?? "", file);
        if (renamed == path)
        {
            return J.Obj(("path", J.S(path)));
        }
        // A name that differs only in case is a rename on any file system; anything else must be new.
        if (Exists(renamed) && !string.Equals(Canonical(renamed), Canonical(path), StringComparison.Ordinal))
        {
            throw Stop.Failed($"{renamed} already exists; pick another name");
        }
        renamed = session.Allowed(renamed, Access.Write);
        var wasFile = File.Exists(path);
        try
        {
            if (wasFile)
            {
                File.Move(path, renamed);
            }
            else
            {
                Directory.Move(path, renamed);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            throw Stop.Failed($"cannot rename {path}: {e.Message}");
        }
        // Its import settings go with it.
        var settings = ImportSettings.SettingsPath(path);
        if (wasFile && File.Exists(settings))
        {
            var moved = session.Allowed(ImportSettings.SettingsPath(renamed), Access.Write);
            try
            {
                File.Move(settings, moved);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                throw Stop.Failed($"cannot rename {settings}: {e.Message}");
            }
        }
        // The open scene follows its file.
        if (session.ScenePath is { } scene && StartsWith(scene, path))
        {
            var (_, rest) = Files.Parts(scene);
            var (_, prefix) = Files.Parts(path);
            session.SetScenePath(rest.Count == prefix.Count ? renamed : Path.Join([renamed, .. rest.Skip(prefix.Count)]));
        }
        return J.Obj(("path", J.S(renamed)));
    }

    private static JsonObject Create(Session session, JsonValue p)
    {
        var f = Params.Of(p, "kind", "folder", "name");
        var kind = Kinds[f.Required("kind", v => JsonRead.Variant(v, Kinds))];
        var folderParam = f.Required("folder", JsonRead.Str);
        var given = f.Required("name", JsonRead.Str);
        if (kind is "script")
        {
            // A C# script, as script.new makes one: its file is also the module objects run.
            var into = session.Allowed(Path.Combine(session.ScriptDir, folderParam), Access.Write);
            if (!Directory.Exists(into))
            {
                throw Stop.BadParams($"{into} is not a folder");
            }
            var script = ScriptCommands.Write(into, given);
            return J.Obj(("path", J.S(script)), ("module", J.S(script)));
        }
        if (!ValidFileName(given))
        {
            throw Stop.BadParams(NameRule);
        }
        var folder = session.Allowed(Path.Combine(session.ScriptDir, folderParam), Access.Write);
        if (!Directory.Exists(folder))
        {
            throw Stop.BadParams($"{folder} is not a folder");
        }
        var name = given.Trim();
        string? extension = kind switch
        {
            "scene" => SceneDoc.Extension,
            "material" => Material.Catalog.Extension,
            "visualScript" => Visual.Catalog.Extension,
            "inputMap" => InputAsset.Extension,
            "lighting" => Lighting.Extension,
            "postProcessing" => PostProcessing.Extension,
            "interface" => Layout.LayoutExtension,
            "interfaceStyle" => StyleSheet.StyleExtension,
            "dataShape" => DataShape.Extension,
            "dataFile" => DataFile.Extension,
            _ => null,
        };
        var file = extension is not null && !Text.AsciiLower(name).EndsWith($".{extension}", StringComparison.Ordinal) ? $"{name}.{extension}" : name;
        var path = Path.Combine(folder, file);
        if (Exists(path))
        {
            throw Stop.Failed($"{path} already exists; pick another name");
        }
        void Write(string text) => Files.Write(path, System.Text.Encoding.UTF8.GetBytes(text)).MapError(e => $"cannot write {path}: {e}").OrFail();
        switch (kind)
        {
            case "folder":
                Files.CreateDirectory(path).MapError(e => $"cannot write {path}: {e}").OrFail();
                break;
            case "scene":
                StarterScene().Save(path).OrFail();
                break;
            case "material":
                Write(Graph.Graph.Create(Material.Catalog).ToJson());
                break;
            case "visualScript":
                Write(Graph.Graph.Create(Visual.Catalog).ToJson());
                break;
            case "inputMap":
                Write(InputAsset.Starter().ToText());
                break;
            case "lighting":
                Write(Lighting.Default.ToText());
                break;
            case "postProcessing":
                Write(PostProcessing.Default.ToText());
                break;
            case "interface":
                Write(Layout.Starter().ToText());
                break;
            case "interfaceStyle":
                Write(StarterStyle().ToText());
                break;
            case "dataShape":
                Write(DataShape.Starter().ToText());
                break;
            default:
                Write(new DataFile("").ToText());
                break;
        }
        return J.Obj(("path", J.S(path)));
    }

    /// <summary>A new style file: one rule for buttons, to change from.</summary>
    private static StyleSheet StarterStyle()
    {
        var style = new StyleSheet();
        var set = new SortedDictionary<string, string>(Utf8Order.Instance) { ["background"] = "#2d3440e6", ["radius"] = "6px", ["padding"] = "8px 16px" };
        style.Rules.Add(new Rule(GameInterface.Kind.Button, null, null, set));
        return style;
    }

    /// <summary>A new scene: a camera looking at the centre and a sun, so it shows something when played.</summary>
    private static SceneModel StarterScene()
    {
        var scene = new SceneModel();
        var camera = scene.CreateObject(null, "Camera").OrFail();
        scene.Set(camera, Transform.Initial with { Position = new Double3(0, 2, 6), Rotation = new System.Numerics.Vector3(-15, 0, 0) }).OrFail();
        scene.Set(camera, Camera.Initial).OrFail();
        var sun = scene.CreateObject(null, "Sun").OrFail();
        scene.Set(sun, Transform.Initial with { Rotation = new System.Numerics.Vector3(-50, 30, 0) }).OrFail();
        scene.Set(sun, DirectionalLight.Initial).OrFail();
        return scene;
    }
}

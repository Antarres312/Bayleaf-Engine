using Bayleaf.Assets;
using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene;

namespace Bayleaf.Engine;

/// <summary>
/// Bringing files into a project: <c>asset.import</c> copies models, images and sounds from outside
/// (a drop from the system, or an agent) into a folder of the project, checking first that each can
/// be read, so nothing half-imported stays behind. Models and images get import settings beside them
/// (<c>asset.importSettings.get</c>/<c>.set</c>). <c>scene.file.place</c> puts a model or sound in
/// the scene as an object.
/// </summary>
internal static class ImportCommands
{
    /// <summary>Most files one <c>asset.import</c> brings in.</summary>
    public const int MaxImported = 256;

    /// <summary>What a file brought in is.</summary>
    internal enum FileKind
    {
        Model,
        Image,
        Sound,
    }

    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "asset.import", Label = "Import Files",
            Params = "{files: [path], folder?}: copies models (.glb, .gltf with the files it names), images (.png, .jpg) and sounds into a folder of the project (relative to the project folder; the project folder when left out), each read first, so a file that cannot be read says why and nothing is copied. A name already taken gets a number. Models and images get import settings beside them. A file already in the project stays where it is. Returns {imported: [{path, kind, source}]}, paths relative to the project folder",
            Run = Import,
        },
        new()
        {
            Id = "asset.importSettings.get", Label = "Import Settings",
            Params = "{path}: how a model or image of the project is read: {settings: {model?: {scale, upAxis: \"y\" | \"z\"}, image?: {mipmaps}}, saved}; saved is false while it has only the defaults",
            Run = SettingsGet,
        },
        new()
        {
            Id = "asset.importSettings.set", Label = "Set Import Settings",
            Params = "{path, settings: {model?: {scale, upAxis}, image?: {mipmaps}}}: saves how a model or image is read, beside it; scenes using it read it again. Returns {path} of the settings file",
            Run = SettingsSet,
        },
        new()
        {
            Id = "scene.file.place", Label = "Place File",
            Params = "{path, parent?, index?, position?: [x, y, z]}: a model or a sound of the project as a new object named after it (a Mesh drawing the model, or an Audio Source playing the sound), at position (world units; the origin when left out). Returns {id}",
            Mutates = Mutates.Scene, Run = Place,
        },
    ];

    /// <summary>What <paramref name="path"/> is, by its extension; null for a file that cannot be brought in.</summary>
    internal static FileKind? KindOf(string path)
    {
        var extension = ExtensionOf(path);
        if (extension is null)
        {
            return null;
        }
        extension = Text.AsciiLower(extension);
        if (Loader.ModelExtensions.Contains(extension))
        {
            return FileKind.Model;
        }
        if (Loader.ImageExtensions.Contains(extension))
        {
            return FileKind.Image;
        }
        return ImportSettings.SoundExtensions.Contains(extension) ? FileKind.Sound : null;
    }

    private static string Word(FileKind kind) => kind switch
    {
        FileKind.Model => "model",
        FileKind.Image => "image",
        _ => "sound",
    };

    /// <summary>The file name: what follows the last separator.</summary>
    private static string NameOf(string path) => Path.GetFileName(path.TrimEnd('/', '\\'));

    /// <summary>The extension as Rust's <c>Path::extension</c> gives it: none without a dot or for a name starting with its only dot.</summary>
    private static string? ExtensionOf(string path)
    {
        var name = NameOf(path);
        var dot = name.LastIndexOf('.');
        return name is "" or ".." || dot <= 0 ? null : name[(dot + 1)..];
    }

    /// <summary>The name without its extension, as Rust's <c>Path::file_stem</c> gives it.</summary>
    private static string? StemOf(string path)
    {
        var name = NameOf(path);
        if (name is "" or "..")
        {
            return null;
        }
        var dot = name.LastIndexOf('.');
        return dot <= 0 ? name : name[..dot];
    }

    /// <summary>Why a file cannot be brought in, by its extension.</summary>
    private static string NotImportable(string path)
    {
        var name = NameOf(path) is { Length: > 0 } n ? n : path;
        var extension = Text.AsciiLower(ExtensionOf(path) ?? "");
        return extension switch
        {
            "fbx" or "dae" or "blend" or "3ds" or "stl" or "ply" =>
                $"{name}: .{extension} models are not read yet; export the model as glTF (.glb) or OBJ and bring that in",
            "ktx2" or "hdr" or "webp" or "tga" or "bmp" or "tif" or "tiff" or "gif" or "dds" or "psd" =>
                $"{name}: .{extension} images are not read yet; save it as PNG, JPEG or OpenEXR",
            "" => $"{name}: a file without an extension cannot be brought in",
            _ => $"{name}: .{extension} files cannot be brought in; models are glTF (.glb, .gltf) or OBJ, images PNG, JPEG or OpenEXR, sounds "
                + string.Join(", ", ImportSettings.SoundExtensions.Select(e => e.ToUpperInvariant())),
        };
    }

    /// <summary>Reads the file as the engine would, so a broken one is refused before anything is copied.</summary>
    private static string? Unreadable(string path, FileKind kind)
    {
        switch (kind)
        {
            case FileKind.Model:
                return Loader.LoadModel(path).TryGet(out _, out var modelError) ? null : modelError;
            case FileKind.Image:
                return Loader.LoadImage(path).TryGet(out _, out var imageError) ? null : imageError;
            default:
                // Without sound built in, a sound is only checked for being a readable file of a fitting size.
                var size = Files.Length(path);
                if (size.IsError)
                {
                    return $"cannot read {path}: {size.Error}";
                }
                return size.Value > Loader.MaxFileBytes ? $"{path} is larger than {Loader.MaxFileBytes >> 20} MiB" : null;
        }
    }

    /// <summary><paramref name="path"/> as scenes name it: relative to the project folder, with <c>/</c>.</summary>
    internal static string Relative(Session session, string path)
    {
        var (dirRoot, dirNames) = Files.Parts(AssetCommands.Canonical(session.ScriptDir));
        var full = AssetCommands.Canonical(path);
        var (root, names) = Files.Parts(full);
        var inside = dirRoot == root && names.Count >= dirNames.Count && dirNames.SequenceEqual(names.Take(dirNames.Count));
        return (inside ? string.Join('/', names.Skip(dirNames.Count)) : full).Replace('\\', '/');
    }

    /// <summary>Whether <paramref name="path"/> lies inside the project folder.</summary>
    private static bool InProject(Session session, string path) =>
        session.ProjectDir is { } project && AssetCommands.StartsWith(AssetCommands.Canonical(path), AssetCommands.Canonical(project));

    /// <summary>A path in <paramref name="folder"/> named <paramref name="stem"/> (and <paramref name="extension"/>, if any) that is not taken: "Crate", then "Crate 2", "Crate 3"…</summary>
    internal static string FreePath(string folder, string stem, string? extension)
    {
        for (var n = 1; n < 1000; n++)
        {
            var named = n == 1 ? stem : $"{stem} {n}";
            var path = Path.Join(folder, extension is null ? named : $"{named}.{extension}");
            if (!AssetCommands.Exists(path) && !AssetCommands.Exists(ImportSettings.SettingsPath(path)))
            {
                return path;
            }
        }
        throw Stop.Failed($"{folder} has too many files named {stem}");
    }

    /// <summary>One file to bring in, once checked: where from, its kind, and for a model the files it names, relative to its folder.</summary>
    private sealed record Planned(string Source, FileKind Kind, IReadOnlyList<string> Beside);

    private static JsonObject Import(Session session, JsonValue p)
    {
        var f = Params.Of(p, "files", "folder");
        var files = f.Required("files", v => JsonRead.List(v, JsonRead.Str));
        var folderParam = f.Optional("folder", JsonRead.Str, "");
        if (files.Count == 0)
        {
            throw Stop.BadParams("name at least one file");
        }
        if (files.Count > MaxImported)
        {
            throw Stop.BadParams($"at most {MaxImported} files at once");
        }
        var folder = session.Allowed(Path.Combine(session.ScriptDir, folderParam), Access.Write);
        if (!Directory.Exists(folder))
        {
            throw Stop.BadParams($"{folder} is not a folder");
        }
        // Everything is checked before anything is copied.
        var planned = new List<Planned>();
        foreach (var file in files)
        {
            // A relative path starts at the project folder, like every other path agents give.
            var source = session.Allowed(Path.Combine(session.ScriptDir, file), Access.Read);
            if (!File.Exists(source))
            {
                throw Stop.BadParams($"{source} is not a file");
            }
            var kind = KindOf(source) ?? throw Stop.Failed(NotImportable(source));
            if (Unreadable(source, kind) is { } problem)
            {
                throw Stop.Failed(problem);
            }
            IReadOnlyList<string> beside = kind == FileKind.Model ? Loader.ModelFiles(source).OrFail() : [];
            var sourceFolder = Path.GetDirectoryName(source) ?? ".";
            foreach (var extra in beside)
            {
                session.Allowed(Path.Join(sourceFolder, extra), Access.Read);
            }
            planned.Add(new Planned(source, kind, beside));
        }
        var created = new List<string>();
        var imported = new JsonArray();
        try
        {
            foreach (var plan in planned)
            {
                // Already in the project: it stays, and gains settings if it has none.
                var path = InProject(session, plan.Source) ? plan.Source : CopyIn(session, plan, folder, created);
                if (ImportSettings.DefaultsFor(path) is { } settings && !AssetCommands.Exists(ImportSettings.SettingsPath(path)))
                {
                    var at = session.Allowed(ImportSettings.SettingsPath(path), Access.Write);
                    Files.Write(at, System.Text.Encoding.UTF8.GetBytes(settings.ToText())).MapError(e => $"cannot write {at}: {e}").OrFail();
                    created.Add(at);
                }
                imported.Add(J.Obj(("path", J.S(Relative(session, path))), ("kind", J.S(Word(plan.Kind))), ("source", J.S(plan.Source))));
            }
        }
        catch (CommandException)
        {
            // Nothing half-imported stays: newest first, so folders are empty when their turn comes.
            foreach (var path in Enumerable.Reverse(created))
            {
                try
                {
                    if (Directory.Exists(path))
                    {
                        Directory.Delete(path);
                    }
                    else
                    {
                        File.Delete(path);
                    }
                }
                catch (Exception e) when (e is IOException or UnauthorizedAccessException)
                {
                    // What cannot be taken away stays; the first problem is the one to tell.
                }
            }
            throw;
        }
        return J.Obj(("imported", imported));
    }

    /// <summary>
    /// Copies one checked file (and what it names beside it) into <paramref name="folder"/>; a model
    /// with files beside it gets a folder of its own, so their names cannot clash with others.
    /// </summary>
    private static string CopyIn(Session session, Planned plan, string folder, List<string> created)
    {
        void Copy(string from, string to)
        {
            try
            {
                File.Copy(from, to);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                throw Stop.Failed($"cannot copy {from}: {e.Message}");
            }
        }
        var stem = StemOf(plan.Source) ?? "Imported";
        var fileName = NameOf(plan.Source);
        var extension = ExtensionOf(plan.Source);
        if (plan.Beside.Count == 0)
        {
            var to = session.Allowed(FreePath(folder, stem, extension), Access.Write);
            Copy(plan.Source, to);
            created.Add(to);
            return to;
        }
        var own = session.Allowed(FreePath(folder, stem, null), Access.Write);
        Files.CreateDirectory(own).MapError(e => $"cannot copy {own}: {e}").OrFail();
        created.Add(own);
        var model = Path.Join(own, fileName);
        Copy(plan.Source, model);
        created.Add(model);
        var sourceFolder = Path.GetDirectoryName(plan.Source) ?? ".";
        foreach (var extra in plan.Beside)
        {
            var target = session.Allowed(Path.Join(own, extra), Access.Write);
            // Folders the model names its files in, made as needed.
            var missing = new List<string>();
            for (var dir = Path.GetDirectoryName(target); dir is not null && !AssetCommands.Exists(dir); dir = Path.GetDirectoryName(dir))
            {
                missing.Add(dir);
            }
            missing.Reverse();
            foreach (var dir in missing)
            {
                Files.CreateDirectory(dir).MapError(e => $"cannot copy {dir}: {e}").OrFail();
                created.Add(dir);
            }
            Copy(Path.Join(sourceFolder, extra), target);
            created.Add(target);
        }
        return model;
    }

    /// <summary>A model or image of the project, checked for reading (or writing its settings).</summary>
    private static string SettingsTarget(Session session, string path, Access access)
    {
        var file = session.Allowed(Path.Combine(session.ScriptDir, path), Access.Read);
        if (!File.Exists(file))
        {
            throw Stop.BadParams($"{file} is not a file");
        }
        if (ImportSettings.DefaultsFor(file) is null)
        {
            throw Stop.BadParams($"{file} has no import settings: only models and images do");
        }
        session.Allowed(ImportSettings.SettingsPath(file), access);
        return file;
    }

    private static JsonObject SettingsGet(Session session, JsonValue p)
    {
        var file = SettingsTarget(session, Params.Of(p, "path").Required("path", JsonRead.Str), Access.Read);
        var settings = ImportSettings.ReadFor(file).OrFail();
        return J.Obj(("settings", settings is null ? JsonNull.Instance : settings.ToJson()), ("saved", J.B(File.Exists(ImportSettings.SettingsPath(file)))));
    }

    private static JsonObject SettingsSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "settings");
        var file = SettingsTarget(session, f.Required("path", JsonRead.Str), Access.Write);
        var given = f.Required("settings", v => v);
        var settings = ImportSettings.DefaultsFor(file) ?? throw Stop.Failed("no import settings for this file");
        if (given is not JsonObject sections)
        {
            throw Stop.BadParams("settings must be an object");
        }
        // Only the section for the file's kind; it may leave out values, which keep the defaults.
        foreach (var (key, value) in sections.OrderBy(e => e.Key, Utf8Order.Instance))
        {
            settings = key switch
            {
                "model" when settings.Model is not null => settings with { Model = Merged(key, settings.Model.Write(), value, ModelImport.Read) },
                "image" when settings.Image is not null => settings with { Image = Merged(key, settings.Image.Write(), value, ImageImport.Read) },
                "sound" when settings.Sound is not null => settings with { Sound = Merged(key, settings.Sound.Write(), value, SoundImport.Read) },
                _ => throw Stop.BadParams($"{file} has no {Text.Debug(key)} import settings"),
            };
        }
        if (settings.Validate().TryGet(out _, out var invalid) is false)
        {
            throw Stop.BadParams(invalid);
        }
        if (settings.Image is { Compression: not Compression.None })
        {
            var bytes = Files.Read(file).MapError(e => $"cannot read {file}: {e}").OrFail();
            var pixels = Loader.DecodeImage(bytes).MapError(e => $"{file}: {e}").OrFail();
            if (pixels.Width % 4 != 0 || pixels.Height % 4 != 0)
            {
                throw Stop.BadParams($"block compression needs sides that are multiples of 4; {file} is {pixels.Width} x {pixels.Height}");
            }
        }
        // Without sound built in, nothing plays, so streaming is not refused.
        var path = ImportSettings.SettingsPath(file);
        Files.Write(path, System.Text.Encoding.UTF8.GetBytes(settings.ToText())).MapError(e => $"cannot write {path}: {e}").OrFail();
        return J.Obj(("path", J.S(path)));
    }

    /// <summary>The section <paramref name="current"/> with each field of <paramref name="given"/> set on it, read back.</summary>
    private static T Merged<T>(string section, JsonObject current, JsonValue given, Func<JsonValue, T> read)
    {
        Merge(current, given);
        try
        {
            return read(current);
        }
        catch (JsonShapeException e)
        {
            throw Stop.BadParams($"{section}: {e.Message}");
        }
    }

    /// <summary>Sets each field of <paramref name="from"/> on <paramref name="into"/> (both objects).</summary>
    private static void Merge(JsonValue into, JsonValue from)
    {
        if (into is not JsonObject target || from is not JsonObject source)
        {
            throw Stop.BadParams("each part of the settings must be an object");
        }
        foreach (var (key, value) in source.OrderBy(e => e.Key, Utf8Order.Instance))
        {
            target.Set(key, value);
        }
    }

    private static JsonObject Place(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "parent", "index", "position");
        var given = f.Required("path", JsonRead.Str);
        var parent = Params.OptionalId(f, "parent");
        int? index = f.NullableValue("index", Params.Index);
        var position = f.Nullable("position", v => JsonRead.Fixed(v, 3, JsonRead.F64));
        var file = session.Allowed(Path.Combine(session.ScriptDir, given), Access.Read);
        if (!File.Exists(file))
        {
            throw Stop.BadParams($"{file} is not a file");
        }
        if (position is not null && !position.All(c => double.IsFinite(c) && Math.Abs(c) <= 1e12))
        {
            throw Stop.BadParams("position must be three finite numbers");
        }
        var name = StemOf(file) ?? "Object";
        var named = Relative(session, file);
        var (component, field) = KindOf(file) switch
        {
            FileKind.Model => ("mesh", "model"),
            FileKind.Sound => ("audioSource", "clip"),
            FileKind.Image => throw Stop.BadParams("an image is not placed on its own: apply it to an object with scene.image.apply"),
            _ => throw Stop.BadParams($"{file} is not a model or a sound"),
        };
        var scene = session.Scene;
        var id = scene.CreateObject(parent, name).OrFail();
        try
        {
            if (index is not null)
            {
                scene.Reparent(id, parent, index).OrFail();
            }
            scene.AddComponent(id, component).OrFail();
            var current = scene.Component(id, component).OrFail();
            Merge(current, J.Obj((field, J.S(named))));
            scene.SetComponent(id, component, current).OrFail();
            if (position is not null)
            {
                var transform = scene.Component(id, "transform").OrFail();
                if (transform is JsonObject t)
                {
                    t.Set("position", new JsonArray(position.Select(c => (JsonValue)JsonNumber.Of(c))));
                }
                scene.SetComponent(id, "transform", transform).OrFail();
            }
        }
        catch (CommandException)
        {
            scene.DeleteObject(id);
            throw;
        }
        return J.Obj(("id", J.Id(id)));
    }
}

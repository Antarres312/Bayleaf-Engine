using System.Globalization;
using System.Numerics;
using System.Text;

using Bayleaf.Assets;
using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.GameInterface;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;
using Bayleaf.Script;

using GraphModel = Bayleaf.Graph.Graph;
using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Engine;

/// <summary>
/// <c>asset.info</c>: what a project file is and holds, for the Inspector and for agents. Reads only;
/// each kind of file says what matters about it (an image its size, a scene its objects, a graph its
/// nodes and problems, a text file its first lines).
/// </summary>
/// <remarks>
/// Two kinds differ from the Rust engine, whose script host runs a script's describe step: a visual
/// script's properties are the ones its graph declares (Property nodes that name a property the graph
/// does not declare are left out, and its problems say it is not compiled yet), and a WebAssembly
/// module is not described, since the C# engine's scripts are C#. A C# script (<c>.cs</c>) is a
/// "Script" with the properties its source declares, read by compiling the project's scripts.
/// </remarks>
internal static class AssetInfoCommands
{
    /// <summary>Text shown from a text file at most.</summary>
    public const int TextPreviewBytes = 4096;

    /// <summary>File extensions shown as text.</summary>
    private static readonly string[] TextExtensions = ["txt", "md", "rs", "toml", "json", "wgsl", "wat", "ron", "yaml", "yml", "csv", "gitignore"];

    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "asset.info", Label = "File Info",
            Params = "{path}: a file or folder, relative to the project folder or absolute; returns {path, kind, bytes, rows: [[label, value]], problems, text?}",
            Run = Info,
        },
    ];

    /// <summary>What is known about one file.</summary>
    internal sealed class Described(string kind)
    {
        /// <summary>What kind of file it is, such as "Image".</summary>
        public string Kind { get; set; } = kind;

        public long Bytes { get; set; }

        /// <summary>Label and value pairs, in the order to show them.</summary>
        public List<(string Label, string Value)> Rows { get; } = [];

        /// <summary>What is wrong with the file, if anything.</summary>
        public List<string> Problems { get; } = [];

        /// <summary>The start of a text file.</summary>
        public string? Text { get; set; }

        public void Row(string label, string value) => Rows.Add((label, value));

        public void Row(string label, long value) => Rows.Add((label, value.ToString(CultureInfo.InvariantCulture)));

        public JsonObject ToJson(string path) => J.Obj(
            ("path", J.S(path)),
            ("kind", J.S(Kind)),
            ("bytes", J.N(Bytes)),
            ("rows", new JsonArray(Rows.Select(r => (JsonValue)J.Strings([r.Label, r.Value])))),
            ("problems", J.Strings(Problems)),
            ("text", J.S(Text)));
    }

    private static JsonObject Info(Session session, JsonValue p)
    {
        var given = Params.Of(p, "path").Required("path", JsonRead.Str);
        // An empty path names the project folder itself, written with a separator at the end as the Rust engine's join writes it.
        var dir = session.ScriptDir;
        var joined = given.Length == 0 && !Path.EndsInDirectorySeparator(dir) ? dir + Path.DirectorySeparatorChar : Path.Combine(dir, given);
        var path = session.Allowed(joined, Access.Read);
        return Describe(session, path).ToJson(path);
    }

    /// <summary>Reads <paramref name="path"/> (already allowed) and says what it is.</summary>
    internal static Described Describe(Session session, string path)
    {
        if (Directory.Exists(path))
        {
            return Folder(path);
        }
        if (!File.Exists(path))
        {
            var missing = new Described("Missing");
            missing.Problems.Add($"{path} does not exist");
            return missing;
        }
        long bytes;
        try
        {
            bytes = new System.IO.FileInfo(path).Length;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            var missing = new Described("Missing");
            missing.Problems.Add($"{path} does not exist");
            return missing;
        }
        var extension = Text.AsciiLower(GraphCommands.ExtensionOf(path) ?? "");
        var name = Path.GetFileName(path);
        var result = extension switch
        {
            var e when Loader.ImageExtensions.Contains(e) => Image(path),
            var e when Loader.ModelExtensions.Contains(e) => Model(path),
            "bscene" => SceneFile(path),
            "bprefab" => Prefab(session, path),
            "bmaterial" or "bscript" or "banim" => GraphFile(path, extension),
            "wasm" => Module(),
            "cs" => CSharpScript(session, path),
            "bproject" => Project(path),
            "binput" => InputMapFile(session, path),
            "bui" => Interface(session, path),
            "bstyle" => InterfaceStyle(path),
            "bshape" => DataShapeFile(path),
            "bdata" => DataFileInfo(session, path),
            var e when TextExtensions.Contains(e) || name.StartsWith('.') => TextFile(path, "Text"),
            _ => new Described("File"),
        };
        result.Bytes = bytes;
        return result;
    }

    private static Described Folder(string path)
    {
        var cargo = Path.Join(path, "Cargo.toml");
        if (File.Exists(cargo))
        {
            var source = TextFile(cargo, "Script Source");
            source.Row("Build", "Right-click the folder in the Project panel and pick Build Script");
            return source;
        }
        var info = new Described("Folder");
        try
        {
            var (files, folders) = (0, 0);
            foreach (var entry in Directory.EnumerateFileSystemEntries(path))
            {
                if (Directory.Exists(entry) && !File.GetAttributes(entry).HasFlag(FileAttributes.ReparsePoint))
                {
                    folders++;
                }
                else
                {
                    files++;
                }
            }
            info.Row("Files", files);
            info.Row("Folders", folders);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            info.Problems.Add($"cannot read the folder: {e.Message}");
        }
        return info;
    }

    private static Described Image(string path)
    {
        var info = new Described("Image");
        if (!Loader.LoadImage(path).TryGet(out var image, out var error))
        {
            info.Problems.Add(error);
            return info;
        }
        info.Row("Size", $"{image.Width} × {image.Height} pixels");
        var opaque = true;
        for (var i = 3; i < image.Rgba.Length; i += 4)
        {
            opaque &= image.Rgba[i] == 255;
        }
        info.Row("Transparency", opaque ? "None" : "Yes");
        var asked = (ImportSettings.ReadFor(path) is { IsOk: true } read ? read.Value?.Image : null) ?? ImageImport.Default;
        if (image.Data)
        {
            info.Row("Pixels", "Plain numbers, read as stored");
        }
        if (image.Blocks is { } blocks)
        {
            info.Row("Compression", $"{(blocks.Compression == Compression.Bc3 ? "BC3" : "BC1")}, where the graphics device reads it");
        }
        else if (asked.Compression != Compression.None)
        {
            info.Problems.Add("kept uncompressed: block compression needs sides that are multiples of 4");
        }
        if (string.Equals(GraphCommands.ExtensionOf(path), "exr", StringComparison.OrdinalIgnoreCase))
        {
            info.Row("Range", "Kept at 8 bits per channel: light brighter than 1 shows as white");
        }
        return info;
    }

    private static Described Model(string path)
    {
        var info = new Described("Model");
        if (!Loader.LoadModel(path).TryGet(out var mesh, out var error))
        {
            info.Problems.Add(error);
            return info;
        }
        info.Row("Vertices", mesh.Positions.Count);
        info.Row("Triangles", mesh.Indices.Count / 3);
        var size = mesh.Max - mesh.Min;
        info.Row("Size", $"{Fixed2(size.X)} × {Fixed2(size.Y)} × {Fixed2(size.Z)} m");
        info.Row("Texture coordinates", mesh.Uvs.Count == 0 ? "No" : "Yes");
        if (mesh.Parts.Count > 1)
        {
            info.Row("Parts", $"{mesh.Parts.Count}, each with its own material");
        }
        if (mesh.Material is { } m)
        {
            var image = m.Image is { } i ? $"a {i.Width} × {i.Height} image" : "no image";
            var normal = m.Normal is not null ? ", a normal map" : "";
            info.Row("Material", $"its own colour, {image}{normal}, metallic {Fixed2(m.Metallic)}, roughness {Fixed2(m.Roughness)}");
        }
        if (mesh.Skeleton is { Animations.Count: > 0 } skeleton)
        {
            info.Row("Animations", string.Join(", ", skeleton.Animations.Select(a => $"{a.Name} ({Fixed2(a.Duration)} s)")));
        }
        info.Problems.AddRange(mesh.Warnings);
        return info;
    }

    private static Described SceneFile(string path)
    {
        var info = new Described("Scene");
        if (!SceneModel.Load(path).TryGet(out var scene, out var error))
        {
            info.Problems.Add(error.Message);
            return info;
        }
        info.Row("Objects", scene.ObjectCount);
        info.Row("Top level", string.Join(", ", scene.Children(null).Select(scene.NameOf).OfType<string>().Take(8)));
        return info;
    }

    private static Described Prefab(Session session, string path)
    {
        var info = new Described("Prefab");
        try
        {
            var objects = PrefabCommands.ReadPrefab(session, path);
            var root = objects.Count > 0 && objects[0].Components.GetValueOrDefault("name") is JsonString s ? s.Value : "Object";
            info.Row("Root", root);
            info.Row("Objects", objects.Count);
        }
        catch (CommandException e)
        {
            info.Problems.Add(e.Error.Message);
        }
        return info;
    }

    private static Described GraphFile(string path, string extension)
    {
        var info = new Described(extension switch
        {
            "bmaterial" => "Material",
            "banim" => "Animation States",
            _ => "Visual Script",
        });
        if (GraphCommands.CatalogForPath(path) is not { } catalog)
        {
            return info;
        }
        if (!Files.ReadText(path).TryGet(out var text, out var unread))
        {
            info.Problems.Add(unread);
            return info;
        }
        if (!GraphModel.FromJson(text, catalog).TryGet(out var graph, out var refused))
        {
            info.Problems.Add(refused.Message);
            return info;
        }
        info.Row("Nodes", graph.Nodes.Count);
        info.Row("Links", graph.Links.Count);
        info.Problems.AddRange(GraphCommands.Problems(graph).Select(p => p.Message));
        if (extension == "bscript")
        {
            // The Rust engine runs the compiled script's describe step; the graph declares the same
            // properties, and the host's check of their Inspector options refuses the same scripts.
            if (Visual.CheckInspector(graph).TryGet(out _, out var layout) is false)
            {
                info.Problems.Add($"{Path.GetFileName(path)} is not a valid script: bayleaf_describe: inspector: {layout}");
                return info;
            }
            var declared = Visual.DeclaredProperties(graph);
            info.Row("Properties", declared.Count == 0
                ? "None"
                : string.Join(", ", declared.Select(d => $"{d.Name} ({d.Type.Code()}) = {d.Default.Write().ToCompact()}")));
        }
        return info;
    }

    /// <summary>
    /// A C# script: what it declares, read by compiling the project's scripts (no script code runs): its
    /// properties with their types and defaults, as the Rust engine lists a module's, and where Add
    /// Component lists it.
    /// </summary>
    private static Described CSharpScript(Session session, string path)
    {
        var info = new Described("Script");
        var folder = Path.GetFullPath(session.ScriptDir);
        var full = Path.GetFullPath(path);
        if (!AssetCommands.StartsWith(full, folder))
        {
            info.Problems.Add($"{path} is outside the project folder, whose scripts compile together");
            return info;
        }
        var described = session.CompiledScripts().Describe(Path.GetRelativePath(folder, full));
        if (described.IsError)
        {
            info.Problems.Add(described.Error);
            return info;
        }
        var script = described.Value;
        info.Row("Properties", script.Properties.Count == 0
            ? "None"
            : string.Join(", ", script.Properties.Select(d => $"{d.Name} ({d.Type.Code()}) = {d.Default.Write().ToCompact()}")));
        if (script.Menu is { } menu)
        {
            info.Row("Menu", menu);
        }
        if (script.Events.Count > 0)
        {
            info.Row("Events", string.Join(", ", script.Events.Select(e => e.Values.Count == 0 ? e.Name : $"{e.Name} ({string.Join(", ", e.Values)})")));
        }
        info.Problems.AddRange(script.Problems);
        return info;
    }

    /// <summary>A WebAssembly module: the C# engine's scripts are C#, so it is not described (or run).</summary>
    private static Described Module()
    {
        var info = new Described("Script");
        info.Problems.Add("the C# engine's scripts are C#: a WebAssembly module (.wasm) is not read or described here; the Rust engine (rust/) runs it");
        return info;
    }

    private static Described Project(string path)
    {
        var info = TextFile(path, "Project");
        if (Files.ReadText(path) is { IsOk: true } text && JsonReader.Parse(text.Value) is { IsOk: true } file)
        {
            info.Rows.Add(("Name", file.Value.Get("name")?.AsString() ?? ""));
            info.Rows.Add(("Main scene", file.Value.Get("mainScene")?.AsString() ?? ""));
            info.Text = null;
        }
        return info;
    }

    /// <summary>An Input Map: its maps, the one play starts with, and how many actions; or what is wrong.</summary>
    private static Described InputMapFile(Session session, string path)
    {
        var info = new Described("Input Map");
        if (!InputCommands.ReadAsset(session, path).TryGet(out var asset, out var error) || !asset.CheckedMaps().TryGet(out var maps, out error))
        {
            info.Problems.Add(error);
            return info;
        }
        info.Row("Maps", string.Join(", ", maps));
        info.Row("Starts with", asset.Start.Length == 0 ? (maps.Count > 0 ? maps[0] : "") : asset.Start);
        info.Row("Actions", asset.Maps.Sum(m => m.Actions.Count));
        return info;
    }

    /// <summary>An interface layout: its elements and style files, and whether they all read.</summary>
    private static Described Interface(Session session, string path)
    {
        var info = new Described("Interface");
        if (!Layout.Load(path).TryGet(out var layout, out var error))
        {
            info.Problems.Add(error);
            return info;
        }
        var elements = layout.Root.Walk();
        info.Row("Elements", elements.Count);
        info.Row("Named", elements.Count(e => e.Name.Length > 0));
        info.Row("Style files", layout.Styles.Count == 0 ? "None" : string.Join(", ", layout.Styles));
        foreach (var style in layout.Styles)
        {
            if (StyleSheet.Load(Path.Combine(session.ScriptDir, style)).TryGet(out _, out var problem) is false)
            {
                info.Problems.Add(problem);
            }
        }
        return info;
    }

    /// <summary>An interface style file: how many rules it has, and whether it reads.</summary>
    private static Described InterfaceStyle(string path)
    {
        var info = new Described("Interface Style");
        if (StyleSheet.Load(path).TryGet(out var style, out var error))
        {
            info.Row("Rules", style.Rules.Count);
        }
        else
        {
            info.Problems.Add(error);
        }
        return info;
    }

    /// <summary>A data shape: how many fields it declares, and their names.</summary>
    private static Described DataShapeFile(string path)
    {
        var info = new Described("Data Shape");
        if (DataShape.Load(path).TryGet(out var shape, out var error))
        {
            info.Row("Fields", shape.Fields.Count);
            info.Row("Names", string.Join(", ", shape.Fields.Select(f => f.Name)));
        }
        else
        {
            info.Problems.Add(error.Message);
        }
        return info;
    }

    /// <summary>A data file: its shape, how many values it sets, and what does not read as written.</summary>
    private static Described DataFileInfo(Session session, string path)
    {
        var info = new Described("Data File");
        if (!DataFile.Load(path).TryGet(out var data, out var error))
        {
            info.Problems.Add(error.Message);
            return info;
        }
        info.Row("Shape", data.Shape.Length == 0 ? "None" : data.Shape);
        info.Row("Values set", data.Values.Count);
        if (DataCommands.ShapeOf(session, data).TryGet(out var shape, out var problem))
        {
            info.Problems.AddRange(data.Problems(shape));
        }
        else
        {
            info.Problems.Add(problem);
        }
        return info;
    }

    /// <summary>The start of a text file, cut at a character boundary.</summary>
    private static Described TextFile(string path, string kind)
    {
        var info = new Described(kind);
        byte[] bytes;
        try
        {
            using var file = File.OpenRead(path);
            bytes = new byte[TextPreviewBytes + 1];
            var length = 0;
            int read;
            while (length < bytes.Length && (read = file.Read(bytes, length, bytes.Length - length)) > 0)
            {
                length += read;
            }
            Array.Resize(ref bytes, length);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            info.Problems.Add($"cannot read the file: {e.Message}");
            return info;
        }
        var cut = bytes.Length > TextPreviewBytes;
        var kept = bytes.AsSpan(0, Math.Min(bytes.Length, TextPreviewBytes));
        var chars = new char[kept.Length];
        // A cut in the middle of a character keeps the whole characters before it; bytes that are no text anywhere else are refused.
        var status = System.Text.Unicode.Utf8.ToUtf16(kept, chars, out _, out var written, replaceInvalidSequences: false, isFinalBlock: false);
        if (status == System.Buffers.OperationStatus.InvalidData)
        {
            info.Problems.Add("this is not a text file");
            return info;
        }
        var text = new string(chars, 0, written);
        info.Text = cut ? $"{text}\n…" : text;
        return info;
    }

    /// <summary>A number with two decimals, as Rust's <c>{:.2}</c> writes it: the exact value rounded half to even.</summary>
    internal static string Fixed2(float value)
    {
        if (float.IsNaN(value))
        {
            return "NaN";
        }
        if (float.IsInfinity(value))
        {
            return value > 0 ? "inf" : "-inf";
        }
        var bits = BitConverter.DoubleToInt64Bits(value);
        var negative = bits < 0;
        var exponent = (int)((bits >> 52) & 0x7FF);
        var mantissa = bits & 0xFFFFFFFFFFFFFL;
        if (exponent == 0)
        {
            exponent = 1;
        }
        else
        {
            mantissa |= 1L << 52;
        }
        exponent -= 1075;
        // value = mantissa × 2^exponent; hundredths = round(value × 100), ties to even.
        BigInteger hundredths;
        if (exponent >= 0)
        {
            hundredths = (new BigInteger(mantissa) << exponent) * 100;
        }
        else
        {
            var numerator = new BigInteger(mantissa) * 100;
            var denominator = BigInteger.One << -exponent;
            var quotient = BigInteger.DivRem(numerator, denominator, out var remainder);
            var twice = remainder * 2;
            if (twice > denominator || (twice == denominator && !quotient.IsEven))
            {
                quotient += 1;
            }
            hundredths = quotient;
        }
        var whole = BigInteger.DivRem(hundredths, 100, out var part);
        var builder = new StringBuilder();
        if (negative)
        {
            builder.Append('-');
        }
        builder.Append(whole.ToString(CultureInfo.InvariantCulture)).Append('.').Append(((int)part).ToString("00", CultureInfo.InvariantCulture));
        return builder.ToString();
    }
}

using System.Globalization;
using System.Text;

namespace Bayleaf.Assets.Wavefront;

/// <summary>
/// Wavefront OBJ geometry and MTL materials, read (ported from craft-mesh, storytold/craft-libs,
/// Copyright (c) 2026 ArtCraft Team and the craft-libs contributors, MIT OR Apache-2.0). <c>v</c> with optional vertex colours, <c>vt</c>, <c>vn</c>, <c>f</c> with
/// every reference form (negative ones counting back), polygons kept whole, <c>o</c> objects (a
/// mesh each, holding only the vertices it uses), <c>g</c> groups, <c>usemtl</c> and <c>s</c>
/// (a part per combination), <c>mtllib</c> (asked of the resolver), <c>\</c> continuations and
/// <c>#</c> comments. A vertex or face line that cannot be read is left out and counted in one
/// warning. MTL materials map onto metallic-roughness.
/// </summary>
internal static class ObjReader
{
    private const string Kind = "OBJ";

    private static MeshException TooLarge(string kind, string m) => new($"{kind}: {m}");

    private static MeshException Malformed(string kind, string m) => new($"{kind}: {m}");

    private static int IndexOfWhite(string s)
    {
        for (var i = 0; i < s.Length; i++)
        {
            if (char.IsWhiteSpace(s[i]))
            {
                return i;
            }
        }
        return -1;
    }

    /// <summary>Rust's <c>split_whitespace</c>.</summary>
    private static string[] Tokens(string s) => s.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);

    /// <summary>Rust's <c>lines</c>: split at line feeds, a carriage return before one dropped, no empty last line.</summary>
    private static List<string> Lines(string text)
    {
        var lines = new List<string>(text.Split('\n'));
        if (lines.Count > 0 && lines[^1].Length == 0)
        {
            lines.RemoveAt(lines.Count - 1);
        }
        for (var i = 0; i < lines.Count; i++)
        {
            if (lines[i].EndsWith('\r'))
            {
                lines[i] = lines[i][..^1];
            }
        }
        return lines;
    }

    private static bool TryF32(string s, out float value) => float.TryParse(s, NumberStyles.Float, CultureInfo.InvariantCulture, out value) && s.Trim().Length == s.Length;

    private static bool TryF64(string s, out double value) => double.TryParse(s, NumberStyles.Float, CultureInfo.InvariantCulture, out value) && s.Trim().Length == s.Length;

    /// <summary>The last component of a referenced path, treating <c>\</c> as a separator too; null when the name carries no path at all.</summary>
    public static string? SiblingName(string name)
    {
        var at = name.LastIndexOfAny(['/', '\\']);
        var file = at < 0 ? name : name[(at + 1)..];
        return file.Length > 0 && file.Length < name.Length ? file : null;
    }

    /// <summary>The image a <c>map_*</c> line names: the rest of the line after its options.</summary>
    public static string MapName(string args)
    {
        static (string, string) Split(string s)
        {
            var at = IndexOfWhite(s);
            return at < 0 ? (s, "") : (s[..at], s[(at + 1)..].TrimStart());
        }
        var rest = args.Trim();
        while (true)
        {
            var (token, after) = Split(rest);
            if (after.Length == 0)
            {
                break;
            }
            int count, optionalFrom;
            switch (token)
            {
                case "-blendu" or "-blendv" or "-bm" or "-boost" or "-cc" or "-clamp" or "-imfchan" or "-texres" or "-type":
                    (count, optionalFrom) = (1, 1);
                    break;
                case "-mm":
                    (count, optionalFrom) = (2, 2);
                    break;
                case "-o" or "-s" or "-t":
                    (count, optionalFrom) = (3, 1);
                    break;
                default:
                    if (token.Length > 1 && token[0] == '-' && char.IsAsciiLetter(token[1]))
                    {
                        var all = Tokens(args);
                        return all.Length > 0 ? all[^1] : args;
                    }
                    return rest.TrimEnd();
            }
            rest = after;
            for (var i = 0; i < count; i++)
            {
                var (arg, next) = Split(rest);
                // The last token is always the name.
                if (next.Length == 0 || (i >= optionalFrom && !TryF32(arg, out _)))
                {
                    break;
                }
                rest = next;
            }
        }
        return rest.TrimEnd();
    }

    private static (string Key, string Remainder) KeyAndRest(string line)
    {
        var at = IndexOfWhite(line);
        return at < 0 ? (line, "") : (line[..at], line[(at + 1)..].Trim());
    }

    /// <summary>Materials of an MTL library and the images they name.</summary>
    public static (List<Material> Materials, List<ImageRef> Images) ReadMtl(byte[] bytes, Limits limits)
    {
        var text = Encoding.UTF8.GetString(bytes);
        var output = new List<Material>();
        var images = new List<ImageRef>();
        var prSeen = false;
        foreach (var raw in Lines(text))
        {
            var line = raw.Trim();
            var (key, rest) = KeyAndRest(line);
            if (key == "newmtl")
            {
                if (output.Count >= limits.MaxMaterials)
                {
                    throw TooLarge("MTL", $"more than {limits.MaxMaterials} materials");
                }
                prSeen = false;
                output.Add(Material.Plain(rest, [1, 1, 1, 1]));
                continue;
            }
            if (output.Count == 0)
            {
                continue;
            }
            MtlLine(output[^1], key, rest, ref prSeen, images, limits);
        }
        return (output, images);
    }

    private static void MtlLine(Material m, string key, string rest, ref bool prSeen, List<ImageRef> images, Limits limits)
    {
        float? Finite(string x) => TryF32(x, out var v) && float.IsFinite(v) ? v : null;
        float[] F3()
        {
            var v = Tokens(rest).Take(3).Select(Finite).OfType<float>().ToList();
            return v.Count switch
            {
                3 => [v[0], v[1], v[2]],
                0 => [0, 0, 0],
                _ => [v[0], v[0], v[0]],
            };
        }
        float? F1() => Tokens(rest) is { Length: > 0 } t ? Finite(t[0]) : null;
        TextureRef? Tex()
        {
            var name = MapName(rest);
            if (name.Length == 0)
            {
                return null;
            }
            var image = images.FindIndex(i => i.Uri == name);
            if (image < 0)
            {
                if (images.Count >= limits.MaxMaterials)
                {
                    throw TooLarge("MTL", $"more than {limits.MaxMaterials} images");
                }
                images.Add(new(name, name, null));
                image = images.Count - 1;
            }
            return new TextureRef(image, 0);
        }
        switch (key)
        {
            case "Kd":
                var c = F3().Select(Colour.SrgbToLinear).ToArray();
                m.BaseColor = [c[0], c[1], c[2], m.BaseColor[3]];
                break;
            case "Ke":
                m.Emissive = [.. F3().Select(Colour.SrgbToLinear)];
                break;
            case "d":
                m.BaseColor[3] = Math.Clamp(F1() ?? 1f, 0f, 1f);
                break;
            case "Tr":
                m.BaseColor[3] = Math.Clamp(1f - (F1() ?? 0f), 0f, 1f);
                break;
            case "Ns" when !prSeen:
                m.Roughness = MathF.Sqrt(2f / (Math.Max(F1() ?? 0f, 0f) + 2f));
                break;
            case "Pr":
                m.Roughness = Math.Clamp(F1() ?? 0.5f, 0f, 1f);
                prSeen = true;
                break;
            case "Pm":
                m.Metallic = Math.Clamp(F1() ?? 0f, 0f, 1f);
                break;
            case "map_Kd":
                m.BaseColorTexture = Tex();
                break;
            case "map_Ke":
                m.EmissiveTexture = Tex();
                if (m.Emissive.All(e => e == 0))
                {
                    m.Emissive = [1, 1, 1];
                }
                break;
            case "map_Bump" or "map_bump" or "bump" or "norm":
                m.NormalTexture = Tex();
                break;
            default:
                break;
        }
    }

    /// <summary>A 1-based (or negative, counting back) reference as an index into <paramref name="length"/> entries.</summary>
    private static uint? Index(string token, int length)
    {
        if (!long.TryParse(token, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var value))
        {
            return null;
        }
        value = value < 0 ? length + value : value - 1;
        return value >= 0 && value < length ? (uint)value : null;
    }

    private sealed class ObjectDef
    {
        public string Name { get; set; } = "";

        public List<Part> Parts { get; } = [];

        public Dictionary<(string, int?, uint), int> Slots { get; } = [];
    }

    private sealed class Reader(Limits limits)
    {
        private static readonly double[] Nan3 = [double.NaN, double.NaN, double.NaN];
        private readonly List<double[]> v = [];
        private readonly List<float[]> colors = [];
        private readonly List<float[]> vt = [];
        private readonly List<double[]> vn = [];
        private readonly List<ObjectDef> objects = [];
        private readonly Scene scene = new();
        private readonly Dictionary<string, int> materialIds = new(StringComparer.Ordinal);
        private readonly HashSet<string> libraries = new(StringComparer.Ordinal);
        private int? material;
        private string group = "";
        private uint smoothing;
        private int? slot;
        private long triangles;
        private int parts;
        private int badLines;
        private long dataBytes;

        private static double[]? Numbers(IEnumerable<string> tokens, int n)
        {
            var output = new double[n];
            using var it = tokens.GetEnumerator();
            for (var i = 0; i < n; i++)
            {
                if (!it.MoveNext() || !TryF64(it.Current, out var x) || !double.IsFinite(x))
                {
                    return null;
                }
                output[i] = x;
            }
            return output;
        }

        private void PoolRoom(int length, string what)
        {
            if (length >= limits.MaxVertices)
            {
                throw TooLarge(Kind, $"more than {limits.MaxVertices} {what}");
            }
        }

        private void Vertex(string[] rest)
        {
            PoolRoom(v.Count, "vertices");
            var p = Numbers(rest, 3);
            // Keep the numbering: later faces count vertices by position.
            v.Add(p ?? Nan3);
            badLines += p is null ? 1 : 0;
            if (rest.Length >= 6 && Numbers(rest.Skip(3), 3) is { } rgb)
            {
                while (colors.Count < v.Count - 1)
                {
                    colors.Add([1, 1, 1, 1]);
                }
                colors.RemoveRange(v.Count - 1, colors.Count - (v.Count - 1));
                colors.Add([(float)rgb[0], (float)rgb[1], (float)rgb[2], 1]);
            }
        }

        private void StartObject(string name)
        {
            slot = null;
            var full = objects.Count >= limits.MaxMeshes;
            if (objects.Count > 0 && objects[^1].Parts.Count == 0)
            {
                // An empty object takes the name instead of starting another.
                objects[^1].Name = name;
            }
            else if (full)
            {
                throw TooLarge(Kind, $"more than {limits.MaxMeshes} objects");
            }
            else
            {
                objects.Add(new ObjectDef { Name = name });
            }
        }

        private void UseMaterial(string name)
        {
            slot = null;
            if (name.Trim().Length == 0)
            {
                material = null;
                return;
            }
            if (materialIds.TryGetValue(name, out var i))
            {
                material = i;
                return;
            }
            if (scene.Materials.Count >= limits.MaxMaterials)
            {
                throw TooLarge(Kind, $"more than {limits.MaxMaterials} materials");
            }
            // Not in any library read: a named grey default.
            scene.Materials.Add(Material.Plain(name, [0.8f, 0.8f, 0.8f, 1f]));
            materialIds[name] = scene.Materials.Count - 1;
            material = scene.Materials.Count - 1;
        }

        private void Library(string name, Func<string, byte[]?> resolve)
        {
            // A library named again is already read.
            if (name.Length == 0 || !libraries.Add(name))
            {
                return;
            }
            if (scene.External.Count >= limits.MaxMaterials)
            {
                throw TooLarge(Kind, $"more than {limits.MaxMaterials} material libraries");
            }
            scene.External.Add(name);
            // The name as written (it may hold spaces), then each of several names, then the file name of a path from another machine.
            var names = new List<string> { name };
            names.AddRange(Tokens(name));
            if (SiblingName(name) is { } sibling)
            {
                names.Add(sibling);
            }
            byte[]? bytes = null;
            foreach (var candidate in names)
            {
                bytes = resolve(candidate);
                if (bytes is not null)
                {
                    break;
                }
            }
            if (bytes is null)
            {
                scene.Warnings.Add($"material library `{name}` was not supplied");
                return;
            }
            dataBytes += bytes.Length;
            if (dataBytes > limits.MaxDataBytes)
            {
                throw TooLarge(Kind, $"material libraries of more than {limits.MaxDataBytes} bytes");
            }
            var (materials, images) = ReadMtl(bytes, limits);
            var imageBase = scene.Images.Count;
            if (imageBase + images.Count > limits.MaxMaterials || scene.Materials.Count + materials.Count > limits.MaxMaterials)
            {
                throw TooLarge(Kind, $"more than {limits.MaxMaterials} materials or images");
            }
            scene.External.AddRange(images.Select(i => i.Uri).OfType<string>());
            scene.Images.AddRange(images);
            foreach (var m in materials)
            {
                static TextureRef? Moved(TextureRef? t, int by) => t is { } r ? r with { Image = r.Image + by } : null;
                m.BaseColorTexture = Moved(m.BaseColorTexture, imageBase);
                m.EmissiveTexture = Moved(m.EmissiveTexture, imageBase);
                m.NormalTexture = Moved(m.NormalTexture, imageBase);
                // The first definition of a name wins.
                materialIds.TryAdd(m.Name, scene.Materials.Count);
                scene.Materials.Add(m);
            }
        }

        private Corner? CornerOf(string token)
        {
            var fields = token.Split('/');
            if (Index(fields[0], v.Count) is not { } position || !double.IsFinite(v[(int)position][0]))
            {
                return null;
            }
            uint? uv = fields.Length > 1 && Index(fields[1], vt.Count) is { } t && float.IsFinite(vt[(int)t][0]) ? t : null;
            uint? normal = fields.Length > 2 && Index(fields[2], vn.Count) is { } n && double.IsFinite(vn[(int)n][0]) ? n : null;
            return new Corner(position, normal ?? Corner.None, uv ?? Corner.None);
        }

        private Part CurrentPart()
        {
            if (objects.Count == 0)
            {
                objects.Add(new ObjectDef());
            }
            var o = objects[^1];
            if (slot is not { } s)
            {
                var key = (group, material, smoothing);
                if (!o.Slots.TryGetValue(key, out s))
                {
                    if (parts >= limits.MaxParts)
                    {
                        throw TooLarge(Kind, $"more than {limits.MaxParts} groups");
                    }
                    parts++;
                    o.Parts.Add(new Part { Name = group, Material = material, Smoothing = smoothing });
                    s = o.Parts.Count - 1;
                    o.Slots[key] = s;
                }
            }
            slot = s;
            return o.Parts[s];
        }

        private void Face(IEnumerable<string> rest)
        {
            var corners = new List<Corner>();
            var invalid = false;
            foreach (var token in rest)
            {
                // Checked before collecting: one line can hold millions of corners.
                if (corners.Count >= limits.MaxFaceCorners)
                {
                    throw TooLarge(Kind, $"a face of more than {limits.MaxFaceCorners} corners");
                }
                if (CornerOf(token) is { } corner)
                {
                    corners.Add(corner);
                }
                else
                {
                    invalid = true;
                    corners.Add(Corner.At(0));
                }
            }
            if (invalid || corners.Count < 3)
            {
                badLines++;
                return;
            }
            triangles += corners.Count - 2;
            if (triangles > limits.MaxTriangles)
            {
                throw TooLarge(Kind, $"more than {limits.MaxTriangles} triangles");
            }
            var part = CurrentPart();
            // Sizes are only kept once a face is not a triangle.
            if (corners.Count != 3 && part.FaceSizes.Count == 0)
            {
                part.FaceSizes = [.. Enumerable.Repeat(3u, part.Corners.Count / 3)];
            }
            if (corners.Count != 3 || part.FaceSizes.Count > 0)
            {
                part.FaceSizes.Add((uint)corners.Count);
            }
            part.Corners.AddRange(corners);
        }

        public void Line(string line, Func<string, byte[]?> resolve)
        {
            var tokens = Tokens(line);
            if (tokens.Length == 0)
            {
                return;
            }
            var key = tokens[0];
            var rest = tokens.Skip(1);
            var after = line.Length >= key.Length ? line[key.Length..].Trim() : "";
            switch (key)
            {
                case "v":
                    Vertex([.. rest.Take(6)]);
                    break;
                case "vt":
                    PoolRoom(vt.Count, "texture coordinates");
                    // `vt u` alone is a 1D coordinate; OBJ's v axis points up the image.
                    var uv = Numbers(rest.Append("0"), 2);
                    vt.Add(uv is { } u ? [(float)u[0], 1f - (float)u[1]] : [float.NaN, float.NaN]);
                    badLines += uv is null ? 1 : 0;
                    break;
                case "vn":
                    PoolRoom(vn.Count, "normals");
                    var n = Numbers(rest, 3);
                    vn.Add(n ?? Nan3);
                    badLines += n is null ? 1 : 0;
                    break;
                case "s":
                    slot = null;
                    var first = rest.FirstOrDefault();
                    smoothing = first switch
                    {
                        null or "off" or "0" => 0,
                        "on" => 1,
                        _ => uint.TryParse(first, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var g) ? g : 0,
                    };
                    break;
                case "o":
                    StartObject(after);
                    break;
                case "g":
                    slot = null;
                    group = after;
                    break;
                case "usemtl":
                    UseMaterial(after);
                    break;
                case "mtllib":
                    Library(after, resolve);
                    break;
                case "f":
                    Face(rest);
                    break;
                default:
                    break;
            }
        }

        /// <summary>Each object as a mesh holding only the vertices it uses.</summary>
        public Scene Finish()
        {
            var colored = colors.Count > 0;
            while (colors.Count < v.Count)
            {
                colors.Add([1, 1, 1, 1]);
            }
            var meshes = new List<Mesh>();
            foreach (var o in objects)
            {
                if (o.Parts.Count == 0)
                {
                    continue;
                }
                var mesh = new Mesh { Name = o.Name, Parts = o.Parts };
                List<uint> Used(Func<Corner, uint> of) => [.. mesh.Parts.SelectMany(p => p.Corners.Select(of)).Where(i => i != Corner.None).Distinct().Order()];
                var (pu, nu, tu) = (Used(c => c.Position), Used(c => c.Normal), Used(c => c.Uv));
                static uint Place(List<uint> used, uint i)
                {
                    if (i == Corner.None)
                    {
                        return Corner.None;
                    }
                    var k = used.BinarySearch(i);
                    return k < 0 ? Corner.None : (uint)k;
                }
                foreach (var p in mesh.Parts)
                {
                    for (var i = 0; i < p.Corners.Count; i++)
                    {
                        var c = p.Corners[i];
                        p.Corners[i] = new Corner(Place(pu, c.Position), Place(nu, c.Normal), Place(tu, c.Uv));
                    }
                }
                mesh.Positions = [.. pu.Select(i => i < v.Count ? v[(int)i] : [0, 0, 0])];
                if (colored)
                {
                    mesh.Colors = [.. pu.Select(i => i < colors.Count ? colors[(int)i] : [1, 1, 1, 1])];
                }
                mesh.Normals = [.. nu.Select(i => i < vn.Count ? vn[(int)i] : [0, 0, 0])];
                var uvs = tu.Select(i => i < vt.Count ? vt[(int)i] : [0, 0]).ToList();
                if (uvs.Count > 0)
                {
                    mesh.UvSets.Add(uvs);
                }
                meshes.Add(mesh);
            }
            if (meshes.Count == 0)
            {
                throw Malformed(Kind, "no faces");
            }
            var result = new Scene { Meshes = meshes, Materials = scene.Materials, Images = scene.Images, External = scene.External, Warnings = scene.Warnings };
            if (badLines > 0)
            {
                result.Warnings.Add($"{badLines} vertex or face line(s) could not be read and were left out");
            }
            return result;
        }

        public static void ReadLines(string text, Reader r, Func<string, byte[]?> resolve, long maxDataBytes)
        {
            // A line ending in `\` continues on the next.
            var joined = new StringBuilder();
            var lines = Lines(text);
            for (var i = 0; i < lines.Count; i++)
            {
                var raw = i == 0 ? lines[i].TrimStart((char)0xFEFF) : lines[i];
                var hash = raw.IndexOf('#', StringComparison.Ordinal);
                var line = hash < 0 ? raw : raw[..hash];
                if (Encoding.UTF8.GetByteCount(line) > maxDataBytes || joined.Length + line.Length > maxDataBytes)
                {
                    throw TooLarge(Kind, "a continued line exceeds the memory limit");
                }
                var trimmed = line.TrimEnd();
                if (trimmed.EndsWith('\\'))
                {
                    joined.Append(trimmed[..^1]).Append(' ');
                    continue;
                }
                if (joined.Length == 0)
                {
                    r.Line(line.Trim(), resolve);
                }
                else
                {
                    joined.Append(line);
                    r.Line(joined.ToString().Trim(), resolve);
                    joined.Clear();
                }
            }
            r.Line(joined.ToString().Trim(), resolve);
        }
    }

    /// <summary>Reads an OBJ file; its material libraries are asked of <paramref name="resolve"/> by the name the file writes.</summary>
    public static Scene Read(byte[] bytes, Limits limits, Func<string, byte[]?> resolve)
    {
        var text = Encoding.UTF8.GetString(bytes);
        var reader = new Reader(limits);
        Reader.ReadLines(text, reader, resolve, limits.MaxDataBytes);
        return reader.Finish();
    }
}

using System.Numerics;

using Bayleaf.Assets.Wavefront;
using Bayleaf.Core;

namespace Bayleaf.Assets;

/// <summary>
/// Models in the formats read through the neutral mesh model (Wavefront OBJ with its MTL
/// libraries), turned into the engine's <see cref="MeshData"/> the way glTF models are: every
/// placed mesh merged, node transforms applied, triangles grouped by material. Files the model
/// names (material libraries, images) are read only beside it or below it; an image named by a
/// full path elsewhere is looked for by its file name beside the model, as exporters often write
/// the artist's own folders.
/// </summary>
internal static class Neutral
{
    /// <summary>The formats read here, by file extension.</summary>
    public static readonly IReadOnlyList<string> Extensions = ["obj"];

    /// <summary>Whether <paramref name="extension"/> (lower case) is read here.</summary>
    public static bool Reads(string extension) => Extensions.Contains(extension);

    private static Limits MeshLimits() => new(Loader.MaxVertices, Loader.MaxVertices);

    /// <summary>The file a model names, beside it or below it; else, for a name with folders that are not there, its file name beside the model.</summary>
    private static string? NamedFile(string folder, string name)
    {
        name = name.Trim().Replace('\\', '/');
        if (Loader.BufferPath(folder, name) is { } path && Files.IsFile(path))
        {
            return path;
        }
        var file = name[(name.LastIndexOf('/') + 1)..];
        if (file.Length == 0 || file is ".." or ".")
        {
            return null;
        }
        var beside = Paths.Join(folder, file);
        return Files.IsFile(beside) ? beside : null;
    }

    /// <summary>Reads the model's own scene, asking for the files it names beside it.</summary>
    private static Result<Scene, string> ReadScene(string path, byte[] bytes)
    {
        var folder = Paths.Folder(path);
        var extension = Loader.ExtensionOf(path);
        byte[]? Resolve(string name) => NamedFile(folder, name) is { } file && Loader.ReadLimited(file) is { IsOk: true } read ? read.Value.Bytes : null;
        if (extension != "obj")
        {
            return Result.Fail($"{extension} models are not read here");
        }
        try
        {
            var scene = ObjReader.Read(bytes, MeshLimits(), Resolve);
            scene.Validate();
            return scene;
        }
        catch (MeshException e)
        {
            return Result.Fail(e.Message);
        }
    }

    /// <summary>The files a model names beside it (material libraries and images that are there), each once.</summary>
    public static Result<List<string>, string> ModelFiles(string path)
    {
        var read = Loader.ReadLimited(path);
        if (read.IsError)
        {
            return read.AsFailure();
        }
        var scene = ReadScene(path, read.Value.Bytes);
        if (scene.IsError)
        {
            return Result.Fail($"{path}: {scene.Error}");
        }
        var folder = Paths.Folder(path);
        var files = new List<string>();
        foreach (var name in scene.Value.External)
        {
            if (NamedFile(folder, name) is not { } file)
            {
                continue;
            }
            var relative = folder.Length == 0 ? file : file[(folder.Length + 1)..];
            if (!files.Contains(relative, StringComparer.Ordinal))
            {
                files.Add(relative);
            }
        }
        return files;
    }

    /// <summary>Reads an OBJ model. Its import settings apply: the scale, and turning a Z-up model to Y up.</summary>
    public static Result<MeshData, string> LoadModel(string path)
    {
        var settings = ImportSettings.ReadFor(path);
        if (settings.IsError)
        {
            return settings.AsFailure();
        }
        var model = settings.Value?.Model ?? ModelImport.Default;
        var read = Loader.ReadLimited(path);
        if (read.IsError)
        {
            return read.AsFailure();
        }
        var scene = ReadScene(path, read.Value.Bytes);
        if (scene.IsError)
        {
            return Result.Fail($"{path}: {scene.Error}");
        }
        var id = Loader.FileId(path, read.Value.Modified);
        var root = Matrix.Scale(model.Scale);
        if (model.UpAxis == UpAxis.Z)
        {
            root = Matrix.Multiply(root, Matrix.RotationX(-Math.PI / 2));
        }
        var gathered = new Gathered();
        foreach (var (index, placed) in scene.Value.Instances())
        {
            var world = Matrix.Multiply(root, placed);
            var added = AddMesh(scene.Value.Meshes[index], world, gathered);
            if (added.IsError)
            {
                return Result.Fail($"{path}: {added.Error}");
            }
        }
        if (gathered.Indices.Count == 0)
        {
            return Result.Fail($"{path}: the model has no triangles");
        }
        var folder = Paths.Folder(path);
        var warnings = new List<string>(scene.Value.Warnings);
        var images = new Dictionary<int, Image?>();
        var materials = new Dictionary<int, ModelMaterial>();
        var parts = new List<MeshPart>();
        foreach (var (first, count, index) in gathered.Runs)
        {
            ModelMaterial? material = null;
            if (index is { } i && i < scene.Value.Materials.Count)
            {
                if (!materials.TryGetValue(i, out material))
                {
                    material = MaterialOf(scene.Value.Materials[i], scene.Value, folder, id, images, warnings);
                    materials[i] = material;
                }
            }
            parts.Add(new(first, count, material));
        }
        var (min, max) = MeshData.Bounds(gathered.Positions);
        return new MeshData
        {
            Id = id,
            Positions = [.. gathered.Positions],
            Normals = [.. gathered.Normals],
            Uvs = [.. gathered.Uvs],
            Indices = [.. gathered.Indices],
            Min = min,
            Max = max,
            Parts = parts,
            Material = parts.Select(p => p.Material).FirstOrDefault(m => m is not null),
            Warnings = warnings,
        };
    }

    private sealed class Gathered
    {
        public List<Vector3> Positions { get; } = [];

        public List<Vector3> Normals { get; } = [];

        public List<Vector2> Uvs { get; } = [];

        public List<uint> Indices { get; } = [];

        public List<(uint First, uint Count, int? Material)> Runs { get; } = [];
    }

    /// <summary>One placed mesh's triangles, moved by <paramref name="world"/>, onto the end of the model.</summary>
    private static Result<Unit, string> AddMesh(Mesh source, double[] world, Gathered mesh)
    {
        var unified = source.Unify();
        if (mesh.Positions.Count + unified.Positions.Count > Loader.MaxVertices)
        {
            return Result.Fail($"models may have at most {Loader.MaxVertices} vertices");
        }
        var vertexBase = (uint)mesh.Positions.Count;
        var count = unified.Positions.Count;
        var normalMatrix = Matrix.NormalMatrix(world);
        // A mirroring placement turns the triangles inside out; their corners go the other way round.
        var mirrored = Matrix.Determinant(world) < 0;
        var triangles = new List<(List<uint> Indices, int? Material)>();
        for (var p = 0; p < Math.Min(source.Parts.Count, unified.Parts.Count); p++)
        {
            var indices = new List<uint>(unified.Parts[p].Count * 3);
            foreach (var tri in unified.Parts[p])
            {
                if (tri.Any(i => i >= count))
                {
                    return Result.Fail("a triangle uses a vertex that does not exist");
                }
                indices.AddRange(mirrored ? [tri[0], tri[2], tri[1]] : tri);
            }
            triangles.Add((indices, source.Parts[p].Material));
        }
        var all = triangles.SelectMany(t => t.Indices).ToList();
        var positions = unified.Positions.Select(p => Matrix.TransformPoint(world, p)).ToList();
        var normals = unified.Normals.Count == count
            ? unified.Normals.Select(n => Matrix.NormalizeOrZero(Matrix.Apply3(normalMatrix, n))).ToList()
            : [.. Loader.FlatNormals(positions, all)];
        var uvs = unified.Uvs.Count == count ? unified.Uvs.Select(u => new Vector2(u[0], u[1])).ToList() : [.. new Vector2[count]];
        mesh.Positions.AddRange(positions);
        mesh.Normals.AddRange(normals);
        mesh.Uvs.AddRange(uvs);
        foreach (var (indices, material) in triangles)
        {
            var (first, n) = ((uint)mesh.Indices.Count, (uint)indices.Count);
            if (n == 0)
            {
                continue;
            }
            mesh.Indices.AddRange(indices.Select(i => vertexBase + i));
            var runs = mesh.Runs;
            if (runs.Count > 0 && runs[^1].Material == material && runs[^1].First + runs[^1].Count == first)
            {
                runs[^1] = (runs[^1].First, runs[^1].Count + n, material);
            }
            else
            {
                runs.Add((first, n, material));
            }
        }
        return Unit.Value;
    }

    /// <summary>The engine's material from a neutral one: base colour, metallic, roughness and the images it can read (an image that cannot be read is left out).</summary>
    private static ModelMaterial MaterialOf(Material m, Scene scene, string folder, ulong modelId, Dictionary<int, Image?> images, List<string> warnings)
    {
        Image? ImageFor(TextureRef? slot)
        {
            if (slot is not { UvSet: 0 } t)
            {
                return null;
            }
            var index = t.Image;
            if (images.TryGetValue(index, out var cached))
            {
                return cached;
            }
            Image? image = null;
            if (index < scene.Images.Count)
            {
                var source = scene.Images[index];
                var made = ImageOf(source, index, folder, modelId);
                if (made.IsOk)
                {
                    image = made.Value;
                }
                else
                {
                    warnings.Add($"material {m.Name}: image {source.Uri ?? source.Name} left out: {made.Error}");
                }
            }
            images[index] = image;
            return image;
        }
        static float Finite(float v, float fallback) => float.IsFinite(v) ? Math.Clamp(v, 0f, 1f) : fallback;
        var c = m.BaseColor;
        var baseImage = ImageFor(m.BaseColorTexture);
        var metalRoughness = ImageFor(m.MetallicRoughnessTexture);
        var normal = ImageFor(m.NormalTexture);
        return new(new(Finite(c[0], 1f), Finite(c[1], 1f), Finite(c[2], 1f), Finite(c[3], 1f)), Finite(m.Metallic, 0f), Finite(m.Roughness, 1f), baseImage,
            metalRoughness is null ? null : Loader.AsData(metalRoughness), normal is null ? null : Loader.AsData(normal));
    }

    private static Result<Image, string> ImageOf(ImageRef source, int index, string folder, ulong modelId)
    {
        byte[] bytes;
        if (source.Data is { } data)
        {
            bytes = data;
        }
        else if (source.Uri is { } uri)
        {
            if (NamedFile(folder, uri) is not { } file)
            {
                return Result.Fail("it is not beside the model");
            }
            var read = Loader.ReadLimited(file);
            if (read.IsError)
            {
                return read.AsFailure();
            }
            bytes = read.Value.Bytes;
        }
        else
        {
            return Result.Fail("it names no file");
        }
        var image = Loader.DecodeImage(bytes);
        // Distinct from the model's id and from its other images.
        return image.IsOk ? image.Value with { Id = Loader.ImageId(modelId, index) } : image;
    }

    /// <summary>Column-major 4 x 4 matrices in double precision, as the neutral model keeps placements.</summary>
    private static class Matrix
    {
        public static double[] Scale(float s) => [s, 0, 0, 0, 0, s, 0, 0, 0, 0, s, 0, 0, 0, 0, 1];

        public static double[] RotationX(double angle)
        {
            var (sin, cos) = Math.SinCos(angle);
            return [1, 0, 0, 0, 0, cos, sin, 0, 0, -sin, cos, 0, 0, 0, 0, 1];
        }

        /// <summary>a * b: b applies first.</summary>
        public static double[] Multiply(double[] a, double[] b)
        {
            var m = new double[16];
            for (var col = 0; col < 4; col++)
            {
                for (var row = 0; row < 4; row++)
                {
                    m[col * 4 + row] = a[row] * b[col * 4] + a[4 + row] * b[col * 4 + 1] + a[8 + row] * b[col * 4 + 2] + a[12 + row] * b[col * 4 + 3];
                }
            }
            return m;
        }

        public static Vector3 TransformPoint(double[] m, double[] p) => new(
            (float)(m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12]),
            (float)(m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13]),
            (float)(m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]));

        public static double Determinant(double[] m) => new Bayleaf.Core.Geometry.DMat4(new(m[0], m[1], m[2], m[3]), new(m[4], m[5], m[6], m[7]),
            new(m[8], m[9], m[10], m[11]), new(m[12], m[13], m[14], m[15])).Determinant();

        /// <summary>The inverse transpose of the 3 x 3 part, row by row.</summary>
        public static double[] NormalMatrix(double[] m)
        {
            double L(int r, int c) => m[c * 4 + r];
            var det = L(0, 0) * (L(1, 1) * L(2, 2) - L(1, 2) * L(2, 1)) - L(0, 1) * (L(1, 0) * L(2, 2) - L(1, 2) * L(2, 0)) + L(0, 2) * (L(1, 0) * L(2, 1) - L(1, 1) * L(2, 0));
            var n = new double[9];
            for (var r = 0; r < 3; r++)
            {
                for (var c = 0; c < 3; c++)
                {
                    var (r1, r2, c1, c2) = ((r + 1) % 3, (r + 2) % 3, (c + 1) % 3, (c + 2) % 3);
                    // The cofactor of (r, c) is the inverse's (c, r) times the determinant: the inverse transpose's (r, c).
                    n[r * 3 + c] = (L(r1, c1) * L(r2, c2) - L(r1, c2) * L(r2, c1)) / det;
                }
            }
            return n;
        }

        public static double[] Apply3(double[] n, double[] v) =>
            [n[0] * v[0] + n[1] * v[1] + n[2] * v[2], n[3] * v[0] + n[4] * v[1] + n[5] * v[2], n[6] * v[0] + n[7] * v[1] + n[8] * v[2]];

        public static Vector3 NormalizeOrZero(double[] v)
        {
            var recip = 1 / Math.Sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
            return double.IsFinite(recip) && recip > 0 ? new((float)(v[0] * recip), (float)(v[1] * recip), (float)(v[2] * recip)) : Vector3.Zero;
        }
    }
}

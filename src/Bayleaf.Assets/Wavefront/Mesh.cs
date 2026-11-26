namespace Bayleaf.Assets.Wavefront;

// The neutral mesh model of craft-mesh (storytold/craft-libs, Copyright (c) 2026 ArtCraft Team and the craft-libs contributors, MIT OR Apache-2.0;
// see ATTRIBUTION.md), ported with the parts the OBJ reader fills: positions and normals in double
// precision, faces of any size whose corners index separate pools, and a triangulation that
// keeps concave faces right.

/// <summary>Why a file could not be read.</summary>
internal sealed class MeshException(string message) : Exception(message);

/// <summary>The bounds a reader keeps.</summary>
internal sealed record Limits(int MaxVertices, int MaxTriangles)
{
    public int MaxFaceCorners { get; init; } = 4096;

    public int MaxMeshes { get; init; } = 100_000;

    public int MaxParts { get; init; } = 100_000;

    public int MaxMaterials { get; init; } = 100_000;

    public int MaxNodes { get; init; } = 100_000;

    public int MaxDepth { get; init; } = 64;

    public long MaxDataBytes { get; init; } = 1L << 30;
}

/// <summary>One corner of a face: indices into the mesh's positions, normals and UVs (<see cref="None"/> when it has none).</summary>
internal readonly record struct Corner(uint Position, uint Normal, uint Uv)
{
    public const uint None = uint.MaxValue;

    public static Corner At(uint position) => new(position, None, None);
}

/// <summary>Faces of a mesh that share a group name, a material and a smoothing group.</summary>
internal sealed class Part
{
    public string Name { get; init; } = "";

    public int? Material { get; init; }

    public uint Smoothing { get; init; }

    /// <summary>Corners of each face, in order. Empty means every face is a triangle.</summary>
    public List<uint> FaceSizes { get; set; } = [];

    public List<Corner> Corners { get; } = [];

    /// <summary>The faces, each its corners; stops at the first face the corner list is too short for.</summary>
    public IEnumerable<ArraySegment<Corner>> Faces()
    {
        var corners = Corners.ToArray();
        var at = 0;
        var sizes = FaceSizes;
        for (var f = 0; ; f++)
        {
            int n;
            if (sizes.Count == 0)
            {
                n = 3;
            }
            else if (f < sizes.Count)
            {
                n = (int)Math.Min(sizes[f], int.MaxValue);
            }
            else
            {
                yield break;
            }
            if (n == 0 || corners.Length - at < n)
            {
                yield break;
            }
            yield return new ArraySegment<Corner>(corners, at, n);
            at += n;
        }
    }

    /// <summary>Triangles the faces give: corners - 2 each.</summary>
    public int TriangleCount() => FaceSizes.Count == 0 ? Corners.Count / 3 : (int)FaceSizes.Sum(n => (long)Math.Max(0, (long)n - 2));

    /// <summary>The faces as triangles: a convex face as a fan, a concave one by ear clipping (<see cref="Triangulate"/>).</summary>
    public IEnumerable<Corner[]> TrianglesIn(IReadOnlyList<double[]> positions)
    {
        var budget = (long)Triangulate.WorkPerCorner * Corners.Count;
        foreach (var face in Faces())
        {
            foreach (var t in Triangulate.Face(face, positions, ref budget))
            {
                yield return t;
            }
        }
    }
}

/// <summary>A triangle list with one index per vertex.</summary>
internal sealed class Unified
{
    public List<double[]> Positions { get; } = [];

    /// <summary>One per vertex when every corner has a normal, else empty.</summary>
    public List<double[]> Normals { get; } = [];

    /// <summary>The first UV set, one per vertex when every corner has a UV, else empty.</summary>
    public List<float[]> Uvs { get; } = [];

    /// <summary>Triangles per part, in the order of the mesh's parts.</summary>
    public List<List<uint[]>> Parts { get; } = [];
}

/// <summary>One object: attribute pools and the parts whose corners index them.</summary>
internal sealed class Mesh
{
    public string Name { get; init; } = "";

    public List<double[]> Positions { get; set; } = [];

    public List<double[]> Normals { get; set; } = [];

    public List<List<float[]>> UvSets { get; } = [];

    public List<float[]> Colors { get; set; } = [];

    public List<Part> Parts { get; init; } = [];

    /// <summary>Each corner's indices are in range, the face sizes add up, and the per-position arrays have the positions' length.</summary>
    public void Validate()
    {
        void Bad(string m) => throw new MeshException($"mesh `{Name}`: {m}");
        var uvs = UvSets.Count > 0 ? UvSets[0].Count : 0;
        if (UvSets.Any(s => s.Count != uvs))
        {
            Bad("UV sets differ in length");
        }
        if (Colors.Count != 0 && Colors.Count != Positions.Count)
        {
            Bad("colours do not match the positions");
        }
        foreach (var p in Parts)
        {
            long? total;
            if (p.FaceSizes.Count == 0)
            {
                total = p.Corners.Count % 3 == 0 ? p.Corners.Count : null;
            }
            else
            {
                total = p.FaceSizes.Any(s => s < 3) ? null : p.FaceSizes.Sum(s => (long)s);
            }
            if (total != p.Corners.Count)
            {
                Bad("face sizes do not match the corners");
            }
            foreach (var c in p.Corners)
            {
                var ok = c.Position < Positions.Count && (c.Normal == Corner.None || c.Normal < Normals.Count) && (c.Uv == Corner.None || c.Uv < uvs);
                if (!ok)
                {
                    Bad("a corner index is out of range");
                }
            }
        }
    }

    /// <summary>The mesh as triangles with one vertex per distinct corner. Corners whose indices are out of range are dropped with their triangle.</summary>
    public Unified Unify()
    {
        var output = new Unified();
        bool All(Func<Corner, bool> f) => Parts.All(p => p.Corners.All(f));
        var normals = Normals.Count > 0 && All(c => c.Normal != Corner.None);
        var uvSet = UvSets.Count > 0 && All(c => c.Uv != Corner.None) ? UvSets[0] : null;
        var ids = new Dictionary<Corner, uint>();
        foreach (var part in Parts)
        {
            var tris = new List<uint[]>(part.TriangleCount());
            foreach (var t in part.TrianglesIn(Positions))
            {
                var tri = new uint[3];
                var dropped = false;
                for (var slot = 0; slot < 3 && !dropped; slot++)
                {
                    var c = t[slot];
                    var key = new Corner(c.Position, normals ? c.Normal : Corner.None, uvSet is not null ? c.Uv : Corner.None);
                    if (ids.TryGetValue(key, out var id))
                    {
                        tri[slot] = id;
                        continue;
                    }
                    if (c.Position >= Positions.Count)
                    {
                        dropped = true;
                        continue;
                    }
                    var n = normals && c.Normal < Normals.Count ? Normals[(int)c.Normal] : null;
                    var uv = uvSet is not null && c.Uv < uvSet.Count ? uvSet[(int)c.Uv] : null;
                    if ((normals && n is null) || (uvSet is not null && uv is null))
                    {
                        dropped = true;
                        continue;
                    }
                    id = (uint)output.Positions.Count;
                    output.Positions.Add(Positions[(int)c.Position]);
                    if (n is not null)
                    {
                        output.Normals.Add(n);
                    }
                    if (uv is not null)
                    {
                        output.Uvs.Add(uv);
                    }
                    ids[key] = id;
                    tri[slot] = id;
                }
                if (!dropped)
                {
                    tris.Add(tri);
                }
            }
            output.Parts.Add(tris);
        }
        return output;
    }
}

/// <summary>A texture slot: an image of the scene and the UV set it reads.</summary>
internal readonly record struct TextureRef(int Image, uint UvSet);

/// <summary>A metallic-roughness material: the parameters an MTL library can carry.</summary>
internal sealed class Material
{
    public string Name { get; init; } = "";

    /// <summary>Linear RGBA.</summary>
    public float[] BaseColor { get; set; } = [1, 1, 1, 1];

    public float Metallic { get; set; } = 1;

    public float Roughness { get; set; } = 1;

    /// <summary>Linear RGB.</summary>
    public float[] Emissive { get; set; } = [0, 0, 0];

    public TextureRef? BaseColorTexture { get; set; }

    public TextureRef? MetallicRoughnessTexture { get; set; }

    public TextureRef? NormalTexture { get; set; }

    public TextureRef? EmissiveTexture { get; set; }

    /// <summary>A plain dielectric of one colour.</summary>
    public static Material Plain(string name, float[] baseColor) => new() { Name = name, BaseColor = baseColor, Metallic = 0, Roughness = 0.5f };
}

/// <summary>An image a material uses: a reference the caller resolves, or bytes the file embedded.</summary>
internal sealed record ImageRef(string Name, string? Uri, byte[]? Data);

/// <summary>What a file holds: its meshes (each placed once, where the file puts it), materials and images.</summary>
internal sealed class Scene
{
    public List<Mesh> Meshes { get; init; } = [];

    public List<Material> Materials { get; set; } = [];

    public List<ImageRef> Images { get; set; } = [];

    /// <summary>Every other file this one names, as written, whether or not the resolver supplied it.</summary>
    public List<string> External { get; set; } = [];

    /// <summary>What was left out or repaired while reading.</summary>
    public List<string> Warnings { get; set; } = [];

    /// <summary>Every mesh's index with its world transform (column-major), one root per mesh at its own place.</summary>
    public IEnumerable<(int Mesh, double[] World)> Instances()
    {
        for (var i = 0; i < Meshes.Count; i++)
        {
            yield return (i, [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
        }
    }

    /// <summary>Every mesh valid, and every material and image index in range.</summary>
    public void Validate()
    {
        foreach (var m in Meshes)
        {
            m.Validate();
            if (m.Parts.Any(p => p.Material is { } i && i >= Materials.Count))
            {
                throw new MeshException($"mesh `{m.Name}` names a material that does not exist");
            }
        }
        foreach (var m in Materials)
        {
            if (new[] { m.BaseColorTexture, m.MetallicRoughnessTexture, m.NormalTexture, m.EmissiveTexture }.Any(t => t is { } r && r.Image >= Images.Count))
            {
                throw new MeshException($"material `{m.Name}` names an image that does not exist");
            }
        }
    }
}

/// <summary>Colour conversions of craft-mesh.</summary>
internal static class Colour
{
    public static float SrgbToLinear(float c) => c <= 0.04045f ? c / 12.92f : MathF.Pow((c + 0.055f) / 1.055f, 2.4f);
}

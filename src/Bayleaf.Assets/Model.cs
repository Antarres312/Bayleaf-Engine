using System.Numerics;

namespace Bayleaf.Assets;

/// <summary>An image as 8-bit RGBA pixels, rows top to bottom.</summary>
/// <param name="Id">Changes when the file changes.</param>
/// <param name="Width">Pixels across.</param>
/// <param name="Height">Pixels down.</param>
/// <param name="Rgba">Four bytes a pixel, row by row.</param>
public sealed record Image(ulong Id, int Width, int Height, byte[] Rgba)
{
    /// <summary>Drawn with smaller copies for far away (its import settings can turn them off).</summary>
    public bool Mipmaps { get; init; } = true;

    /// <summary>Plain numbers rather than colours (a normal map, a mask): read as stored, not as sRGB.</summary>
    public bool Data { get; init; }

    /// <summary>
    /// The image in 4 x 4 blocks for the GPU, when its import settings ask for it: each level of
    /// <see cref="Loader.MipLevels"/> too when it has mipmaps.
    /// </summary>
    public Blocks? Blocks { get; init; }
}

/// <summary>An image block-compressed for the GPU.</summary>
/// <param name="Compression">BC1 or BC3.</param>
/// <param name="Levels">The blocks of the image, then of each smaller copy.</param>
public sealed record Blocks(Compression Compression, IReadOnlyList<byte[]> Levels);

/// <summary>One size of an image: width, height and RGBA pixels.</summary>
public readonly record struct MipLevel(int Width, int Height, byte[] Rgba);

/// <summary>Triangles <c>Indices[First..First + Count]</c> and the material they use.</summary>
public sealed record MeshPart(uint First, uint Count, ModelMaterial? Material);

/// <summary>
/// The parts of a glTF material the engine draws: base colour (linear RGBA, times the image when
/// there is one), metallic and roughness (times their image), and a normal map.
/// </summary>
/// <param name="BaseColor">Linear RGBA.</param>
/// <param name="Metallic">0 to 1.</param>
/// <param name="Roughness">0 to 1.</param>
/// <param name="Image">The base colour image, read by the first texture coordinates.</param>
/// <param name="MetalRoughness">Roughness in green and metallic in blue, times the numbers above.</param>
/// <param name="Normal">A tangent-space normal map.</param>
public sealed record ModelMaterial(Vector4 BaseColor, float Metallic, float Roughness, Image? Image, Image? MetalRoughness, Image? Normal);

/// <summary>A model's triangles, every mesh of its default scene merged, with node transforms applied.</summary>
public sealed record MeshData
{
    /// <summary>Changes when the file changes.</summary>
    public ulong Id { get; init; }

    public IReadOnlyList<Vector3> Positions { get; init; } = [];

    public IReadOnlyList<Vector3> Normals { get; init; } = [];

    public IReadOnlyList<Vector2> Uvs { get; init; } = [];

    public IReadOnlyList<uint> Indices { get; init; } = [];

    /// <summary>Smallest corner of the box around every position.</summary>
    public Vector3 Min { get; init; }

    /// <summary>Largest corner of the box around every position.</summary>
    public Vector3 Max { get; init; }

    /// <summary>The model's own material, from its first triangles that have one.</summary>
    public ModelMaterial? Material { get; init; }

    /// <summary>Runs of triangles that share a material, in index order. A model with one material has one.</summary>
    public IReadOnlyList<MeshPart> Parts { get; init; } = [];

    /// <summary>What was left out or repaired while reading (an image that could not be read, for one).</summary>
    public IReadOnlyList<string> Warnings { get; init; } = [];

    /// <summary>The model's nodes and animations, when it has any: the vertices above are its rest pose.</summary>
    public Skeleton? Skeleton { get; init; }

    /// <summary>Changes when the vertices change while the id stays (a model posed by its animation).</summary>
    public ulong Version { get; init; }

    /// <summary>The box round <paramref name="positions"/>: infinities for none, as the Rust engine's fold gives.</summary>
    internal static (Vector3 Min, Vector3 Max) Bounds(IEnumerable<Vector3> positions)
    {
        var (min, max) = (new Vector3(float.PositiveInfinity), new Vector3(float.NegativeInfinity));
        foreach (var p in positions)
        {
            min = FMath.Min(min, p);
            max = FMath.Max(max, p);
        }
        return (min, max);
    }
}

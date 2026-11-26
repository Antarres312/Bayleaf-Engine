using System.Buffers.Binary;
using System.Numerics;

namespace Bayleaf.Assets.Gltf;

/// <summary>
/// Reads accessors' elements out of a document's buffers. A read gives null when the data is not
/// there (no buffer view, or a range outside its buffer), which the loader treats as the attribute
/// missing, as the Rust engine's reader does.
/// </summary>
internal sealed class Accessors(Document document, IReadOnlyList<byte[]> buffers)
{
    private const int Byte = 5120;
    private const int UnsignedByte = 5121;
    private const int Short = 5122;
    private const int UnsignedShort = 5123;
    private const int UnsignedInt = 5125;
    private const int Float = 5126;

    /// <summary>Elements of <paramref name="size"/> bytes each, read by <paramref name="read"/>, with sparse substitutions applied.</summary>
    private T[]? Read<T>(int index, int size, Func<ReadOnlySpan<byte>, T> read)
    {
        var accessor = document.Accessors[index];
        var count = accessor.Count;
        T[]? values;
        if (accessor.View is { } viewIndex)
        {
            var view = document.Views[viewIndex];
            values = Elements(view, accessor.Offset, count, view.Stride ?? size, size, read);
            if (values is null)
            {
                return null;
            }
        }
        else if (accessor.Sparse is not null)
        {
            values = new T[count];
            var zero = read(new byte[size]);
            Array.Fill(values, zero);
        }
        else
        {
            return null;
        }
        if (accessor.Sparse is { } sparse)
        {
            var indexSize = sparse.IndicesType switch
            {
                UnsignedByte => 1,
                UnsignedShort => 2,
                _ => 4,
            };
            var indices = Elements(document.Views[sparse.IndicesView], sparse.IndicesOffset, sparse.Count, indexSize, indexSize, b => indexSize switch
            {
                1 => b[0],
                2 => BinaryPrimitives.ReadUInt16LittleEndian(b),
                _ => BinaryPrimitives.ReadUInt32LittleEndian(b),
            });
            var replacements = Elements(document.Views[sparse.ValuesView], sparse.ValuesOffset, sparse.Count, size, size, read);
            if (indices is null || replacements is null)
            {
                return null;
            }
            for (var i = 0; i < indices.Length; i++)
            {
                if (indices[i] < (uint)values.Length)
                {
                    values[indices[i]] = replacements[i];
                }
            }
        }
        return values;
    }

    private T[]? Elements<T>(ViewDef view, ulong offset, int count, int stride, int size, Func<ReadOnlySpan<byte>, T> read)
    {
        if (count == 0)
        {
            return [];
        }
        if (view.Buffer >= buffers.Count)
        {
            return null;
        }
        var buffer = buffers[view.Buffer];
        var start = view.Offset + offset;
        var end = start + (ulong)stride * (ulong)(count - 1) + (ulong)size;
        if (start > (ulong)buffer.Length || end > (ulong)buffer.Length || end < start)
        {
            return null;
        }
        var values = new T[count];
        for (var i = 0; i < count; i++)
        {
            values[i] = read(buffer.AsSpan((int)start + i * stride, size));
        }
        return values;
    }

    private static float F(ReadOnlySpan<byte> b, int at) => BinaryPrimitives.ReadSingleLittleEndian(b[(at * 4)..]);

    public int ComponentType(int index) => document.Accessors[index].ComponentType;

    public Vector3[]? Vec3(int index) => Read(index, 12, b => new Vector3(F(b, 0), F(b, 1), F(b, 2)));

    public Vector4[]? Vec4(int index) => Read(index, 16, b => new Vector4(F(b, 0), F(b, 1), F(b, 2), F(b, 3)));

    public float[]? Scalars(int index) => Read(index, 4, b => F(b, 0));

    public float[][]? Matrices(int index) => Read(index, 64, b =>
    {
        var m = new float[16];
        for (var i = 0; i < 16; i++)
        {
            m[i] = F(b, i);
        }
        return m;
    });

    /// <summary>Triangle indices of any unsigned type; null for another type.</summary>
    public uint[]? Indices(int index) => ComponentType(index) switch
    {
        UnsignedByte => Read(index, 1, b => (uint)b[0]),
        UnsignedShort => Read(index, 2, b => (uint)BinaryPrimitives.ReadUInt16LittleEndian(b)),
        UnsignedInt => Read(index, 4, BinaryPrimitives.ReadUInt32LittleEndian),
        _ => null,
    };

    /// <summary>Texture coordinates: floats, or unsigned bytes and shorts read as 0 to 1.</summary>
    public Vector2[]? TexCoords(int index) => ComponentType(index) switch
    {
        UnsignedByte => Read(index, 2, b => new Vector2(b[0] / 255f, b[1] / 255f)),
        UnsignedShort => Read(index, 4, b => new Vector2(BinaryPrimitives.ReadUInt16LittleEndian(b) / 65535f, BinaryPrimitives.ReadUInt16LittleEndian(b[2..]) / 65535f)),
        Float => Read(index, 8, b => new Vector2(F(b, 0), F(b, 1))),
        _ => null,
    };

    /// <summary>Four joints of each vertex, from unsigned bytes or shorts.</summary>
    public ushort[][]? Joints(int index) => ComponentType(index) switch
    {
        UnsignedByte => Read(index, 4, b => new ushort[] { b[0], b[1], b[2], b[3] }),
        UnsignedShort => Read(index, 8, b => new[]
        {
            BinaryPrimitives.ReadUInt16LittleEndian(b), BinaryPrimitives.ReadUInt16LittleEndian(b[2..]), BinaryPrimitives.ReadUInt16LittleEndian(b[4..]),
            BinaryPrimitives.ReadUInt16LittleEndian(b[6..]),
        }),
        _ => null,
    };

    /// <summary>Four numbers of each element as floats: unsigned types read as 0 to 1, signed ones as -1 to 1.</summary>
    public Vector4[]? NormalizedVec4(int index) => ComponentType(index) switch
    {
        Byte => Read(index, 4, b => new Vector4(Signed((sbyte)b[0], 127f), Signed((sbyte)b[1], 127f), Signed((sbyte)b[2], 127f), Signed((sbyte)b[3], 127f))),
        UnsignedByte => Read(index, 4, b => new Vector4(b[0] / 255f, b[1] / 255f, b[2] / 255f, b[3] / 255f)),
        Short => Read(index, 8, b => new Vector4(Signed(BinaryPrimitives.ReadInt16LittleEndian(b), 32767f), Signed(BinaryPrimitives.ReadInt16LittleEndian(b[2..]), 32767f),
            Signed(BinaryPrimitives.ReadInt16LittleEndian(b[4..]), 32767f), Signed(BinaryPrimitives.ReadInt16LittleEndian(b[6..]), 32767f))),
        UnsignedShort => Read(index, 8, b => new Vector4(BinaryPrimitives.ReadUInt16LittleEndian(b) / 65535f, BinaryPrimitives.ReadUInt16LittleEndian(b[2..]) / 65535f,
            BinaryPrimitives.ReadUInt16LittleEndian(b[4..]) / 65535f, BinaryPrimitives.ReadUInt16LittleEndian(b[6..]) / 65535f)),
        Float => Vec4(index),
        _ => null,
    };

    private static float Signed(float v, float max) => Math.Max(v / max, -1f);

    /// <summary>Four weights of each vertex as floats (unsigned bytes and shorts read as 0 to 1).</summary>
    public Vector4[]? Weights(int index) => ComponentType(index) is UnsignedByte or UnsignedShort or Float ? NormalizedVec4(index) : null;
}

using System.Buffers.Binary;
using System.IO.Compression;
using System.Text;

namespace Bayleaf.Render.Tests;

/// <summary>Small image and model files the renderer's tests write for themselves.</summary>
internal static class TestFiles
{
    private static readonly uint[] CrcTable = [.. Enumerable.Range(0, 256).Select(n =>
    {
        var c = (uint)n;
        for (var k = 0; k < 8; k++)
        {
            c = (c & 1) != 0 ? 0xEDB88320u ^ (c >> 1) : c >> 1;
        }
        return c;
    })];

    private static void Chunk(Stream output, string type, byte[] data)
    {
        var name = Encoding.ASCII.GetBytes(type);
        Span<byte> word = stackalloc byte[4];
        BinaryPrimitives.WriteUInt32BigEndian(word, (uint)data.Length);
        output.Write(word);
        output.Write(name);
        output.Write(data);
        var c = 0xFFFFFFFFu;
        foreach (var b in name.Concat(data))
        {
            c = CrcTable[(c ^ b) & 0xFF] ^ (c >> 8);
        }
        BinaryPrimitives.WriteUInt32BigEndian(word, c ^ 0xFFFFFFFFu);
        output.Write(word);
    }

    /// <summary>A PNG file of 8-bit pixels, <paramref name="channels"/> a pixel (3 RGB, 4 RGBA).</summary>
    public static byte[] Png(int width, int height, int channels, byte[] pixels)
    {
        using var output = new MemoryStream();
        output.Write([0x89, (byte)'P', (byte)'N', (byte)'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        var header = new byte[13];
        BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(0), (uint)width);
        BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(4), (uint)height);
        header[8] = 8;
        header[9] = (byte)(channels == 4 ? 6 : 2);
        Chunk(output, "IHDR", header);
        using (var raw = new MemoryStream())
        {
            using (var z = new ZLibStream(raw, CompressionLevel.Optimal, true))
            {
                for (var y = 0; y < height; y++)
                {
                    z.WriteByte(0);
                    z.Write(pixels, y * width * channels, width * channels);
                }
            }
            Chunk(output, "IDAT", raw.ToArray());
        }
        Chunk(output, "IEND", []);
        return output.ToArray();
    }

    /// <summary>A new empty folder of its own.</summary>
    public static string Folder(string name)
    {
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-render-{name}-{System.Environment.ProcessId}-{Guid.NewGuid():N}");
        Directory.CreateDirectory(dir);
        return dir;
    }

    public static byte[] Floats(params float[] values)
    {
        var bytes = new byte[values.Length * 4];
        for (var i = 0; i < values.Length; i++)
        {
            BinaryPrimitives.WriteSingleLittleEndian(bytes.AsSpan(i * 4), values[i]);
        }
        return bytes;
    }

    public static byte[] Shorts(params ushort[] values)
    {
        var bytes = new byte[values.Length * 2];
        for (var i = 0; i < values.Length; i++)
        {
            BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(i * 2), values[i]);
        }
        return bytes;
    }

    /// <summary>
    /// A folder with <c>stripes.png</c> (left half red, right half blue) and <c>wall.gltf</c> (a 2 x 2
    /// square facing +Z, made of two triangles).
    /// </summary>
    public static string AssetFolder(string name)
    {
        var dir = Folder(name);
        File.WriteAllBytes(Path.Join(dir, "stripes.png"), Png(2, 1, 4, [255, 0, 0, 255, 0, 0, 255, 255]));
        File.WriteAllBytes(Path.Join(dir, "wall.bin"), [.. Floats(-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0), .. Shorts(0, 1, 2, 0, 2, 3)]);
        File.WriteAllText(Path.Join(dir, "wall.gltf"), WallGltf(""));
        return dir;
    }

    /// <summary>The wall's glTF; <paramref name="extra"/> is added to its primitive and the file (a material, animations).</summary>
    public static string WallGltf(string material, string extraBuffers = "", string extraViews = "", string extraAccessors = "", string rest = "") => $$"""
        {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
         "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1{{material}}}]}],
         "buffers": [{"uri": "wall.bin", "byteLength": 60}{{extraBuffers}}],
         "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 48}, {"buffer": 0, "byteOffset": 48, "byteLength": 12}{{extraViews}}],
         "accessors": [
           {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1, -1, 0], "max": [1, 1, 0]},
           {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"}{{extraAccessors}}
         ]{{rest}}}
        """;

    /// <summary>A 2 x 2 square facing +Z with texture coordinates (v down, as in glTF) and a one-pixel normal map of <paramref name="normal"/>, in a folder of its own.</summary>
    public static string NormalMappedSquare(string name, byte[] normal)
    {
        var dir = Folder(name);
        File.WriteAllBytes(Path.Join(dir, "normal.png"), Png(1, 1, 3, normal));
        File.WriteAllBytes(Path.Join(dir, "square.bin"),
        [
            .. Floats(-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0),
            .. Floats(0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1),
            .. Floats(0, 1, 1, 1, 1, 0, 0, 0),
            .. Shorts(0, 1, 2, 0, 2, 3),
        ]);
        File.WriteAllText(Path.Join(dir, "square.gltf"), """
            {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
             "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0}]}],
             "materials": [{"pbrMetallicRoughness": {"metallicFactor": 0.0, "roughnessFactor": 1.0}, "normalTexture": {"index": 0}}],
             "textures": [{"source": 0}], "images": [{"uri": "normal.png"}],
             "buffers": [{"uri": "square.bin", "byteLength": 140}],
             "bufferViews": [
               {"buffer": 0, "byteOffset": 0, "byteLength": 48}, {"buffer": 0, "byteOffset": 48, "byteLength": 48},
               {"buffer": 0, "byteOffset": 96, "byteLength": 32}, {"buffer": 0, "byteOffset": 128, "byteLength": 12}
             ],
             "accessors": [
               {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1, -1, 0], "max": [1, 1, 0]},
               {"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3"},
               {"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC2"},
               {"bufferView": 3, "componentType": 5123, "count": 6, "type": "SCALAR"}
             ]}
            """);
        return dir;
    }

    public static void Remove(string dir)
    {
        try
        {
            Directory.Delete(dir, true);
        }
        catch (IOException)
        {
        }
    }
}

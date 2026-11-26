using System.Buffers.Binary;
using System.IO.Compression;
using System.Text;

namespace Bayleaf.Assets.Tests;

/// <summary>Files the tests write: small images and models made in code.</summary>
internal static class TestFiles
{
    private static int next;

    /// <summary>A new empty folder under the system's temporary folder.</summary>
    public static string TempDir(string name)
    {
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-assets-cs-{name}-{Environment.ProcessId}-{Interlocked.Increment(ref next)}");
        Directory.CreateDirectory(dir);
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
            // Left for the system to clear.
        }
    }

    private static byte[] Chunk(string type, byte[] data)
    {
        var chunk = new byte[12 + data.Length];
        BinaryPrimitives.WriteUInt32BigEndian(chunk, (uint)data.Length);
        Encoding.ASCII.GetBytes(type).CopyTo(chunk, 4);
        data.CopyTo(chunk, 8);
        var c = 0xFFFFFFFFu;
        foreach (var b in chunk.AsSpan(4, 4 + data.Length))
        {
            c ^= b;
            for (var k = 0; k < 8; k++)
            {
                c = (c & 1) != 0 ? 0xEDB88320u ^ (c >> 1) : c >> 1;
            }
        }
        BinaryPrimitives.WriteUInt32BigEndian(chunk.AsSpan(8 + data.Length), c ^ 0xFFFFFFFFu);
        return chunk;
    }

    /// <summary>An 8-bit PNG of <paramref name="channels"/> samples a pixel (3 RGB, 4 RGBA), rows unfiltered.</summary>
    public static byte[] Png(int width, int height, int channels, byte[] samples)
    {
        var header = new byte[13];
        BinaryPrimitives.WriteUInt32BigEndian(header, (uint)width);
        BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(4), (uint)height);
        (header[8], header[9]) = (8, channels == 4 ? (byte)6 : (byte)2);
        var raw = new MemoryStream();
        for (var y = 0; y < height; y++)
        {
            raw.WriteByte(0);
            raw.Write(samples, y * width * channels, width * channels);
        }
        var packed = new MemoryStream();
        using (var z = new ZLibStream(packed, CompressionLevel.Optimal, true))
        {
            z.Write(raw.ToArray());
        }
        return [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, .. Chunk("IHDR", header), .. Chunk("IDAT", packed.ToArray()), .. Chunk("IEND", [])];
    }

    /// <summary>A 2 x 1 RGB PNG: red, then blue.</summary>
    public static byte[] PngBytes() => Png(2, 1, 3, [255, 0, 0, 0, 0, 255]);

    /// <summary>
    /// A 16 x 8 grey JPEG, written by hand: one channel, quantisation of 1, and Huffman tables with
    /// one code each (DC difference 0, end of block), so every pixel is the middle grey 128.
    /// </summary>
    public static byte[] GreyJpeg(ushort width)
    {
        var b = new List<byte> { 0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00 };
        b.AddRange(Enumerable.Repeat((byte)1, 64));
        b.AddRange([0xFF, 0xC0, 0x00, 0x0B, 8, 0x00, 0x08, (byte)(width >> 8), (byte)width, 1, 1, 0x11, 0]);
        foreach (var kind in new byte[] { 0x00, 0x10 })
        {
            b.AddRange([0xFF, 0xC4, 0x00, 0x14, kind, 1]);
            b.AddRange(new byte[15]);
            b.Add(0x00);
        }
        b.AddRange([0xFF, 0xDA, 0x00, 0x08, 1, 1, 0x00, 0, 63, 0]);
        // Two blocks of "DC 0, end of block" (bits 00 00), padded with ones.
        b.AddRange([0x0F, 0xFF, 0xD9]);
        return [.. b];
    }

    private static void Floats(List<byte> bin, params float[] values)
    {
        var four = new byte[4];
        foreach (var v in values)
        {
            BinaryPrimitives.WriteSingleLittleEndian(four, v);
            bin.AddRange(four);
        }
    }

    /// <summary>A one-triangle glTF model with its buffer in <c>tri.bin</c>, moved up by 2 by its node; <paramref name="extra"/> adds top-level fields and <paramref name="primitive"/> fields of its primitive.</summary>
    public static void WriteTriangle(string dir, string bufferUri, ushort index, string extra = "", string primitive = "")
    {
        var bin = new List<byte>();
        Floats(bin, 0, 0, 0, 1, 0, 0, 0, 0, -1);
        foreach (var i in new ushort[] { 0, 1, index })
        {
            bin.AddRange([(byte)i, (byte)(i >> 8)]);
        }
        bin.AddRange([0, 0]);
        File.WriteAllBytes(Path.Join(dir, "tri.bin"), [.. bin]);
        var gltf = $$"""
            {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
             "nodes": [{"mesh": 0, "translation": [0.0, 2.0, 0.0]}],
             "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1{{primitive}}}]}],
             "buffers": [{"uri": "{{bufferUri}}", "byteLength": {{bin.Count}}}],
             "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 6}],
             "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, -1], "max": [1, 0, 0]},
                           {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}]{{extra}}}
            """;
        File.WriteAllText(Path.Join(dir, "tri.gltf"), gltf);
    }

    /// <summary>
    /// A glTF triangle on node 1 (a child of node 0) and an animation "Slide" moving node 1 from x 0
    /// to x 2 over a second. With <paramref name="skinned"/>, the triangle's vertices instead follow
    /// joint node 1 of a skin, from a mesh on node 0.
    /// </summary>
    public static string AnimatedTriangle(string dir, bool skinned)
    {
        var bin = new List<byte>();
        Floats(bin, 0, 0, 0, 1, 0, 0, 0, 1, 0);
        Floats(bin, 0, 1);
        Floats(bin, 0, 0, 0, 2, 0, 0);
        for (var i = 0; i < 3; i++)
        {
            bin.AddRange([1, 0, 0, 0]);
        }
        Floats(bin, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0);
        File.WriteAllBytes(Path.Join(dir, "slide.bin"), [.. bin]);
        var attributes = skinned ? """{"POSITION": 0, "JOINTS_0": 3, "WEIGHTS_0": 4}""" : """{"POSITION": 0}""";
        var nodes = skinned ? """[{"children": [1], "mesh": 0, "skin": 0}, {}]""" : """[{"children": [1]}, {"mesh": 0}]""";
        var skins = skinned ? """[{"joints": [0, 1]}]""" : "[]";
        var gltf = $$$"""
            {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": {{{nodes}}},
             "meshes": [{"primitives": [{"attributes": {{{attributes}}}}]}], "skins": {{{skins}}},
             "animations": [{"name": "Slide", "channels": [{"sampler": 0, "target": {"node": 1, "path": "translation"}}], "samplers": [{"input": 1, "output": 2}]}],
             "buffers": [{"uri": "slide.bin", "byteLength": {{{bin.Count}}}}],
             "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 8},
                             {"buffer": 0, "byteOffset": 44, "byteLength": 24}, {"buffer": 0, "byteOffset": 68, "byteLength": 12},
                             {"buffer": 0, "byteOffset": 80, "byteLength": 48}],
             "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]},
                           {"bufferView": 1, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
                           {"bufferView": 2, "componentType": 5126, "count": 2, "type": "VEC3"},
                           {"bufferView": 3, "componentType": 5121, "count": 3, "type": "VEC4"},
                           {"bufferView": 4, "componentType": 5126, "count": 3, "type": "VEC4"}]}
            """;
        var path = Path.Join(dir, skinned ? "skinned.gltf" : "slide.gltf");
        File.WriteAllText(path, gltf);
        return path;
    }

    /// <summary>IEEE 754 binary16 bits of <paramref name="v"/>.</summary>
    private static ushort Half(float v) => BitConverter.HalfToUInt16Bits((Half)v);

    /// <summary>
    /// An uncompressed single-part scan-line OpenEXR file of half-float RGBA (alpha premultiplied),
    /// after craft-exr's RGBA8 writer (storytold/craft-libs, Copyright (c) 2026 ArtCraft Team and the craft-libs contributors, MIT OR Apache-2.0): <paramref name="linear"/> maps each 8-bit value to linear light.
    /// </summary>
    public static byte[] Exr(byte[] rgba, int w, int h, float[] linear)
    {
        var hd = new List<byte> { 0x76, 0x2F, 0x31, 0x01, 2, 0, 0, 0 };
        void Attr(string name, string type, byte[] value)
        {
            hd.AddRange(Encoding.ASCII.GetBytes(name));
            hd.Add(0);
            hd.AddRange(Encoding.ASCII.GetBytes(type));
            hd.Add(0);
            hd.AddRange(BitConverter.GetBytes(value.Length));
            hd.AddRange(value);
        }
        static byte[] Le(params float[] v) => [.. v.SelectMany(BitConverter.GetBytes)];
        var channels = new List<byte>();
        foreach (var c in "ABGR")
        {
            channels.AddRange([(byte)c, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
        }
        channels.Add(0);
        byte[] window = [.. new[] { 0, 0, w - 1, h - 1 }.SelectMany(BitConverter.GetBytes)];
        Attr("channels", "chlist", [.. channels]);
        Attr("compression", "compression", [0]);
        Attr("dataWindow", "box2i", window);
        Attr("displayWindow", "box2i", window);
        Attr("lineOrder", "lineOrder", [0]);
        Attr("pixelAspectRatio", "float", Le(1));
        Attr("screenWindowCenter", "v2f", Le(0, 0));
        Attr("screenWindowWidth", "float", Le(1));
        hd.Add(0);
        var table = hd.Count;
        var output = new List<byte>(hd);
        output.AddRange(new byte[h * 8]);
        for (var y = 0; y < h; y++)
        {
            BinaryPrimitives.WriteUInt64LittleEndian(System.Runtime.InteropServices.CollectionsMarshal.AsSpan(output)[(table + y * 8)..], (ulong)output.Count);
            output.AddRange(BitConverter.GetBytes(y));
            output.AddRange(BitConverter.GetBytes(w * 4 * 2));
            // Planar per line: every pixel of A, then B, G, R.
            foreach (var channel in new[] { 3, 2, 1, 0 })
            {
                for (var x = 0; x < w; x++)
                {
                    var at = (y * w + x) * 4;
                    var k = rgba[at + 3] / 255f;
                    var value = channel == 3 ? k : linear[rgba[at + channel]] * k;
                    var half = Half(value);
                    output.AddRange([(byte)half, (byte)(half >> 8)]);
                }
            }
        }
        return [.. output];
    }
}

using System.Buffers.Binary;
using System.IO.Compression;

using Bayleaf.Core;

namespace Bayleaf.Engine;

/// <summary>
/// The PNG files commands write (8-bit RGBA, from the PNG specification) and the size of one they
/// read. Decoding images belongs to <c>Bayleaf.Assets</c>.
/// </summary>
internal static class Png
{
    private static readonly uint[] CrcTable = Enumerable.Range(0, 256).Select(n =>
    {
        var c = (uint)n;
        for (var k = 0; k < 8; k++)
        {
            c = (c & 1) != 0 ? 0xEDB88320u ^ (c >> 1) : c >> 1;
        }
        return c;
    }).ToArray();

    private static uint Crc(ReadOnlySpan<byte> type, ReadOnlySpan<byte> data)
    {
        var c = 0xFFFFFFFFu;
        foreach (var b in type)
        {
            c = CrcTable[(c ^ b) & 0xFF] ^ (c >> 8);
        }
        foreach (var b in data)
        {
            c = CrcTable[(c ^ b) & 0xFF] ^ (c >> 8);
        }
        return c ^ 0xFFFFFFFFu;
    }

    private static void Chunk(Stream output, string type, ReadOnlySpan<byte> data)
    {
        Span<byte> word = stackalloc byte[4];
        BinaryPrimitives.WriteUInt32BigEndian(word, (uint)data.Length);
        output.Write(word);
        var name = System.Text.Encoding.ASCII.GetBytes(type);
        output.Write(name);
        output.Write(data);
        BinaryPrimitives.WriteUInt32BigEndian(word, Crc(name, data));
        output.Write(word);
    }

    /// <summary>The PNG file of an RGBA image (4 bytes a pixel, rows top first); marked as sRGB (perceptual) when <paramref name="srgb"/> is set, as rendered pictures are.</summary>
    public static byte[] Encode(int width, int height, ReadOnlySpan<byte> rgba, bool srgb = false)
    {
        using var output = new MemoryStream();
        output.Write([0x89, (byte)'P', (byte)'N', (byte)'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        var header = new byte[13];
        BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(0), (uint)width);
        BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(4), (uint)height);
        header[8] = 8;
        header[9] = 6;
        Chunk(output, "IHDR", header);
        if (srgb)
        {
            Chunk(output, "sRGB", [0]);
        }
        using (var raw = new MemoryStream())
        {
            using (var z = new ZLibStream(raw, CompressionLevel.Optimal, true))
            {
                for (var y = 0; y < height; y++)
                {
                    z.WriteByte(0);
                    z.Write(rgba.Slice(y * width * 4, width * 4));
                }
            }
            Chunk(output, "IDAT", raw.ToArray());
        }
        Chunk(output, "IEND", []);
        return output.ToArray();
    }

    public static Result<Unit, string> Write(string path, int width, int height, byte[] rgba) => Files.Write(path, Encode(width, height, rgba));

    /// <summary>The width and height of a PNG file, from its header.</summary>
    public static Result<(uint Width, uint Height), string> Size(string path)
    {
        var bytes = Files.Read(path);
        if (bytes.IsError)
        {
            return Result.Fail(bytes.Error);
        }
        var b = bytes.Value;
        ReadOnlySpan<byte> signature = [0x89, (byte)'P', (byte)'N', (byte)'G', 0x0D, 0x0A, 0x1A, 0x0A];
        if (b.Length < 24 || !b.AsSpan(0, 8).SequenceEqual(signature) || System.Text.Encoding.ASCII.GetString(b, 12, 4) != "IHDR")
        {
            return Result.Fail("not a PNG image (the C# engine reads the size of PNG images only, for now)");
        }
        return (BinaryPrimitives.ReadUInt32BigEndian(b.AsSpan(16)), BinaryPrimitives.ReadUInt32BigEndian(b.AsSpan(20)));
    }
}

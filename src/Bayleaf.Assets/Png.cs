using System.Buffers.Binary;
using System.IO.Compression;

using Bayleaf.Core;

namespace Bayleaf.Assets;

/// <summary>
/// A PNG reader written from the PNG specification (W3C, third edition): every colour type and
/// bit depth, Adam7 interlacing, and transparency chunks. Palettes and low bit depths become 8-bit
/// channels, 16-bit channels become 8-bit (their high byte), and every image comes out as RGBA.
/// </summary>
internal static class Png
{
    private static ReadOnlySpan<byte> Signature => [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    private sealed class Broken(string message) : Exception(message);

    private static readonly uint[] CrcTable = MakeCrcTable();

    private static uint[] MakeCrcTable()
    {
        var table = new uint[256];
        for (uint n = 0; n < 256; n++)
        {
            var c = n;
            for (var k = 0; k < 8; k++)
            {
                c = (c & 1) != 0 ? 0xEDB88320u ^ (c >> 1) : c >> 1;
            }
            table[n] = c;
        }
        return table;
    }

    private static uint Crc(ReadOnlySpan<byte> bytes)
    {
        var c = 0xFFFFFFFFu;
        foreach (var b in bytes)
        {
            c = CrcTable[(c ^ b) & 0xFF] ^ (c >> 8);
        }
        return c ^ 0xFFFFFFFFu;
    }

    private sealed record Header(int Width, int Height, int Depth, int ColorType, bool Interlaced)
    {
        /// <summary>Samples a pixel holds as stored.</summary>
        public int Channels => ColorType switch
        {
            0 => 1,
            2 => 3,
            3 => 1,
            4 => 2,
            _ => 4,
        };

        public int BitsPerPixel => Channels * Depth;

        /// <summary>Bytes of one stored row of <paramref name="width"/> pixels.</summary>
        public long RowBytes(int width) => ((long)width * BitsPerPixel + 7) / 8;
    }

    public static Result<Image, string> Decode(ReadOnlySpan<byte> bytes)
    {
        Header header;
        byte[]? palette = null;
        byte[]? transparency = null;
        var data = new MemoryStream();
        int at;
        // The header and the chunks before the image data: what the Rust engine's reader reads as the image's info.
        try
        {
            if (!bytes.StartsWith(Signature))
            {
                throw new Broken("wrong signature");
            }
            at = Signature.Length;
            var type = NextChunk(bytes, ref at, out var range);
            var body = bytes[range];
            if (type != "IHDR" || body.Length != 13)
            {
                throw new Broken("the first chunk is not IHDR");
            }
            header = new((int)Math.Min(BinaryPrimitives.ReadUInt32BigEndian(body), int.MaxValue), (int)Math.Min(BinaryPrimitives.ReadUInt32BigEndian(body[4..]), int.MaxValue),
                body[8], body[9], body[12] == 1);
            var allowed = header.ColorType switch
            {
                0 => header.Depth is 1 or 2 or 4 or 8 or 16,
                2 or 4 or 6 => header.Depth is 8 or 16,
                3 => header.Depth is 1 or 2 or 4 or 8,
                _ => false,
            };
            if (!allowed || body[10] != 0 || body[11] != 0 || body[12] > 1)
            {
                throw new Broken($"invalid colour type {header.ColorType} with bit depth {header.Depth}");
            }
            while (true)
            {
                var chunkAt = at;
                type = NextChunk(bytes, ref at, out range);
                body = bytes[range];
                if (type == "IDAT")
                {
                    at = chunkAt;
                    break;
                }
                if (type == "IEND")
                {
                    throw new Broken("no image data");
                }
                if (type == "PLTE")
                {
                    palette = body.ToArray();
                }
                else if (type == "tRNS")
                {
                    transparency = body.ToArray();
                }
            }
        }
        catch (Broken e)
        {
            return Result.Fail($"not a PNG image: {e.Message}");
        }
        if (header.Width == 0 || header.Height == 0 || header.Width > Loader.MaxImageSide || header.Height > Loader.MaxImageSide)
        {
            return Result.Fail($"images may be at most {Loader.MaxImageSide} pixels on a side, not {header.Width} x {header.Height}");
        }
        try
        {
            if (header.ColorType == 3 && (palette is null || palette.Length % 3 != 0 || palette.Length == 0))
            {
                throw new Broken("a palette image without a palette");
            }
            // The image data: one or more IDAT chunks in a row.
            while (true)
            {
                var type = NextChunk(bytes, ref at, out var range);
                var body = bytes[range];
                if (type != "IDAT")
                {
                    break;
                }
                data.Write(body);
            }
            var raw = Inflate(data.ToArray(), RawSize(header));
            return Expand(header, Unfilter(header, raw), palette, transparency);
        }
        catch (Broken e)
        {
            return Result.Fail($"broken PNG image: {e.Message}");
        }
    }

    /// <summary>The next chunk's type and data, its checksum checked.</summary>
    private static string NextChunk(ReadOnlySpan<byte> bytes, ref int at, out Range body)
    {
        if (bytes.Length - at < 12)
        {
            throw new Broken("unexpected end of file");
        }
        var length = BinaryPrimitives.ReadUInt32BigEndian(bytes[at..]);
        if (length > int.MaxValue || bytes.Length - at - 12 < length)
        {
            throw new Broken("unexpected end of file");
        }
        var typed = bytes.Slice(at + 4, 4 + (int)length);
        var stored = BinaryPrimitives.ReadUInt32BigEndian(bytes[(at + 8 + (int)length)..]);
        if (Crc(typed) != stored)
        {
            throw new Broken("CRC error");
        }
        var type = string.Create(4, typed[..4].ToArray(), static (span, b) =>
        {
            for (var i = 0; i < 4; i++)
            {
                span[i] = (char)b[i];
            }
        });
        body = new Range(at + 8, at + 8 + (int)length);
        at += 12 + (int)length;
        return type;
    }

    /// <summary>Bytes of filtered rows the image's data holds (a filter byte before each row of each pass).</summary>
    private static long RawSize(Header h)
    {
        if (!h.Interlaced)
        {
            return (h.RowBytes(h.Width) + 1) * h.Height;
        }
        long total = 0;
        foreach (var (x0, y0, dx, dy) in Passes)
        {
            var (w, ht) = PassSize(h, x0, y0, dx, dy);
            if (w > 0 && ht > 0)
            {
                total += (h.RowBytes(w) + 1) * ht;
            }
        }
        return total;
    }

    private static readonly (int X0, int Y0, int Dx, int Dy)[] Passes = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

    private static (int Width, int Height) PassSize(Header h, int x0, int y0, int dx, int dy) =>
        ((h.Width - x0 + dx - 1) / dx, (h.Height - y0 + dy - 1) / dy);

    /// <summary>The zlib stream's data: its header checked, the deflate data read, its checksum not checked.</summary>
    private static byte[] Inflate(byte[] stream, long size)
    {
        if (stream.Length < 2 || (stream[0] & 0x0F) != 8 || (stream[0] >> 4) > 7 || ((stream[0] << 8) | stream[1]) % 31 != 0 || (stream[1] & 0x20) != 0)
        {
            throw new Broken("corrupt deflate stream");
        }
        if (size > int.MaxValue)
        {
            throw new Broken("the image is too large");
        }
        var output = new byte[size];
        try
        {
            using var deflate = new DeflateStream(new MemoryStream(stream, 2, stream.Length - 2), CompressionMode.Decompress);
            var filled = 0;
            while (filled < output.Length)
            {
                var read = deflate.Read(output, filled, output.Length - filled);
                if (read == 0)
                {
                    break;
                }
                filled += read;
            }
            if (filled < output.Length)
            {
                throw new Broken("unexpected end of image data");
            }
        }
        catch (InvalidDataException e)
        {
            throw new Broken($"corrupt deflate stream: {e.Message}");
        }
        return output;
    }

    /// <summary>Each pass's rows with their filters undone, as one image of stored rows (no filter bytes).</summary>
    private static byte[] Unfilter(Header h, byte[] raw)
    {
        var stride = h.RowBytes(h.Width);
        var image = new byte[stride * h.Height];
        var bpp = Math.Max(1, h.BitsPerPixel / 8);
        var at = 0;
        if (!h.Interlaced)
        {
            UnfilterPass(raw, ref at, image, (int)stride, h.Height, bpp);
            return image;
        }
        foreach (var (x0, y0, dx, dy) in Passes)
        {
            var (w, ht) = PassSize(h, x0, y0, dx, dy);
            if (w <= 0 || ht <= 0)
            {
                continue;
            }
            var passStride = (int)h.RowBytes(w);
            var pass = new byte[passStride * ht];
            UnfilterPass(raw, ref at, pass, passStride, ht, bpp);
            // Each pass pixel to its place in the image.
            for (var y = 0; y < ht; y++)
            {
                for (var x = 0; x < w; x++)
                {
                    CopyPixel(h, pass, y * passStride, x, image, (y0 + y * dy) * (int)stride, x0 + x * dx);
                }
            }
        }
        return image;
    }

    private static void CopyPixel(Header h, byte[] from, int fromRow, int fromX, byte[] to, int toRow, int toX)
    {
        var bits = h.BitsPerPixel;
        if (bits >= 8)
        {
            var n = bits / 8;
            Array.Copy(from, fromRow + fromX * n, to, toRow + toX * n, n);
            return;
        }
        var value = (from[fromRow + fromX * bits / 8] >> (8 - bits - fromX * bits % 8)) & ((1 << bits) - 1);
        var shift = 8 - bits - toX * bits % 8;
        ref var target = ref to[toRow + toX * bits / 8];
        target = (byte)((target & ~(((1 << bits) - 1) << shift)) | (value << shift));
    }

    private static void UnfilterPass(byte[] raw, ref int at, byte[] output, int stride, int rows, int bpp)
    {
        for (var y = 0; y < rows; y++)
        {
            var filter = raw[at++];
            var row = output.AsSpan(y * stride, stride);
            raw.AsSpan(at, stride).CopyTo(row);
            at += stride;
            ReadOnlySpan<byte> prior = y > 0 ? output.AsSpan((y - 1) * stride, stride) : new byte[stride];
            switch (filter)
            {
                case 0:
                    break;
                case 1:
                    for (var i = bpp; i < stride; i++)
                    {
                        row[i] += row[i - bpp];
                    }
                    break;
                case 2:
                    for (var i = 0; i < stride; i++)
                    {
                        row[i] += prior[i];
                    }
                    break;
                case 3:
                    for (var i = 0; i < stride; i++)
                    {
                        var left = i >= bpp ? row[i - bpp] : 0;
                        row[i] += (byte)((left + prior[i]) >> 1);
                    }
                    break;
                case 4:
                    for (var i = 0; i < stride; i++)
                    {
                        int a = i >= bpp ? row[i - bpp] : 0, b = prior[i], c = i >= bpp ? prior[i - bpp] : 0;
                        var p = a + b - c;
                        var (pa, pb, pc) = (Math.Abs(p - a), Math.Abs(p - b), Math.Abs(p - c));
                        row[i] += (byte)(pa <= pb && pa <= pc ? a : pb <= pc ? b : c);
                    }
                    break;
                default:
                    throw new Broken($"unknown filter method {filter}");
            }
        }
    }

    /// <summary>Stored rows as RGBA: palettes looked up, low depths scaled to 8 bits, 16 bits cut to their high byte, transparency applied.</summary>
    private static Image Expand(Header h, byte[] rows, byte[]? palette, byte[]? transparency)
    {
        var (w, ht) = (h.Width, h.Height);
        var stride = (int)h.RowBytes(w);
        var rgba = new byte[(long)w * ht * 4];
        // The usual kinds, 8-bit RGBA and RGB without a transparent colour, without the general path's per-sample work.
        if (h.Depth == 8 && (h.ColorType == 6 || (h.ColorType == 2 && transparency is not { Length: 6 })))
        {
            for (var y = 0; y < ht; y++)
            {
                var row = rows.AsSpan(y * stride, stride);
                var to = rgba.AsSpan(y * w * 4, w * 4);
                if (h.ColorType == 6)
                {
                    row.CopyTo(to);
                    continue;
                }
                for (var x = 0; x < w; x++)
                {
                    (to[x * 4], to[x * 4 + 1], to[x * 4 + 2], to[x * 4 + 3]) = (row[x * 3], row[x * 3 + 1], row[x * 3 + 2], 255);
                }
            }
            return new Image(0, w, ht, rgba);
        }
        int Sample(int y, int x, int channel)
        {
            var row = y * stride;
            if (h.Depth == 16)
            {
                var at = row + (x * h.Channels + channel) * 2;
                return (rows[at] << 8) | rows[at + 1];
            }
            if (h.Depth == 8)
            {
                return rows[row + x * h.Channels + channel];
            }
            var bit = x * h.Depth;
            return (rows[row + bit / 8] >> (8 - h.Depth - bit % 8)) & ((1 << h.Depth) - 1);
        }
        byte Eight(int value) => h.Depth switch
        {
            16 => (byte)(value >> 8),
            8 => (byte)value,
            _ => (byte)(value * (255 / ((1 << h.Depth) - 1))),
        };
        int? Key(int i) => transparency is { Length: >= 2 } t && t.Length >= (i + 1) * 2 ? (t[i * 2] << 8) | t[i * 2 + 1] : null;
        for (var y = 0; y < ht; y++)
        {
            for (var x = 0; x < w; x++)
            {
                var o = ((long)y * w + x) * 4;
                switch (h.ColorType)
                {
                    case 0:
                    {
                        var g = Sample(y, x, 0);
                        var e = Eight(g);
                        var key = transparency is { Length: 2 } ? Key(0) : null;
                        (rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]) = (e, e, e, key == g ? (byte)0 : (byte)255);
                        break;
                    }
                    case 2:
                    {
                        var (r, g, b) = (Sample(y, x, 0), Sample(y, x, 1), Sample(y, x, 2));
                        var keyed = transparency is { Length: 6 } && Key(0) == r && Key(1) == g && Key(2) == b;
                        (rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]) = (Eight(r), Eight(g), Eight(b), keyed ? (byte)0 : (byte)255);
                        break;
                    }
                    case 3:
                    {
                        var i = Sample(y, x, 0);
                        var p = palette!;
                        if (i * 3 + 2 < p.Length)
                        {
                            (rgba[o], rgba[o + 1], rgba[o + 2]) = (p[i * 3], p[i * 3 + 1], p[i * 3 + 2]);
                        }
                        rgba[o + 3] = transparency is not null && i < transparency.Length ? transparency[i] : (byte)255;
                        break;
                    }
                    case 4:
                    {
                        var g = Eight(Sample(y, x, 0));
                        (rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]) = (g, g, g, Eight(Sample(y, x, 1)));
                        break;
                    }
                    default:
                        (rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]) = (Eight(Sample(y, x, 0)), Eight(Sample(y, x, 1)), Eight(Sample(y, x, 2)), Eight(Sample(y, x, 3)));
                        break;
                }
            }
        }
        return new Image(0, w, ht, rgba);
    }
}

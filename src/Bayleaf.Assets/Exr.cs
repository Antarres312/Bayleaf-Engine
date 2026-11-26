using System.Buffers.Binary;
using System.IO.Compression;
using System.Text;

using Bayleaf.Core;

namespace Bayleaf.Assets;

/// <summary>
/// An OpenEXR reader written from the OpenEXR file layout documentation: single- and multi-part
/// files of flat scan lines or tiles (the full-resolution level), in half, float or unsigned
/// samples, stored as they are or with RLE, ZIPS, ZIP or PXR24 compression. PIZ, B44, B44A, DWAA,
/// DWAB and HTJ2K are refused with a message saying so. The channel naming and colour channel
/// choice are ported from craft-exr (storytold/craft-libs, Copyright (c) 2026 ArtCraft Team and the craft-libs contributors, MIT OR Apache-2.0; see ATTRIBUTION.md).
/// </summary>
internal static class Exr
{
    /// <summary>Most pixels a picture may have (8192 x 8192).</summary>
    private const long MaxPixels = 1L << 26;

    private const long MaxBytes = MaxPixels * 20;
    private const long MaxSide = 1L << 18;
    private const long MaxRatio = 1L << 16;

    private sealed class Malformed(string message) : Exception(message);

    private sealed class Unsupported(string message) : Exception(message);

    private sealed class LimitExceeded(string message) : Exception(message);

    private enum Sample
    {
        UInt = 0,
        Half = 1,
        Float = 2,
    }

    private sealed record ChannelInfo(string Name, Sample Sample, int XSampling, int YSampling)
    {
        public int Bytes => Sample == Sample.Half ? 2 : 4;
    }

    private readonly record struct Box(int X, int Y, long Width, long Height)
    {
        public long XMax => X + Width - 1;

        public long YMax => Y + Height - 1;
    }

    private sealed record Tiles(int XSize, int YSize, int Mode, bool RoundUp);

    private sealed class PartInfo
    {
        public int Index { get; init; }

        public string? Name { get; set; }

        public Box Data { get; set; }

        public Box Display { get; set; }

        public bool Deep { get; set; }

        public Tiles? Tiles { get; set; }

        public int Compression { get; set; } = -1;

        public List<ChannelInfo> Channels { get; } = [];

        public int? ChunkCount { get; set; }

        public HashSet<string> Seen { get; } = new(StringComparer.Ordinal);

        public long[] Offsets { get; set; } = [];

        public int ColorRank => ColorChannels(Channels.Select(c => c.Name).ToList()) is { } c ? c.Indices.Count : 0;
    }

    private static readonly string[] Wanted = ["R", "G", "B", "A", "Y"];

    private static readonly string[] CompressionNames = ["NONE", "RLE", "ZIPS", "ZIP", "PIZ", "PXR24", "B44", "B44A", "DWAA", "DWAB", "HTJ2K256", "HTJ2K32"];

    /// <summary>Scan lines a chunk holds, by compression.</summary>
    private static int LinesPerBlock(int compression) => compression switch
    {
        0 or 1 or 2 => 1,
        3 or 5 => 16,
        9 or 10 => 256,
        _ => 32,
    };

    public static Result<Image, string> DecodeImage(ReadOnlySpan<byte> bytes)
    {
        try
        {
            return Decode(bytes);
        }
        catch (Malformed e)
        {
            return Result.Fail($"OpenEXR: {e.Message}");
        }
        catch (Unsupported e)
        {
            return Result.Fail($"unsupported OpenEXR: {e.Message}");
        }
        catch (LimitExceeded e)
        {
            return Result.Fail($"OpenEXR exceeds a limit: {e.Message}");
        }
        catch (NoColour e)
        {
            return Result.Fail(e.Message);
        }
        catch (NoRgb e)
        {
            return Result.Fail(e.Message);
        }
    }

    private static Image Decode(ReadOnlySpan<byte> bytes)
    {
        var parts = ReadHeaders(bytes);
        CheckLimits(parts, bytes.Length);
        // The part with the most colour channels, the earliest on a tie.
        PartInfo? best = null;
        foreach (var p in parts)
        {
            if (p.ColorRank > 0 && (best is null || p.ColorRank > best.ColorRank))
            {
                best = p;
            }
        }
        if (best is null)
        {
            throw new NoColour();
        }
        var named = Wanted.Select(c => PartChannelName(best.Name, c)).ToArray();
        var planes = DecodeDisplay(bytes, parts, best, name => named.Contains(name, StringComparer.Ordinal), name => name == named[3] ? 1f : 0f);
        var (width, height) = ((int)best.Display.Width, (int)best.Display.Height);
        var pixels = width * height;
        float[]? Channel(string name) => planes.FirstOrDefault(p => p.Name == name).Samples is { } s && s.Length == pixels ? s : null;
        var grey = Channel(named[4]);
        var (r, g, b) = (Channel(named[0]) ?? grey, Channel(named[1]) ?? grey, Channel(named[2]) ?? grey);
        if (r is null || g is null || b is null)
        {
            throw new NoRgb();
        }
        var alpha = Channel(named[3]);
        static byte ToByte(float v) => FMath.ToByte(FMath.Round(FMath.Clamp(v, 0f, 1f) * 255f));
        static float Finite(float v) => float.IsFinite(v) ? v : 0f;
        var rgba = new byte[pixels * 4];
        for (var i = 0; i < pixels; i++)
        {
            rgba[i * 4] = ToByte(FMath.LinearToSrgb(FMath.Clamp(Finite(r[i]), 0f, 1f)));
            rgba[i * 4 + 1] = ToByte(FMath.LinearToSrgb(FMath.Clamp(Finite(g[i]), 0f, 1f)));
            rgba[i * 4 + 2] = ToByte(FMath.LinearToSrgb(FMath.Clamp(Finite(b[i]), 0f, 1f)));
            rgba[i * 4 + 3] = alpha is null ? (byte)255 : ToByte(Finite(alpha[i]));
        }
        return new Image(0, width, height, rgba);
    }

    /// <summary>The engine's own refusals, worded as the Rust engine words them (without the OpenEXR prefix).</summary>
    private sealed class NoColour() : Exception("the picture has no colour channels");

    private sealed class NoRgb() : Exception("the picture has no R, G and B (or Y) channels");

    private sealed class Reader(byte[] bytes)
    {
        public int At { get; set; }

        public byte[] Bytes => bytes;

        public int Left => bytes.Length - At;

        public string CString(string what)
        {
            var end = Array.IndexOf(bytes, (byte)0, At);
            if (end < 0)
            {
                throw new Malformed($"{what} runs past the end of the file");
            }
            var text = Encoding.Latin1.GetString(bytes, At, end - At);
            At = end + 1;
            return text;
        }

        public int Int32()
        {
            if (Left < 4)
            {
                throw new Malformed("unexpected end of file");
            }
            var v = BinaryPrimitives.ReadInt32LittleEndian(bytes.AsSpan(At));
            At += 4;
            return v;
        }

        public ulong UInt64()
        {
            if (Left < 8)
            {
                throw new Malformed("unexpected end of file");
            }
            var v = BinaryPrimitives.ReadUInt64LittleEndian(bytes.AsSpan(At));
            At += 8;
            return v;
        }
    }

    private static List<PartInfo> ReadHeaders(ReadOnlySpan<byte> span)
    {
        var bytes = span.ToArray();
        var r = new Reader(bytes) { At = 4 };
        var version = r.Int32();
        if ((version & 0xFF) != 2)
        {
            throw new Unsupported($"file format version {version & 0xFF}");
        }
        var (singleTiled, nonImage, multipart) = ((version & 0x200) != 0, (version & 0x800) != 0, (version & 0x1000) != 0);
        var parts = new List<PartInfo>();
        while (true)
        {
            if (multipart && r.Left > 0 && bytes[r.At] == 0)
            {
                r.At++;
                break;
            }
            var part = new PartInfo { Index = parts.Count };
            string? type = null;
            while (true)
            {
                var name = r.CString("an attribute name");
                if (name.Length == 0)
                {
                    break;
                }
                var kind = r.CString("an attribute type");
                var size = r.Int32();
                if (size < 0 || size > r.Left)
                {
                    throw new Malformed($"attribute {name} is larger than the file");
                }
                var value = bytes.AsSpan(r.At, size);
                r.At += size;
                part.Seen.Add(name);
                switch (name)
                {
                    case "channels" when kind == "chlist":
                        ReadChannels(value, part.Channels);
                        break;
                    case "compression" when kind == "compression" && size >= 1:
                        part.Compression = value[0];
                        break;
                    case "dataWindow" when kind == "box2i" && size >= 16:
                        part.Data = ReadBox(value, "data window");
                        break;
                    case "displayWindow" when kind == "box2i" && size >= 16:
                        part.Display = ReadBox(value, "display window");
                        break;
                    case "tiles" when kind == "tiledesc" && size >= 9:
                        var (xs, ys) = (BinaryPrimitives.ReadUInt32LittleEndian(value), BinaryPrimitives.ReadUInt32LittleEndian(value[4..]));
                        if (xs == 0 || ys == 0 || xs > int.MaxValue || ys > int.MaxValue)
                        {
                            throw new Malformed("invalid tile size");
                        }
                        part.Tiles = new((int)xs, (int)ys, value[8] & 0x0F, (value[8] >> 4 & 1) != 0);
                        break;
                    case "name" when kind == "string":
                        part.Name = Encoding.UTF8.GetString(value);
                        break;
                    case "type" when kind == "string":
                        type = Encoding.UTF8.GetString(value);
                        break;
                    case "chunkCount" when kind == "int" && size >= 4:
                        part.ChunkCount = BinaryPrimitives.ReadInt32LittleEndian(value);
                        break;
                    default:
                        break;
                }
            }
            foreach (var required in new[] { "channels", "compression", "dataWindow", "displayWindow", "lineOrder", "pixelAspectRatio", "screenWindowCenter", "screenWindowWidth" })
            {
                if (!part.Seen.Contains(required))
                {
                    throw new Malformed($"missing attribute {required}");
                }
            }
            if (part.Compression < 0 || part.Compression >= CompressionNames.Length)
            {
                throw new Malformed($"unknown compression {part.Compression}");
            }
            part.Deep = type is "deepscanline" or "deeptile" || (!multipart && nonImage);
            var tiled = type is "tiledimage" or "deeptile" || (!multipart && singleTiled);
            if (tiled && part.Tiles is null)
            {
                throw new Malformed("missing attribute tiles");
            }
            if (!tiled)
            {
                part.Tiles = null;
            }
            parts.Add(part);
            if (!multipart)
            {
                break;
            }
        }
        if (parts.Count == 0)
        {
            throw new Malformed("the file has no parts");
        }
        // Each part's chunk offsets, part after part.
        foreach (var part in parts)
        {
            var count = multipart ? part.ChunkCount ?? throw new Malformed("missing attribute chunkCount") : ChunkCount(part);
            if (count < 0 || (long)count * 8 > r.Left)
            {
                throw new Malformed("the chunk offset table is larger than the file");
            }
            var offsets = new long[count];
            for (var i = 0; i < count; i++)
            {
                var offset = r.UInt64();
                offsets[i] = offset > (ulong)bytes.Length ? -1 : (long)offset;
            }
            part.Offsets = offsets;
        }
        return parts;
    }

    private static Box ReadBox(ReadOnlySpan<byte> v, string what)
    {
        var (x0, y0, x1, y1) = (BinaryPrimitives.ReadInt32LittleEndian(v), BinaryPrimitives.ReadInt32LittleEndian(v[4..]),
            BinaryPrimitives.ReadInt32LittleEndian(v[8..]), BinaryPrimitives.ReadInt32LittleEndian(v[12..]));
        if (x1 < x0 || y1 < y0)
        {
            throw new Malformed($"invalid {what}");
        }
        return new(x0, y0, (long)x1 - x0 + 1, (long)y1 - y0 + 1);
    }

    private static void ReadChannels(ReadOnlySpan<byte> v, List<ChannelInfo> channels)
    {
        var at = 0;
        while (at < v.Length && v[at] != 0)
        {
            var end = v[at..].IndexOf((byte)0);
            if (end < 0 || v.Length - (at + end + 1) < 16)
            {
                throw new Malformed("broken channel list");
            }
            var name = Encoding.UTF8.GetString(v.Slice(at, end));
            at += end + 1;
            var type = BinaryPrimitives.ReadInt32LittleEndian(v[at..]);
            var (xs, ys) = (BinaryPrimitives.ReadInt32LittleEndian(v[(at + 8)..]), BinaryPrimitives.ReadInt32LittleEndian(v[(at + 12)..]));
            at += 16;
            if (type is < 0 or > 2 || xs < 1 || ys < 1)
            {
                throw new Malformed($"invalid channel {name}");
            }
            channels.Add(new(name, (Sample)type, xs, ys));
        }
        if (channels.Count == 0)
        {
            throw new Malformed("no channels");
        }
    }

    /// <summary>Levels along one side of a tiled part.</summary>
    private static int LevelCount(long size, bool roundUp)
    {
        var levels = 1;
        while (size > 1)
        {
            size = roundUp ? (size + 1) / 2 : size / 2;
            levels++;
        }
        return levels;
    }

    private static long LevelSize(long size, int level, bool roundUp)
    {
        for (var i = 0; i < level; i++)
        {
            size = Math.Max(1, roundUp ? (size + 1) / 2 : size / 2);
        }
        return size;
    }

    private static int ChunkCount(PartInfo part)
    {
        if (part.Tiles is not { } t)
        {
            var lines = LinesPerBlock(part.Compression);
            return (int)Math.Min(int.MaxValue, (part.Data.Height + lines - 1) / lines);
        }
        long Tiles(long w, long h) => (w + t.XSize - 1) / t.XSize * ((h + t.YSize - 1) / t.YSize);
        var (width, height) = (part.Data.Width, part.Data.Height);
        long total = 0;
        switch (t.Mode)
        {
            case 0:
                total = Tiles(width, height);
                break;
            case 1:
                var levels = LevelCount(Math.Max(width, height), t.RoundUp);
                for (var l = 0; l < levels; l++)
                {
                    total += Tiles(LevelSize(width, l, t.RoundUp), LevelSize(height, l, t.RoundUp));
                }
                break;
            case 2:
                var (lx, ly) = (LevelCount(width, t.RoundUp), LevelCount(height, t.RoundUp));
                for (var y = 0; y < ly; y++)
                {
                    for (var x = 0; x < lx; x++)
                    {
                        total += Tiles(LevelSize(width, x, t.RoundUp), LevelSize(height, y, t.RoundUp));
                    }
                }
                break;
            default:
                throw new Malformed($"unknown tile level mode {t.Mode}");
        }
        return (int)Math.Min(int.MaxValue, total);
    }

    /// <summary>Every part's declared size against the limits, alone and together, before any pixel is read.</summary>
    private static void CheckLimits(List<PartInfo> parts, long fileLength)
    {
        long total = 0, flat = 0, dwa = 0;
        foreach (var part in parts)
        {
            var (w, h) = (part.Data.Width, part.Data.Height);
            if (w > MaxSide || h > MaxSide)
            {
                throw new LimitExceeded($"a {w}x{h} part exceeds {MaxSide}x{MaxSide}");
            }
            var pixels = w * h;
            if (pixels > MaxPixels)
            {
                throw new LimitExceeded($"a part of {pixels} pixels exceeds {MaxPixels}");
            }
            var bytes = pixels * Math.Max(part.Channels.Count, 1) * 4;
            total += bytes;
            if (!part.Deep)
            {
                flat += bytes;
                if (part.Compression is 8 or 9)
                {
                    dwa += bytes;
                }
            }
        }
        if (total > MaxBytes)
        {
            throw new LimitExceeded($"{total} bytes across {parts.Count} parts exceed {MaxBytes}");
        }
        var declared = flat - dwa + dwa / 4;
        var capacity = fileLength * MaxRatio + (1 << 20);
        if (declared > capacity)
        {
            throw new LimitExceeded($"a {fileLength} byte file cannot hold {declared} bytes of samples");
        }
    }

    /// <summary>Samples of a channel along <paramref name="from"/>..<paramref name="to"/> (inclusive) at every <paramref name="step"/>-th coordinate.</summary>
    private static int SampleCount(long from, long to, int step) => (int)Math.Max(0, FloorDiv(to, step) - CeilDiv(from, step) + 1);

    private static long FloorDiv(long a, long b) => a >= 0 ? a / b : -((-a + b - 1) / b);

    private static long CeilDiv(long a, long b) => -FloorDiv(-a, b);

    private static bool OnLine(long y, int step) => ((y % step) + step) % step == 0;

    private static List<(string Name, float[] Samples)> DecodeDisplay(ReadOnlySpan<byte> span, List<PartInfo> parts, PartInfo info, Func<string, bool> keep, Func<string, float> fill)
    {
        if (info.Deep)
        {
            throw new Unsupported("deep data (use the deep reader)");
        }
        var display = info.Display;
        var pixels = display.Width * display.Height;
        if (pixels > MaxPixels)
        {
            throw new LimitExceeded($"display window of {pixels} pixels exceeds {MaxPixels}");
        }
        var kept = info.Channels.Count(c => keep(PartChannelName(info.Name, c.Name)));
        var needed = pixels * 4 * kept;
        if (needed > MaxBytes)
        {
            throw new LimitExceeded($"{needed} display-window bytes exceed {MaxBytes}");
        }
        var lines = info.Tiles is { } tiles ? Math.Min(tiles.YSize, info.Data.Height) : Math.Min(LinesPerBlock(info.Compression), info.Data.Height);
        var blockWidth = info.Tiles is { } t2 ? Math.Min(t2.XSize, info.Data.Width) : info.Data.Width;
        var blockBytes = lines * blockWidth * info.Channels.Sum(c => c.Bytes);
        if (blockBytes > MaxBytes)
        {
            throw new LimitExceeded($"{blockBytes} bytes in one data-window block exceed {MaxBytes}");
        }
        var prefix = string.IsNullOrEmpty(info.Name) ? null : info.Name;
        var planes = new (string Name, float[] Samples)?[info.Channels.Count];
        for (var c = 0; c < info.Channels.Count; c++)
        {
            var channel = info.Channels[c];
            var name = PartChannelName(prefix, channel.Name);
            if (!keep(name))
            {
                continue;
            }
            if (channel.XSampling != 1 || channel.YSampling != 1)
            {
                throw new Unsupported("subsampled channels");
            }
            var plane = new float[pixels];
            Array.Fill(plane, fill(name));
            planes[c] = (name, plane);
        }
        if (planes.Any(p => p is not null))
        {
            if (info.Compression is 4 or 6 or 7 or 8 or 9 or 10 or 11)
            {
                throw new Unsupported($"{CompressionNames[info.Compression]} compression is not read yet");
            }
            var bytes = span.ToArray();
            var multipart = parts.Count > 1 || (BinaryPrimitives.ReadInt32LittleEndian(span[4..]) & 0x1000) != 0;
            foreach (var offset in info.Offsets)
            {
                ReadChunk(bytes, offset, multipart, info, planes);
            }
        }
        return [.. planes.OfType<(string, float[])>()];
    }

    private static void ReadChunk(byte[] bytes, long offset, bool multipart, PartInfo info, (string Name, float[] Samples)?[] planes)
    {
        if (offset < 0)
        {
            throw new Malformed("a chunk offset lies outside the file");
        }
        var r = new Reader(bytes) { At = (int)offset };
        if (multipart && r.Int32() != info.Index)
        {
            throw new Malformed("a chunk belongs to another part");
        }
        long x0, y0, x1, y1;
        var data = info.Data;
        if (info.Tiles is { } t)
        {
            var (tx, ty, lx, ly) = (r.Int32(), r.Int32(), r.Int32(), r.Int32());
            if (lx != 0 || ly != 0)
            {
                // Only the full-resolution level is read.
                return;
            }
            if (tx < 0 || ty < 0)
            {
                throw new Malformed("invalid tile coordinates");
            }
            x0 = data.X + (long)tx * t.XSize;
            y0 = data.Y + (long)ty * t.YSize;
            if (x0 > data.XMax || y0 > data.YMax)
            {
                throw new Malformed("a tile lies outside the data window");
            }
            (x1, y1) = (Math.Min(x0 + t.XSize - 1, data.XMax), Math.Min(y0 + t.YSize - 1, data.YMax));
        }
        else
        {
            y0 = r.Int32();
            if (y0 < data.Y || y0 > data.YMax)
            {
                throw new Malformed("a block lies outside the data window");
            }
            (x0, x1, y1) = (data.X, data.XMax, Math.Min(y0 + LinesPerBlock(info.Compression) - 1, data.YMax));
        }
        var size = r.Int32();
        if (size < 0 || size > r.Left)
        {
            throw new Malformed("a chunk is larger than the file");
        }
        var packed = bytes.AsSpan(r.At, size);
        long rawSize = 0;
        for (var y = y0; y <= y1; y++)
        {
            foreach (var c in info.Channels)
            {
                if (OnLine(y, c.YSampling))
                {
                    rawSize += (long)SampleCount(x0, x1, c.XSampling) * c.Bytes;
                }
            }
        }
        if (rawSize > int.MaxValue)
        {
            throw new LimitExceeded("a block is too large");
        }
        var raw = Uncompress(packed, (int)rawSize, info, x0, x1, y0, y1);
        var display = info.Display;
        var at = 0;
        for (var y = y0; y <= y1; y++)
        {
            for (var ci = 0; ci < info.Channels.Count; ci++)
            {
                var c = info.Channels[ci];
                if (!OnLine(y, c.YSampling))
                {
                    continue;
                }
                var count = SampleCount(x0, x1, c.XSampling);
                if (planes[ci] is { } plane && c.XSampling == 1)
                {
                    var ty = y - display.Y;
                    if (ty >= 0 && ty < display.Height)
                    {
                        for (var i = 0; i < count; i++)
                        {
                            var tx = x0 + i - display.X;
                            if (tx >= 0 && tx < display.Width)
                            {
                                plane.Samples[ty * display.Width + tx] = Value(raw, at + i * c.Bytes, c.Sample);
                            }
                        }
                    }
                }
                at += count * c.Bytes;
            }
        }
    }

    private static float Value(byte[] raw, int at, Sample sample) => sample switch
    {
        Sample.Half => (float)BitConverter.UInt16BitsToHalf(BinaryPrimitives.ReadUInt16LittleEndian(raw.AsSpan(at))),
        Sample.Float => BinaryPrimitives.ReadSingleLittleEndian(raw.AsSpan(at)),
        _ => BinaryPrimitives.ReadUInt32LittleEndian(raw.AsSpan(at)),
    };

    private static byte[] Uncompress(ReadOnlySpan<byte> packed, int rawSize, PartInfo info, long x0, long x1, long y0, long y1)
    {
        // A chunk that would not shrink is stored as it is.
        if (packed.Length == rawSize)
        {
            return packed.ToArray();
        }
        switch (info.Compression)
        {
            case 0:
                throw new Malformed("an uncompressed chunk has the wrong size");
            case 1:
                return Reorder(Rle(packed, rawSize));
            case 2:
            case 3:
                return Reorder(Inflate(packed, rawSize));
            default:
                return Pxr24(Inflate(packed, Pxr24Size(info, x0, x1, y0, y1)), rawSize, info, x0, x1, y0, y1);
        }
    }

    private static byte[] Rle(ReadOnlySpan<byte> packed, int size)
    {
        var output = new byte[size];
        var (at, to) = (0, 0);
        while (at < packed.Length)
        {
            var count = (sbyte)packed[at++];
            if (count < 0)
            {
                var n = -count;
                if (at + n > packed.Length || to + n > size)
                {
                    throw new Malformed("broken RLE data");
                }
                packed.Slice(at, n).CopyTo(output.AsSpan(to));
                (at, to) = (at + n, to + n);
            }
            else
            {
                var n = count + 1;
                if (at >= packed.Length || to + n > size)
                {
                    throw new Malformed("broken RLE data");
                }
                output.AsSpan(to, n).Fill(packed[at++]);
                to += n;
            }
        }
        if (to != size)
        {
            throw new Malformed("broken RLE data");
        }
        return output;
    }

    private static byte[] Inflate(ReadOnlySpan<byte> packed, int size)
    {
        if (packed.Length < 2 || (packed[0] & 0x0F) != 8 || ((packed[0] << 8) | packed[1]) % 31 != 0)
        {
            throw new Malformed("broken zlib data");
        }
        var output = new byte[size];
        try
        {
            using var deflate = new DeflateStream(new MemoryStream(packed[2..].ToArray()), CompressionMode.Decompress);
            var filled = 0;
            while (filled < size)
            {
                var read = deflate.Read(output, filled, size - filled);
                if (read == 0)
                {
                    break;
                }
                filled += read;
            }
            if (filled != size)
            {
                throw new Malformed("compressed data is shorter than its block");
            }
        }
        catch (InvalidDataException e)
        {
            throw new Malformed($"broken zlib data: {e.Message}");
        }
        return output;
    }

    /// <summary>The ZIP and RLE predictor and byte split undone: each byte the previous plus its difference, then the two halves interleaved.</summary>
    private static byte[] Reorder(byte[] t)
    {
        for (var i = 1; i < t.Length; i++)
        {
            t[i] = (byte)(t[i - 1] + t[i] - 128);
        }
        var output = new byte[t.Length];
        var half = (t.Length + 1) / 2;
        for (var i = 0; i < t.Length; i++)
        {
            output[i] = t[i % 2 == 0 ? i / 2 : half + i / 2];
        }
        return output;
    }

    /// <summary>Bytes PXR24 keeps for a block: floats cut to 24 bits.</summary>
    private static int Pxr24Size(PartInfo info, long x0, long x1, long y0, long y1)
    {
        long size = 0;
        for (var y = y0; y <= y1; y++)
        {
            foreach (var c in info.Channels)
            {
                if (OnLine(y, c.YSampling))
                {
                    size += (long)SampleCount(x0, x1, c.XSampling) * (c.Sample switch { Sample.Half => 2, Sample.Float => 3, _ => 4 });
                }
            }
        }
        return (int)Math.Min(size, int.MaxValue);
    }

    /// <summary>PXR24's byte planes of differences, line by line and channel by channel, back into samples.</summary>
    private static byte[] Pxr24(byte[] planes, int rawSize, PartInfo info, long x0, long x1, long y0, long y1)
    {
        var output = new byte[rawSize];
        var (at, to) = (0, 0);
        for (var y = y0; y <= y1; y++)
        {
            foreach (var c in info.Channels)
            {
                if (!OnLine(y, c.YSampling))
                {
                    continue;
                }
                var n = SampleCount(x0, x1, c.XSampling);
                uint pixel = 0;
                switch (c.Sample)
                {
                    case Sample.Half:
                        for (var j = 0; j < n; j++)
                        {
                            pixel = (ushort)(pixel + (uint)((planes[at + j] << 8) | planes[at + n + j]));
                            BinaryPrimitives.WriteUInt16LittleEndian(output.AsSpan(to), (ushort)pixel);
                            to += 2;
                        }
                        at += n * 2;
                        break;
                    case Sample.Float:
                        for (var j = 0; j < n; j++)
                        {
                            pixel += (uint)((planes[at + j] << 24) | (planes[at + n + j] << 16) | (planes[at + 2 * n + j] << 8));
                            BinaryPrimitives.WriteUInt32LittleEndian(output.AsSpan(to), pixel);
                            to += 4;
                        }
                        at += n * 3;
                        break;
                    default:
                        for (var j = 0; j < n; j++)
                        {
                            pixel += (uint)((planes[at + j] << 24) | (planes[at + n + j] << 16) | (planes[at + 2 * n + j] << 8) | planes[at + 3 * n + j]);
                            BinaryPrimitives.WriteUInt32LittleEndian(output.AsSpan(to), pixel);
                            to += 4;
                        }
                        at += n * 4;
                        break;
                }
            }
        }
        return output;
    }

    /// <summary>
    /// The full name of a channel of a named part: <c>part.channel</c>, except that a channel which
    /// already starts with the part's name is kept as is (ported from craft-exr).
    /// </summary>
    public static string PartChannelName(string? part, string channel)
    {
        if (part is null)
        {
            return channel;
        }
        return channel.StartsWith(part, StringComparison.Ordinal) && channel.Length > part.Length && channel[part.Length] == '.' ? channel : $"{part}.{channel}";
    }

    /// <summary>
    /// The channels of <paramref name="names"/> that make a colour picture, as the indices of its
    /// channels in layout order (R, G, B, A; or Y, A): never put together from separate layers;
    /// the group with the most channels wins, ties going to the unnamed layer, then the first
    /// prefix alphabetically; a file of exactly one channel is luminance (ported from craft-exr).
    /// </summary>
    public static (string Layout, List<int> Indices)? ColorChannels(IReadOnlyList<string> names)
    {
        var groups = new SortedDictionary<string, int?[]>(StringComparer.Ordinal);
        for (var index = 0; index < names.Count; index++)
        {
            var name = names[index];
            var dot = name.LastIndexOf('.');
            var (prefix, b) = dot > 0 && dot < name.Length - 1 ? (name[..dot], name[(dot + 1)..]) : ("", name);
            var slot = b switch
            {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                "A" => 3,
                "Y" => 4,
                _ => -1,
            };
            if (slot < 0)
            {
                continue;
            }
            if (!groups.TryGetValue(prefix, out var group))
            {
                group = new int?[5];
                groups[prefix] = group;
            }
            group[slot] ??= index;
        }
        (string, List<int>)? best = null;
        var bestRank = 0;
        foreach (var g in groups.Values)
        {
            (string, List<int>) candidate;
            if (g[0] is { } r && g[1] is { } gr && g[2] is { } bl)
            {
                candidate = g[3] is { } a ? ("RGBA", [r, gr, bl, a]) : ("RGB", [r, gr, bl]);
            }
            else if (g[4] is { } y)
            {
                candidate = g[3] is { } a ? ("YA", [y, a]) : ("Y", [y]);
            }
            else
            {
                continue;
            }
            if (candidate.Item2.Count > bestRank)
            {
                best = candidate;
                bestRank = candidate.Item2.Count;
            }
        }
        return best ?? (names.Count == 1 ? ("Y", [0]) : null);
    }
}

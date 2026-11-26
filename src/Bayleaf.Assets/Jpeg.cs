using Bayleaf.Core;

namespace Bayleaf.Assets;

/// <summary>
/// A JPEG reader written from ITU-T T.81 and JFIF 1.02: baseline, extended (8-bit) and progressive
/// Huffman-coded frames, any sampling factors, restart intervals, greyscale and YCbCr. Chroma at
/// half resolution is brought up with a triangle filter. Lossless, hierarchical, arithmetic-coded
/// and 12-bit frames, Adobe RGB and CMYK are refused with a message saying so. Data that ends early
/// decodes as far as it goes, the rest grey, as the Rust engine's reader does.
/// </summary>
internal static class Jpeg
{
    private sealed class Broken(string message) : Exception(message);

    private sealed class Component
    {
        public int Id { get; init; }

        public int H { get; init; }

        public int V { get; init; }

        public int Tq { get; init; }

        /// <summary>Blocks across and down in the padded MCU grid.</summary>
        public int BlocksWide { get; set; }

        public int BlocksHigh { get; set; }

        /// <summary>Blocks across and down that hold image samples (what a scan of this component alone covers).</summary>
        public int UsedWide { get; set; }

        public int UsedHigh { get; set; }

        public short[] Coefficients { get; set; } = [];

        public int Dc { get; set; }

        public int DcTable { get; set; }

        public int AcTable { get; set; }
    }

    private sealed class Huffman
    {
        public int[] MaxCode { get; } = new int[18];

        public int[] ValPtr { get; } = new int[17];

        public int[] MinCode { get; } = new int[17];

        public byte[] Values { get; init; } = [];

        /// <summary>For the next 9 bits: the code's length (0 when longer) and its value.</summary>
        public byte[] FastLength { get; } = new byte[512];

        public byte[] FastValue { get; } = new byte[512];
    }

    private static readonly byte[] ZigZag =
    [
        0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28,
        35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
    ];

    private sealed class Frame
    {
        public int Width { get; set; }

        public int Height { get; set; }

        public bool Progressive { get; set; }

        public List<Component> Components { get; } = [];

        public int HMax { get; set; }

        public int VMax { get; set; }

        public int McusWide { get; set; }

        public int McusHigh { get; set; }
    }

    private sealed class State
    {
        public ushort[][] Quant { get; } = new ushort[4][];

        public Huffman?[] Dc { get; } = new Huffman?[4];

        public Huffman?[] Ac { get; } = new Huffman?[4];

        public int RestartInterval { get; set; }

        public int? AdobeTransform { get; set; }

        public Frame? Frame { get; set; }
    }

    public static Result<Image, string> Decode(ReadOnlySpan<byte> span)
    {
        var bytes = span.ToArray();
        var state = new State();
        int at;
        // The headers, up to the first scan: what the Rust engine's reader reads as the image's headers.
        try
        {
            at = ReadHeaders(bytes, 2, state, untilScan: true);
        }
        catch (Broken e)
        {
            return Result.Fail($"not a JPEG image: {e.Message}");
        }
        var frame = state.Frame!;
        if (frame.Width == 0 || frame.Height == 0 || frame.Width > Loader.MaxImageSide || frame.Height > Loader.MaxImageSide)
        {
            return Result.Fail($"images may be at most {Loader.MaxImageSide} pixels on a side, not {frame.Width} x {frame.Height}");
        }
        try
        {
            Allocate(frame);
            // Scans, and the tables between them, to the end of the image.
            while (at < bytes.Length)
            {
                at = Scan(bytes, at, state);
                at = ReadHeaders(bytes, at, state, untilScan: true);
            }
            return Render(frame, state);
        }
        catch (Broken e)
        {
            return Result.Fail($"broken JPEG image: {e.Message}");
        }
    }

    private static int Be16(byte[] b, int at) => at + 1 < b.Length ? (b[at] << 8) | b[at + 1] : throw new Broken("unexpected end of file");

    /// <summary>Reads markers from <paramref name="at"/> up to a scan (giving the place of its SOS marker) or the end of the image (giving the file's length).</summary>
    private static int ReadHeaders(byte[] b, int at, State state, bool untilScan)
    {
        if (state.Frame is null && (b.Length < 2 || b[0] != 0xFF || b[1] != 0xD8))
        {
            throw new Broken("no start of image");
        }
        while (true)
        {
            // Fill bytes and stray data before a marker are passed over.
            while (at < b.Length && b[at] != 0xFF)
            {
                at++;
            }
            while (at < b.Length && b[at] == 0xFF)
            {
                at++;
            }
            if (at >= b.Length)
            {
                if (state.Frame is null || state.Frame.Components.Count == 0)
                {
                    throw new Broken("no frame before the end of the file");
                }
                return b.Length;
            }
            var marker = b[at++];
            if (marker == 0xD9)
            {
                if (state.Frame is null)
                {
                    throw new Broken("no frame before the end of the image");
                }
                return b.Length;
            }
            if (marker is 0xD8 or 0x01 or (>= 0xD0 and <= 0xD7))
            {
                continue;
            }
            var length = Be16(b, at);
            if (length < 2 || at + length > b.Length)
            {
                throw new Broken("a segment runs past the end of the file");
            }
            var body = b.AsSpan(at + 2, length - 2);
            switch (marker)
            {
                case 0xDA:
                    if (state.Frame is null)
                    {
                        throw new Broken("a scan before the frame header");
                    }
                    if (untilScan)
                    {
                        return at - 2;
                    }
                    break;
                case 0xC0 or 0xC1 or 0xC2:
                    ReadFrame(body, marker == 0xC2, state);
                    break;
                case 0xC3 or 0xC5 or 0xC6 or 0xC7:
                    throw new Broken("lossless and hierarchical JPEG are not supported");
                case >= 0xC9 and <= 0xCF and not 0xCC:
                    throw new Broken("arithmetic-coded JPEG is not supported");
                case 0xC4:
                    ReadHuffman(body, state);
                    break;
                case 0xDB:
                    ReadQuant(body, state);
                    break;
                case 0xDD:
                    state.RestartInterval = body.Length >= 2 ? (body[0] << 8) | body[1] : throw new Broken("a short restart interval");
                    break;
                case 0xEE:
                    if (body.Length >= 12 && body[..5].SequenceEqual("Adobe"u8))
                    {
                        state.AdobeTransform = body[11];
                    }
                    break;
                default:
                    break;
            }
            at += length;
        }
    }

    private static void ReadFrame(ReadOnlySpan<byte> body, bool progressive, State state)
    {
        if (state.Frame is not null)
        {
            throw new Broken("more than one frame");
        }
        if (body.Length < 6)
        {
            throw new Broken("a short frame header");
        }
        if (body[0] != 8)
        {
            throw new Broken($"{body[0]}-bit JPEG is not supported");
        }
        var frame = new Frame { Height = (body[1] << 8) | body[2], Width = (body[3] << 8) | body[4], Progressive = progressive };
        var count = body[5];
        if (count is not (1 or 3 or 4) || body.Length < 6 + count * 3)
        {
            throw new Broken($"{count} components");
        }
        for (var i = 0; i < count; i++)
        {
            var c = body.Slice(6 + i * 3, 3);
            var (h, v) = (c[1] >> 4, c[1] & 15);
            if (h is < 1 or > 4 || v is < 1 or > 4 || c[2] > 3)
            {
                throw new Broken("invalid sampling factors");
            }
            frame.Components.Add(new Component { Id = c[0], H = h, V = v, Tq = c[2] });
        }
        frame.HMax = frame.Components.Max(c => c.H);
        frame.VMax = frame.Components.Max(c => c.V);
        if (frame.Components.Any(c => frame.HMax % c.H != 0 || frame.VMax % c.V != 0))
        {
            throw new Broken("sampling factors that do not divide the largest");
        }
        state.Frame = frame;
    }

    private static void ReadQuant(ReadOnlySpan<byte> body, State state)
    {
        var at = 0;
        while (at < body.Length)
        {
            var (precision, id) = (body[at] >> 4, body[at] & 15);
            at++;
            var size = precision == 0 ? 64 : 128;
            if (id > 3 || precision > 1 || at + size > body.Length)
            {
                throw new Broken("a broken quantisation table");
            }
            var table = new ushort[64];
            for (var k = 0; k < 64; k++)
            {
                table[ZigZag[k]] = precision == 0 ? body[at + k] : (ushort)((body[at + k * 2] << 8) | body[at + k * 2 + 1]);
            }
            state.Quant[id] = table;
            at += size;
        }
    }

    private static void ReadHuffman(ReadOnlySpan<byte> body, State state)
    {
        var at = 0;
        while (at < body.Length)
        {
            if (at + 17 > body.Length)
            {
                throw new Broken("a broken Huffman table");
            }
            var (kind, id) = (body[at] >> 4, body[at] & 15);
            var counts = body.Slice(at + 1, 16);
            var total = 0;
            foreach (var n in counts)
            {
                total += n;
            }
            if (kind > 1 || id > 3 || total > 256 || at + 17 + total > body.Length)
            {
                throw new Broken("a broken Huffman table");
            }
            var table = new Huffman { Values = body.Slice(at + 17, total).ToArray() };
            // Canonical codes (T.81 Annex C), and the decoding limits of F.2.2.3.
            int code = 0, k = 0;
            for (var length = 1; length <= 16; length++)
            {
                table.ValPtr[length] = k;
                table.MinCode[length] = code;
                for (var i = 0; i < counts[length - 1]; i++)
                {
                    if (length <= 9)
                    {
                        var shift = 9 - length;
                        for (var fill = 0; fill < 1 << shift; fill++)
                        {
                            var index = (code << shift) | fill;
                            if (index < 512)
                            {
                                table.FastLength[index] = (byte)length;
                                table.FastValue[index] = table.Values[k];
                            }
                        }
                    }
                    code++;
                    k++;
                }
                table.MaxCode[length] = counts[length - 1] > 0 ? code - 1 : -1;
                code <<= 1;
            }
            table.MaxCode[17] = int.MaxValue;
            (kind == 0 ? state.Dc : state.Ac)[id] = table;
            at += 17 + total;
        }
    }

    private static void Allocate(Frame frame)
    {
        frame.McusWide = (frame.Width + 8 * frame.HMax - 1) / (8 * frame.HMax);
        frame.McusHigh = (frame.Height + 8 * frame.VMax - 1) / (8 * frame.VMax);
        foreach (var c in frame.Components)
        {
            c.BlocksWide = frame.McusWide * c.H;
            c.BlocksHigh = frame.McusHigh * c.V;
            c.UsedWide = ((frame.Width * c.H + frame.HMax - 1) / frame.HMax + 7) / 8;
            c.UsedHigh = ((frame.Height * c.V + frame.VMax - 1) / frame.VMax + 7) / 8;
            c.Coefficients = new short[(long)c.BlocksWide * c.BlocksHigh * 64];
        }
    }

    /// <summary>Entropy-coded bits: stuffed zero bytes dropped, and zeros once a marker or the end is reached.</summary>
    private sealed class Bits(byte[] b, int at)
    {
        private uint buffer;
        private int count;

        /// <summary>How many of the bits held are the zeros given past the data.</summary>
        private int padding;

        /// <summary>Whether a code took bits past the data: the scan's data ended early.</summary>
        public bool Overrun { get; private set; }

        public int At { get; private set; } = at;

        private void Fill()
        {
            while (count <= 24)
            {
                uint next = 0;
                if (At < b.Length)
                {
                    if (b[At] == 0xFF)
                    {
                        if (At + 1 < b.Length && b[At + 1] == 0x00)
                        {
                            next = 0xFF;
                            At += 2;
                        }
                        else
                        {
                            // A marker: zeros from here on, the marker left for the caller.
                            padding += 8;
                        }
                    }
                    else
                    {
                        next = b[At++];
                    }
                }
                else
                {
                    padding += 8;
                }
                buffer |= next << (24 - count);
                count += 8;
            }
        }

        public int Peek9()
        {
            Fill();
            return (int)(buffer >> 23);
        }

        public void Skip(int n)
        {
            buffer <<= n;
            count -= n;
            if (count < padding)
            {
                padding = Math.Max(count, 0);
                Overrun = true;
            }
        }

        public int Bit()
        {
            Fill();
            var bit = (int)(buffer >> 31);
            Skip(1);
            return bit;
        }

        public int Receive(int n)
        {
            if (n == 0)
            {
                return 0;
            }
            Fill();
            var v = (int)(buffer >> (32 - n));
            Skip(n);
            return v;
        }

        /// <summary>A received value made signed (T.81 F.2.2.1, EXTEND).</summary>
        public int Extend(int n)
        {
            var v = Receive(n);
            return n == 0 ? 0 : v < 1 << (n - 1) ? v - (1 << n) + 1 : v;
        }

        public int Decode(Huffman h)
        {
            var peek = Peek9();
            var length = h.FastLength[peek];
            if (length > 0)
            {
                Skip(length);
                return h.FastValue[peek];
            }
            var code = 0;
            for (var l = 1; l <= 16; l++)
            {
                code = (code << 1) | Bit();
                if (code <= h.MaxCode[l])
                {
                    var index = h.ValPtr[l] + code - h.MinCode[l];
                    return index < h.Values.Length ? h.Values[index] : 0;
                }
            }
            throw new Broken("a Huffman code that is in no table");
        }

        /// <summary>Starts again after a restart marker: the bits left are dropped, the marker passed over.</summary>
        public void Restart()
        {
            buffer = 0;
            count = 0;
            padding = 0;
            var search = At;
            while (search + 1 < b.Length && !(b[search] == 0xFF && b[search + 1] is >= 0xD0 and <= 0xD7))
            {
                if (b[search] == 0xFF && b[search + 1] != 0x00 && b[search + 1] != 0xFF)
                {
                    // Another marker: no restart here; the scan's data has ended.
                    At = search;
                    return;
                }
                search++;
            }
            At = search + 1 < b.Length ? search + 2 : b.Length;
        }

        /// <summary>The place after the scan's data: its next marker other than a restart.</summary>
        public int End()
        {
            var search = Math.Max(At - 2, 0);
            while (search + 1 < b.Length && !(b[search] == 0xFF && b[search + 1] != 0x00 && b[search + 1] is not (>= 0xD0 and <= 0xD7) && b[search + 1] != 0xFF))
            {
                search++;
            }
            return search + 1 < b.Length ? search : b.Length;
        }
    }

    /// <summary>One scan at <paramref name="at"/> (its SOS marker); gives the place after its data.</summary>
    private static int Scan(byte[] b, int at, State state)
    {
        var frame = state.Frame!;
        var length = Be16(b, at + 2);
        if (length < 3 || at + 2 + length > b.Length)
        {
            throw new Broken("a broken scan header");
        }
        var body = b.AsSpan(at + 4, length - 2);
        var count = body[0];
        if (count is < 1 or > 4 || body.Length < 1 + count * 2 + 3)
        {
            throw new Broken("a broken scan header");
        }
        var components = new List<Component>();
        for (var i = 0; i < count; i++)
        {
            var id = body[1 + i * 2];
            var c = frame.Components.FirstOrDefault(c => c.Id == id) ?? throw new Broken($"a scan of component {id}, which the frame does not have");
            c.DcTable = body[2 + i * 2] >> 4;
            c.AcTable = body[2 + i * 2] & 15;
            if (c.DcTable > 3 || c.AcTable > 3)
            {
                throw new Broken("a scan names a table that does not exist");
            }
            components.Add(c);
        }
        var (ss, se, ah, al) = (body[1 + count * 2], body[2 + count * 2], body[3 + count * 2] >> 4, body[3 + count * 2] & 15);
        if (!frame.Progressive)
        {
            (ss, se, ah, al) = (0, 63, 0, 0);
        }
        else if (ss > se || se > 63 || (ss == 0 && se != 0) || (ss > 0 && count != 1) || al > 13)
        {
            throw new Broken("a progressive scan with a broken spectral band");
        }
        foreach (var c in components)
        {
            c.Dc = 0;
            if ((ss == 0 && ah == 0 && state.Dc[c.DcTable] is null) || (se > 0 && (ss > 0 || !frame.Progressive) && state.Ac[c.AcTable] is null))
            {
                throw new Broken("a scan uses a Huffman table that is not defined");
            }
        }
        var bits = new Bits(b, at + 2 + length);
        var eobrun = 0;
        // The order blocks come in: MCU by MCU for several components, else the one component's used blocks row by row.
        var single = components.Count == 1;
        var mcus = single ? components[0].UsedWide * components[0].UsedHigh : frame.McusWide * frame.McusHigh;
        var restart = state.RestartInterval;
        for (var mcu = 0; mcu < mcus; mcu++)
        {
            if (bits.Overrun)
            {
                // The data ended early: the blocks left keep no coefficients (grey), as the Rust engine's reader leaves them.
                break;
            }
            if (restart > 0 && mcu > 0 && mcu % restart == 0)
            {
                bits.Restart();
                foreach (var c in components)
                {
                    c.Dc = 0;
                }
                eobrun = 0;
            }
            if (single)
            {
                var c = components[0];
                var (bx, by) = (mcu % c.UsedWide, mcu / c.UsedWide);
                Block(bits, state, c, (by * c.BlocksWide + bx) * 64, frame.Progressive, ss, se, ah, al, ref eobrun);
                continue;
            }
            var (mx, my) = (mcu % frame.McusWide, mcu / frame.McusWide);
            foreach (var c in components)
            {
                for (var v = 0; v < c.V; v++)
                {
                    for (var h = 0; h < c.H; h++)
                    {
                        var (bx, by) = (mx * c.H + h, my * c.V + v);
                        Block(bits, state, c, (by * c.BlocksWide + bx) * 64, frame.Progressive, ss, se, ah, al, ref eobrun);
                    }
                }
            }
        }
        return bits.End();
    }

    private static void Block(Bits bits, State state, Component c, int offset, bool progressive, int ss, int se, int ah, int al, ref int eobrun)
    {
        var coef = c.Coefficients.AsSpan(offset, 64);
        if (!progressive)
        {
            var t = bits.Decode(state.Dc[c.DcTable]!);
            if (t > 16)
            {
                throw new Broken("a DC difference out of range");
            }
            c.Dc += bits.Extend(t);
            coef[0] = (short)c.Dc;
            var ac = state.Ac[c.AcTable]!;
            for (var k = 1; k < 64;)
            {
                var rs = bits.Decode(ac);
                var (r, s) = (rs >> 4, rs & 15);
                if (s == 0)
                {
                    if (r != 15)
                    {
                        break;
                    }
                    k += 16;
                    continue;
                }
                k += r;
                if (k > 63)
                {
                    break;
                }
                coef[ZigZag[k]] = (short)bits.Extend(s);
                k++;
            }
            return;
        }
        if (ss == 0)
        {
            // DC scans (T.81 G.1.2.1): the first gives the difference, later ones a bit each.
            if (ah == 0)
            {
                var t = bits.Decode(state.Dc[c.DcTable]!);
                if (t > 16)
                {
                    throw new Broken("a DC difference out of range");
                }
                c.Dc += bits.Extend(t);
                coef[0] = (short)(c.Dc << al);
            }
            else if (bits.Bit() == 1)
            {
                coef[0] |= (short)(1 << al);
            }
            return;
        }
        var table = state.Ac[c.AcTable]!;
        if (ah == 0)
        {
            // An AC band's first scan (G.1.2.2), with runs of blocks that end at once.
            if (eobrun > 0)
            {
                eobrun--;
                return;
            }
            for (var k = ss; k <= se;)
            {
                var rs = bits.Decode(table);
                var (r, s) = (rs >> 4, rs & 15);
                if (s == 0)
                {
                    if (r < 15)
                    {
                        eobrun = (1 << r) - 1 + bits.Receive(r);
                        break;
                    }
                    k += 16;
                    continue;
                }
                k += r;
                if (k > 63)
                {
                    break;
                }
                coef[ZigZag[k]] = (short)(bits.Extend(s) * (1 << al));
                k++;
            }
            return;
        }
        // An AC band's refining scan (G.1.2.3): a bit for each coefficient already set, new ones of size 1.
        var p1 = 1 << al;
        var m1 = -1 << al;
        var at = ss;
        if (eobrun <= 0)
        {
            for (; at <= se;)
            {
                var rs = bits.Decode(table);
                var (r, s) = (rs >> 4, rs & 15);
                var value = 0;
                if (s == 0)
                {
                    if (r < 15)
                    {
                        eobrun = (1 << r) + bits.Receive(r);
                        break;
                    }
                }
                else
                {
                    value = bits.Bit() == 1 ? p1 : m1;
                }
                while (at <= se)
                {
                    ref var z = ref coef[ZigZag[at]];
                    if (z != 0)
                    {
                        if (bits.Bit() == 1 && (z & p1) == 0)
                        {
                            z = (short)(z >= 0 ? z + p1 : z + m1);
                        }
                    }
                    else
                    {
                        if (r == 0)
                        {
                            if (value != 0)
                            {
                                z = (short)value;
                            }
                            at++;
                            break;
                        }
                        r--;
                    }
                    at++;
                }
            }
        }
        if (eobrun > 0)
        {
            for (; at <= se; at++)
            {
                ref var z = ref coef[ZigZag[at]];
                if (z != 0 && bits.Bit() == 1 && (z & p1) == 0)
                {
                    z = (short)(z >= 0 ? z + p1 : z + m1);
                }
            }
            eobrun--;
        }
    }

    private static readonly float[] IdctTable = MakeIdctTable();

    /// <summary>cos((2x + 1) u pi / 16) times C(u), by x and u.</summary>
    private static float[] MakeIdctTable()
    {
        var table = new float[64];
        for (var x = 0; x < 8; x++)
        {
            for (var u = 0; u < 8; u++)
            {
                var cu = u == 0 ? 1 / Math.Sqrt(2) : 1;
                table[x * 8 + u] = (float)(cu * Math.Cos((2 * x + 1) * u * Math.PI / 16));
            }
        }
        return table;
    }

    /// <summary>One block's samples (T.81 A.3.3): dequantised, transformed back, level-shifted and clamped.</summary>
    private static void Idct(ReadOnlySpan<short> coef, ushort[] quant, Span<byte> output, int stride)
    {
        Span<float> f = stackalloc float[64];
        Span<float> rows = stackalloc float[64];
        for (var i = 0; i < 64; i++)
        {
            f[i] = coef[i] * quant[i];
        }
        // Along each row (u to x), then down each column (v to y).
        for (var v = 0; v < 8; v++)
        {
            for (var x = 0; x < 8; x++)
            {
                var s = 0f;
                for (var u = 0; u < 8; u++)
                {
                    s += IdctTable[x * 8 + u] * f[v * 8 + u];
                }
                rows[v * 8 + x] = s;
            }
        }
        for (var y = 0; y < 8; y++)
        {
            for (var x = 0; x < 8; x++)
            {
                var s = 0f;
                for (var v = 0; v < 8; v++)
                {
                    s += IdctTable[y * 8 + v] * rows[v * 8 + x];
                }
                var value = MathF.Round(s / 4f + 128f);
                output[y * stride + x] = (byte)Math.Clamp(value, 0f, 255f);
            }
        }
    }

    private static Image Render(Frame frame, State state)
    {
        var (width, height) = (frame.Width, frame.Height);
        // The Rust engine's reader gives no RGBA for Adobe RGB, and scrambled pixels for CMYK: both are refused.
        if (frame.Components.Count == 3 && state.AdobeTransform == 0)
        {
            throw new Broken("\"Unimplemented colorspace mapping from RGB to RGBA\"");
        }
        if (frame.Components.Count == 4)
        {
            throw new Broken("CMYK and YCCK JPEG images are not read yet");
        }
        // Each component's samples at its own resolution.
        var planes = new List<(byte[] Samples, int Stride, int Width, int Height)>();
        foreach (var c in frame.Components)
        {
            var quant = state.Quant[c.Tq] ?? throw new Broken("a component's quantisation table is not defined");
            var stride = c.BlocksWide * 8;
            var samples = new byte[(long)stride * c.BlocksHigh * 8];
            for (var by = 0; by < c.BlocksHigh; by++)
            {
                for (var bx = 0; bx < c.BlocksWide; bx++)
                {
                    Idct(c.Coefficients.AsSpan((by * c.BlocksWide + bx) * 64, 64), quant, samples.AsSpan(by * 8 * stride + bx * 8), stride);
                }
            }
            planes.Add((samples, stride, (width * c.H + frame.HMax - 1) / frame.HMax, (height * c.V + frame.VMax - 1) / frame.VMax));
        }
        // Each brought up to the image's resolution.
        var full = new byte[frame.Components.Count][];
        for (var i = 0; i < frame.Components.Count; i++)
        {
            var c = frame.Components[i];
            full[i] = Upsample(planes[i], frame.HMax / c.H, frame.VMax / c.V, width, height);
        }
        var rgba = new byte[width * height * 4];
        var count = frame.Components.Count;
        for (var p = 0; p < width * height; p++)
        {
            byte r, g, b;
            if (count == 1)
            {
                r = g = b = full[0][p];
            }
            else
            {
                (r, g, b) = FromYCbCr(full[0][p], full[1][p], full[2][p]);
            }
            (rgba[p * 4], rgba[p * 4 + 1], rgba[p * 4 + 2], rgba[p * 4 + 3]) = (r, g, b, 255);
        }
        return new Image(0, width, height, rgba);
    }

    private static (byte R, byte G, byte B) FromYCbCr(int y, int cb, int cr)
    {
        var (u, v) = (cb - 128f, cr - 128f);
        static byte Clamp(float x) => (byte)Math.Clamp(MathF.Round(x), 0f, 255f);
        return (Clamp(y + 1.402f * v), Clamp(y - 0.344136f * u - 0.714136f * v), Clamp(y + 1.772f * u));
    }

    /// <summary>
    /// A component plane at the image's resolution: halved planes by a triangle filter (each output
    /// sample three quarters its nearest input and a quarter the next), other ratios by repeating.
    /// </summary>
    private static byte[] Upsample((byte[] Samples, int Stride, int Width, int Height) plane, int sx, int sy, int width, int height)
    {
        var (src, stride, w, h) = plane;
        int At(int x, int y) => src[Math.Clamp(y, 0, h - 1) * stride + Math.Clamp(x, 0, w - 1)];
        var output = new byte[width * height];
        if (sx == 1 && sy == 1)
        {
            for (var y = 0; y < height; y++)
            {
                for (var x = 0; x < width; x++)
                {
                    output[y * width + x] = (byte)At(x, y);
                }
            }
            return output;
        }
        if (sx is 1 or 2 && sy is 1 or 2)
        {
            for (var y = 0; y < height; y++)
            {
                // The input row and its neighbour on this output row's side.
                var (iy, ny) = sy == 2 ? (y / 2, y % 2 == 0 ? y / 2 - 1 : y / 2 + 1) : (y, y);
                for (var x = 0; x < width; x++)
                {
                    var (ix, nx) = sx == 2 ? (x / 2, x % 2 == 0 ? x / 2 - 1 : x / 2 + 1) : (x, x);
                    int value;
                    if (sx == 2 && sy == 2)
                    {
                        var near = 3 * At(ix, iy) + At(ix, ny);
                        var far = 3 * At(nx, iy) + At(nx, ny);
                        value = (3 * near + far + (x % 2 == 0 ? 8 : 7)) >> 4;
                    }
                    else if (sx == 2)
                    {
                        value = (3 * At(ix, iy) + At(nx, iy) + (x % 2 == 0 ? 1 : 2)) >> 2;
                    }
                    else
                    {
                        value = (3 * At(ix, iy) + At(ix, ny) + (y % 2 == 0 ? 1 : 2)) >> 2;
                    }
                    output[y * width + x] = (byte)value;
                }
            }
            return output;
        }
        for (var y = 0; y < height; y++)
        {
            for (var x = 0; x < width; x++)
            {
                output[y * width + x] = (byte)At(x / sx, y / sy);
            }
        }
        return output;
    }
}

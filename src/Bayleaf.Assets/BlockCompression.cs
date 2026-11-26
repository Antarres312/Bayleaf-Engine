namespace Bayleaf.Assets;

/// <summary>
/// BC1 and BC3 (DXT1, DXT5) blocks, ported from craft-dds (storytold/craft-libs, Copyright (c) 2026 ArtCraft Team and the craft-libs contributors, MIT OR Apache-2.0;
/// see ATTRIBUTION.md), which follows Microsoft's public block compression
/// documentation. The encoder fits a block's colours with two RGB565 endpoints along their
/// principal axis, refines them by least squares, and scores every candidate against the exact
/// palette a reader rebuilds, so the indices it picks are the best ones for what a decoder shows.
/// </summary>
internal static class BlockCompression
{
    /// <summary>The blocks of one level of <paramref name="width"/> x <paramref name="height"/> RGBA pixels, row of blocks by row.</summary>
    public static byte[] Encode(int width, int height, byte[] rgba, bool bc1)
    {
        var unit = bc1 ? 8 : 16;
        var (across, down) = ((width + 3) / 4, (height + 3) / 4);
        var output = new byte[across * down * unit];
        var block = new byte[16][];
        for (var t = 0; t < 16; t++)
        {
            block[t] = new byte[4];
        }
        for (var by = 0; by < down; by++)
        {
            for (var bx = 0; bx < across; bx++)
            {
                var inside = Gather(rgba, width, height, bx, by, block);
                var encoded = bc1 ? EncodeBc1(block, inside) : EncodeBc3(block, inside);
                encoded.CopyTo(output, (by * across + bx) * unit);
            }
        }
        return output;
    }

    /// <summary>The texels of block (bx, by) and the mask of those inside the image; outside ones are zero.</summary>
    private static ushort Gather(byte[] rgba, int width, int height, int bx, int by, byte[][] block)
    {
        ushort inside = 0;
        for (var t = 0; t < 16; t++)
        {
            var (x, y) = (bx * 4 + t % 4, by * 4 + t / 4);
            if (x < width && y < height)
            {
                Array.Copy(rgba, (y * width + x) * 4, block[t], 0, 4);
                inside |= (ushort)(1 << t);
            }
            else
            {
                Array.Clear(block[t]);
            }
        }
        return inside;
    }

    public static byte[] Rgb565(ushort c)
    {
        static byte Scale(int v, int max) => (byte)((v * 255 + max / 2) / max);
        return [Scale(c >> 11, 31), Scale((c >> 5) & 63, 63), Scale(c & 31, 31)];
    }

    /// <summary>The colours a colour block's indices select: four-colour mode adds two colours at thirds; three-colour mode the midpoint and transparent black.</summary>
    public static byte[][] ColourPalette(ushort c0, ushort c1, bool four)
    {
        var (a, b) = (Rgb565(c0), Rgb565(c1));
        byte[] Mix(int wa, int wb)
        {
            var d = wa + wb;
            byte At(int i) => (byte)((wa * a[i] + wb * b[i] + d / 2) / d);
            return [At(0), At(1), At(2), 255];
        }
        return four
            ? [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], Mix(2, 1), Mix(1, 2)]
            : [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], Mix(1, 1), [0, 0, 0, 0]];
    }

    /// <summary>A value block's eight values: six interpolated at sevenths when <c>a0 &gt; a1</c>, else four at fifths followed by 0 and 255.</summary>
    public static byte[] AlphaPalette(byte a0, byte a1)
    {
        var palette = new int[] { a0, a1, 0, 0, 0, 0, 0, 255 };
        if (a0 > a1)
        {
            for (var k = 2; k < 8; k++)
            {
                palette[k] = ((8 - k) * a0 + (k - 1) * a1 + 3) / 7;
            }
        }
        else
        {
            for (var k = 2; k < 6; k++)
            {
                palette[k] = ((6 - k) * a0 + (k - 1) * a1 + 2) / 5;
            }
        }
        return [.. palette.Select(v => (byte)v)];
    }

    /// <summary>A colour block's 16 texels as RGBA, for checking what a reader shows.</summary>
    public static byte[][] DecodeColour(ReadOnlySpan<byte> block, bool oneBitAlpha)
    {
        var (c0, c1) = ((ushort)(block[0] | block[1] << 8), (ushort)(block[2] | block[3] << 8));
        var palette = ColourPalette(c0, c1, c0 > c1 || !oneBitAlpha);
        var indices = (uint)(block[4] | block[5] << 8 | block[6] << 16 | block[7] << 24);
        return [.. Enumerable.Range(0, 16).Select(t => palette[(indices >> (2 * t)) & 3])];
    }

    /// <summary>A BC1 block. Texels under half opacity become BC1's transparent black; an opaque block takes whichever mode fits its colours better.</summary>
    public static byte[] EncodeBc1(byte[][] block, ushort inside)
    {
        var transparent = Select(block, inside, t => t[3] < 128);
        return ColourBlock(block, (ushort)(inside & ~transparent), transparent, true);
    }

    /// <summary>A BC3 block: an interpolated alpha block, then a four-colour block.</summary>
    public static byte[] EncodeBc3(byte[][] block, ushort inside)
    {
        var alphas = new byte[16];
        for (var t = 0; t < 16; t++)
        {
            alphas[t] = block[t][3];
        }
        return [.. ValueBlock(alphas, inside), .. ColourBlock(block, inside, 0, false)];
    }

    private static ushort Select(byte[][] block, ushort inside, Func<byte[], bool> f)
    {
        ushort mask = 0;
        for (var t = 0; t < 16; t++)
        {
            if ((inside >> t & 1) != 0 && f(block[t]))
            {
                mask |= (ushort)(1 << t);
            }
        }
        return mask;
    }

    /// <summary>RGB565 of a colour, rounding each channel to the nearest level.</summary>
    private static ushort To565(float r, float g, float b)
    {
        static int Q(float v, float max) => FMath.ToUInt16(FMath.Round(FMath.Clamp(v, 0f, 255f) * max / 255f));
        return (ushort)(Q(r, 31f) << 11 | Q(g, 63f) << 5 | Q(b, 31f));
    }

    private static uint Distance(byte[] a, byte[] b)
    {
        static uint D(int x) => (uint)(x * x);
        return D(a[0] - b[0]) + D(a[1] - b[1]) + D(a[2] - b[2]);
    }

    /// <summary>The index of the first of the entries nearest by <paramref name="distance"/>, and that distance.</summary>
    private static (int Index, uint Distance) Nearest(int count, Func<int, uint> distance)
    {
        var best = (Index: 0, Distance: uint.MaxValue);
        for (var i = 0; i < count; i++)
        {
            var d = distance(i);
            if (d < best.Distance)
            {
                best = (i, d);
                if (d == 0)
                {
                    break;
                }
            }
        }
        return count == 0 ? (0, 0) : best;
    }

    /// <summary>A colour block (endpoints, then 2-bit indices) for the opaque texels' colours, with the transparent ones on BC1's transparent index.</summary>
    private static byte[] ColourBlock(byte[][] block, ushort opaque, ushort transparent, bool bc1)
    {
        var colours = new List<float[]>(16);
        for (var t = 0; t < 16; t++)
        {
            if ((opaque >> t & 1) != 0)
            {
                colours.Add([block[t][0], block[t][1], block[t][2]]);
            }
        }
        if (colours.Count == 0)
        {
            // Nothing visible: every texel on the transparent index (or index 0 of black).
            return Pack(0, 0, transparent != 0 ? uint.MaxValue : 0);
        }
        var first = colours[0];
        var (lo, hi) = colours.Skip(1).All(c => c.AsSpan().SequenceEqual(first)) ? (first, first) : PrincipalEndpoints(colours);
        var candidate = (To565(lo[0], lo[1], lo[2]), To565(hi[0], hi[1], hi[2]));
        var fitted = new List<(ushort, ushort)> { candidate };
        bool[] modes = transparent != 0 ? [false] : bc1 ? [true, false] : [true];
        (uint Error, byte[] Bytes)? best = null;
        // Each refinement starts from the best block so far: its indices give the least-squares endpoints, one more candidate.
        for (var round = 0; round < 3; round++)
        {
            var (e0, e1) = candidate;
            foreach (var four in modes)
            {
                var (bytes, error) = Fit(block, opaque, transparent, e0, e1, four);
                if (best is null || error < best.Value.Error)
                {
                    best = (error, bytes);
                }
            }
            if (best is not { } b || b.Error == 0 || round == 2)
            {
                break;
            }
            if (LeastSquares(block, opaque, b.Bytes) is not { } next)
            {
                break;
            }
            // A pair fitted before (in either order) fits the same again: the rounds left would change nothing.
            if (fitted.Any(p => p == next || (p.Item2, p.Item1) == next))
            {
                break;
            }
            fitted.Add(next);
            candidate = next;
        }
        return best?.Bytes ?? new byte[8];
    }

    /// <summary>Orders the endpoints for the mode (<c>c0 &gt; c1</c> for four colours) and picks each texel's nearest palette colour; the block and its squared error.</summary>
    private static (byte[] Bytes, uint Error) Fit(byte[][] block, ushort opaque, ushort transparent, ushort e0, ushort e1, bool four)
    {
        var (c0, c1) = four == (e0 >= e1) ? (e0, e1) : (e1, e0);
        var palette = ColourPalette(c0, c1, four && c0 > c1);
        var choices = four && c0 > c1 ? 4 : 3;
        uint indices = 0;
        uint error = 0;
        for (var t = 0; t < 16; t++)
        {
            uint index;
            if ((transparent >> t & 1) != 0)
            {
                index = 3;
            }
            else if ((opaque >> t & 1) != 0)
            {
                var texel = block[t];
                var (i, d) = Nearest(choices, p => Distance(texel, palette[p]));
                error += d;
                index = (uint)i;
            }
            else
            {
                index = 0;
            }
            indices |= index << (2 * t);
        }
        return (Pack(c0, c1, indices), error);
    }

    private static byte[] Pack(ushort c0, ushort c1, uint indices) =>
        [(byte)c0, (byte)(c0 >> 8), (byte)c1, (byte)(c1 >> 8), (byte)indices, (byte)(indices >> 8), (byte)(indices >> 16), (byte)(indices >> 24)];

    /// <summary>The ends of the colours' spread along their principal axis (the covariance's dominant eigenvector, by power iteration), clamped to the RGB cube.</summary>
    private static (float[] Lo, float[] Hi) PrincipalEndpoints(List<float[]> colours)
    {
        var n = (float)Math.Max(colours.Count, 1);
        var mean = new float[3];
        for (var i = 0; i < 3; i++)
        {
            var s = 0f;
            foreach (var c in colours)
            {
                s += c[i];
            }
            mean[i] = s / n;
        }
        var cov = new float[3, 3];
        foreach (var c in colours)
        {
            var d = new[] { c[0] - mean[0], c[1] - mean[1], c[2] - mean[2] };
            for (var i = 0; i < 3; i++)
            {
                for (var j = 0; j < 3; j++)
                {
                    cov[i, j] += d[i] * d[j];
                }
            }
        }
        float[] Times(float[] a) => [cov[0, 0] * a[0] + cov[0, 1] * a[1] + cov[0, 2] * a[2], cov[1, 0] * a[0] + cov[1, 1] * a[1] + cov[1, 2] * a[2],
            cov[2, 0] * a[0] + cov[2, 1] * a[1] + cov[2, 2] * a[2]];
        float[] Iterate(int start)
        {
            var axis = new float[3];
            axis[start] = 1f;
            for (var step = 0; step < 8; step++)
            {
                var next = Times(axis);
                var len = MathF.Sqrt(next[0] * next[0] + next[1] * next[1] + next[2] * next[2]);
                if (!float.IsNormal(len))
                {
                    break;
                }
                var normalised = new[] { next[0] / len, next[1] / len, next[2] / len };
                // A step that returns the same axis, bit for bit, returns it on every later step too.
                if (normalised.Select(BitConverter.SingleToInt32Bits).SequenceEqual(axis.Select(BitConverter.SingleToInt32Bits)))
                {
                    break;
                }
                axis = normalised;
            }
            return axis;
        }
        float Spread(float[] a)
        {
            var t = Times(a);
            return t[0] * a[0] + t[1] * a[1] + t[2] * a[2];
        }
        // From each channel's axis, keeping the one the colours spread along most (the last of equals, as Rust's max_by keeps).
        float[]? axisBest = null;
        for (var start = 0; start < 3; start++)
        {
            var candidate = Iterate(start);
            if (axisBest is null || Spread(candidate).CompareTo(Spread(axisBest)) >= 0)
            {
                axisBest = candidate;
            }
        }
        var axis = axisBest ?? [1f, 0f, 0f];
        float Along(float[] c) => (c[0] - mean[0]) * axis[0] + (c[1] - mean[1]) * axis[1] + (c[2] - mean[2]) * axis[2];
        var (lo, hi) = (float.MaxValue, float.MinValue);
        foreach (var c in colours)
        {
            var t = Along(c);
            (lo, hi) = (FMath.Min(lo, t), FMath.Max(hi, t));
        }
        float[] At(float t) => [FMath.Clamp(mean[0] + t * axis[0], 0f, 255f), FMath.Clamp(mean[1] + t * axis[1], 0f, 255f), FMath.Clamp(mean[2] + t * axis[2], 0f, 255f)];
        return lo > hi ? (mean, mean) : (At(lo), At(hi));
    }

    /// <summary>The endpoints that minimise the squared error for the indices the block already chose; null when the indices do not constrain both.</summary>
    private static (ushort, ushort)? LeastSquares(byte[][] block, ushort opaque, byte[] bytes)
    {
        var (c0, c1) = ((ushort)(bytes[0] | bytes[1] << 8), (ushort)(bytes[2] | bytes[3] << 8));
        var indices = (uint)(bytes[4] | bytes[5] << 8 | bytes[6] << 16 | bytes[7] << 24);
        var four = c0 > c1;
        float? Weight(uint i) => (four, i) switch
        {
            (_, 0) => 1f,
            (_, 1) => 0f,
            (true, 2) => 2f / 3f,
            (true, _) => 1f / 3f,
            (false, 2) => 0.5f,
            (false, _) => null,
        };
        float aa = 0, ab = 0, bb = 0;
        var ax = new float[3];
        var bx = new float[3];
        for (var t = 0; t < 16; t++)
        {
            if ((opaque >> t & 1) == 0 || Weight(indices >> (2 * t) & 3) is not { } a)
            {
                continue;
            }
            var b = 1f - a;
            aa += a * a;
            ab += a * b;
            bb += b * b;
            for (var i = 0; i < 3; i++)
            {
                float v = block[t][i];
                ax[i] += a * v;
                bx[i] += b * v;
            }
        }
        var det = aa * bb - ab * ab;
        if (MathF.Abs(det) < 1e-3f)
        {
            return null;
        }
        float E0(int i) => (ax[i] * bb - bx[i] * ab) / det;
        float E1(int i) => (bx[i] * aa - ax[i] * ab) / det;
        return (To565(E0(0), E0(1), E0(2)), To565(E1(0), E1(1), E1(2)));
    }

    /// <summary>A block of 16 single-channel values (BC3's alpha): whichever of the two modes fits better. Only the values inside count.</summary>
    public static byte[] ValueBlock(byte[] values, ushort inside)
    {
        (byte Lo, byte Hi)? all = null, mid = null;
        static (byte, byte) Widen((byte Lo, byte Hi)? r, byte v) => r is { } x ? (Math.Min(x.Lo, v), Math.Max(x.Hi, v)) : (v, v);
        for (var t = 0; t < 16; t++)
        {
            if ((inside >> t & 1) != 0)
            {
                all = Widen(all, values[t]);
                if (values[t] is not (0 or 255))
                {
                    mid = Widen(mid, values[t]);
                }
            }
        }
        var (lo, hi) = all ?? ((byte)0, (byte)0);
        // Eight values need a0 > a1; equal ends fall to the six-value mode, which starts with a0 too.
        var eight = ValueFit(values, inside, hi, lo);
        var (midLo, midHi) = mid ?? ((byte)0, (byte)0);
        var six = ValueFit(values, inside, midLo, midHi);
        return six.Error < eight.Error ? six.Bytes : eight.Bytes;
    }

    private static (byte[] Bytes, uint Error) ValueFit(byte[] values, ushort inside, byte a0, byte a1)
    {
        var palette = AlphaPalette(a0, a1);
        ulong bits = 0;
        uint error = 0;
        for (var t = 0; t < 16; t++)
        {
            if ((inside >> t & 1) == 0)
            {
                continue;
            }
            var v = values[t];
            var (i, d) = Nearest(8, p => (uint)((palette[p] - v) * (palette[p] - v)));
            error += d;
            bits |= (ulong)i << (3 * t);
        }
        return ([a0, a1, (byte)bits, (byte)(bits >> 8), (byte)(bits >> 16), (byte)(bits >> 24), (byte)(bits >> 32), (byte)(bits >> 40)], error);
    }
}

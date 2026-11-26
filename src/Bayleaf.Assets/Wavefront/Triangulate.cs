namespace Bayleaf.Assets.Wavefront;

/// <summary>
/// Triangulating one polygon face: a fan for a convex face, ear clipping for a concave one (ported
/// from craft-mesh, storytold/craft-libs, Copyright (c) 2026 ArtCraft Team and the craft-libs contributors, MIT OR Apache-2.0). The face is projected onto the plane
/// of its two minor axes and clipped there. Work is charged to a budget the caller shares between
/// the faces of a part; when it runs out, or no ear can be found, what is left of the face is
/// fanned. A face always gives corners - 2 triangles, wound as the face is.
/// </summary>
internal static class Triangulate
{
    /// <summary>Steps allowed per corner of a part, shared by its faces.</summary>
    public const int WorkPerCorner = 256;

    /// <summary>Relative tolerance under which a turn counts as straight.</summary>
    private const double Straight = 1e-12;

    public static List<Corner[]> Face(ArraySegment<Corner> face, IReadOnlyList<double[]> positions, ref long budget)
    {
        if (face.Count <= 3 || Project(face, positions) is not { } pts)
        {
            return Fan(face);
        }
        var area = 0.0;
        for (var i = 0; i < pts.Length; i++)
        {
            area += Cross(pts[i], pts[(i + 1) % pts.Length]);
        }
        if (!double.IsFinite(area) || area == 0)
        {
            return Fan(face);
        }
        var ring = new Ring(pts, Math.Sign(area));
        return ring.IsConvex() ? Fan(face) : ring.Clip(face, ref budget);
    }

    private static List<Corner[]> Fan(ArraySegment<Corner> face)
    {
        var output = new List<Corner[]>();
        for (var next = 1; next + 1 < face.Count; next++)
        {
            output.Add([face[0], face[next], face[next + 1]]);
        }
        return output;
    }

    /// <summary>The face's corners in 2D, on the plane of the two axes its Newell normal leans on least; null when a corner has no position or the normal is not finite.</summary>
    private static double[][]? Project(ArraySegment<Corner> face, IReadOnlyList<double[]> positions)
    {
        var p = new double[face.Count][];
        for (var i = 0; i < face.Count; i++)
        {
            if (face[i].Position >= positions.Count)
            {
                return null;
            }
            p[i] = positions[(int)face[i].Position];
        }
        var n = new double[3];
        for (var i = 0; i < p.Length; i++)
        {
            var (a, b) = (p[i], p[(i + 1) % p.Length]);
            n[0] += (a[1] - b[1]) * (a[2] + b[2]);
            n[1] += (a[2] - b[2]) * (a[0] + b[0]);
            n[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        n = [Math.Abs(n[0]), Math.Abs(n[1]), Math.Abs(n[2])];
        if (!n.All(double.IsFinite))
        {
            return null;
        }
        var (u, v) = n[0] >= n[1] && n[0] >= n[2] ? (1, 2) : n[1] >= n[2] ? (2, 0) : (0, 1);
        return [.. p.Select(q => new[] { q[u], q[v] })];
    }

    /// <summary>The z component of a x b.</summary>
    private static double Cross(double[] a, double[] b) => a[0] * b[1] - a[1] * b[0];

    /// <summary>Twice the signed area of triangle a b c.</summary>
    private static double Orient(double[] a, double[] b, double[] c) => (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);

    private static bool Same(double[] a, double[] b) => a[0] == b[0] && a[1] == b[1];

    /// <summary>A face's projected corners as a doubly linked ring that ears are clipped from.</summary>
    private sealed class Ring
    {
        private readonly double[][] pts;
        private readonly double sign;
        private readonly int[] prev;
        private readonly int[] next;

        public Ring(double[][] pts, double sign)
        {
            this.pts = pts;
            this.sign = sign;
            var n = pts.Length;
            prev = [.. Enumerable.Range(0, n).Select(i => (i + n - 1) % n)];
            next = [.. Enumerable.Range(0, n).Select(i => (i + 1) % n)];
        }

        /// <summary>The turn at corner i, positive when convex; and the tolerance under which it is straight.</summary>
        private (double Turn, double Tolerance) Turn(int i)
        {
            var (a, b, c) = (pts[prev[i]], pts[i], pts[next[i]]);
            double[] e = [b[0] - a[0], b[1] - a[1]], f = [c[0] - b[0], c[1] - b[1]];
            return (Cross(e, f) * sign, Straight * double.Hypot(e[0], e[1]) * double.Hypot(f[0], f[1]));
        }

        /// <summary>No corner turns the other way.</summary>
        public bool IsConvex()
        {
            for (var i = 0; i < pts.Length; i++)
            {
                var (t, tol) = Turn(i);
                if (!(t >= -tol))
                {
                    return false;
                }
            }
            return true;
        }

        /// <summary>Whether the triangle at corner i is an ear; null when the budget runs out.</summary>
        private bool? IsEar(int i, List<int> reflex, ref long budget)
        {
            if (--budget < 0)
            {
                return null;
            }
            var (t, tol) = Turn(i);
            if (t <= tol)
            {
                return false;
            }
            var (a, b, c) = (pts[prev[i]], pts[i], pts[next[i]]);
            foreach (var r in reflex)
            {
                if (--budget < 0)
                {
                    return null;
                }
                var p = pts[r];
                var (turn, tolerance) = Turn(r);
                if (Same(p, a) || Same(p, b) || Same(p, c) || turn > tolerance)
                {
                    continue;
                }
                if (LeftOf(a, b, p) && LeftOf(b, c, p) && LeftOf(c, a, p))
                {
                    return false;
                }
            }
            return true;
        }

        /// <summary>Whether p lies on the inner side of edge a b, or on it within rounding.</summary>
        private bool LeftOf(double[] a, double[] b, double[] p)
        {
            static double L1(double[] u, double[] v) => Math.Abs(v[0] - u[0]) + Math.Abs(v[1] - u[1]);
            return Orient(a, b, p) * sign >= -Straight * L1(a, b) * L1(a, p);
        }

        /// <summary>The first straight corner of the <paramref name="left"/> corners from <paramref name="from"/> on.</summary>
        private int? StraightCorner(int from, int left, ref long budget)
        {
            var j = from;
            for (var k = 0; k < left; k++)
            {
                if (--budget < 0)
                {
                    return null;
                }
                var (t, tol) = Turn(j);
                if (Math.Abs(t) <= tol)
                {
                    return j;
                }
                j = next[j];
            }
            return null;
        }

        private Corner[] Cut(int i, ArraySegment<Corner> face)
        {
            var (a, c) = (prev[i], next[i]);
            next[a] = c;
            prev[c] = a;
            return [face[a], face[i], face[c]];
        }

        /// <summary>Clips ears until a triangle is left; a straight corner when a lap finds no ear; else the rest is fanned.</summary>
        public List<Corner[]> Clip(ArraySegment<Corner> face, ref long budget)
        {
            var output = new List<Corner[]>(face.Count - 2);
            var removed = new bool[face.Count];
            var reflex = Enumerable.Range(0, face.Count).Where(i => Turn(i).Turn <= Turn(i).Tolerance).ToList();
            var (left, i, stop) = (face.Count, 0, 0);
            while (left > 3)
            {
                bool ear;
                switch (IsEar(i, reflex, ref budget))
                {
                    case true:
                        ear = true;
                        break;
                    case false:
                        i = next[i];
                        if (i != stop)
                        {
                            continue;
                        }
                        if (StraightCorner(i, left, ref budget) is { } j)
                        {
                            i = j;
                            ear = true;
                        }
                        else
                        {
                            ear = false;
                        }
                        break;
                    default:
                        ear = false;
                        break;
                }
                if (!ear)
                {
                    break;
                }
                var c = next[i];
                output.Add(Cut(i, face));
                removed[i] = true;
                reflex.RemoveAll(r => removed[r]);
                left--;
                (i, stop) = (c, c);
            }
            // Fan whatever is left from corner i.
            var first = i;
            var b = next[first];
            while (next[b] != first)
            {
                output.Add([face[first], face[b], face[next[b]]]);
                b = next[b];
            }
            return output;
        }
    }
}

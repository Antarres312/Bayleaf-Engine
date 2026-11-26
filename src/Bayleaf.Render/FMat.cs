using System.Numerics;

using Bayleaf.Core.Geometry;

namespace Bayleaf.Render;

/// <summary>
/// Single-precision matrix helpers with the Rust engine's results. Matrices are System.Numerics ones:
/// a point is <c>Vector3.Transform(p, m)</c> and in <c>a * b</c> the transform <c>a</c> applies first,
/// so the Rust engine's <c>a * b</c> is <c>b * a</c> here; their numbers are laid out the same, so they
/// go to the GPU as they are.
/// </summary>
internal static class FMat
{
    /// <summary>A right-handed perspective projection to depth 0 (near) to 1 (far).</summary>
    public static Matrix4x4 PerspectiveRh(float fovY, float aspect, float near, float far)
    {
        var (sin, cos) = MathF.SinCos(0.5f * fovY);
        var h = cos / sin;
        var w = h / aspect;
        var r = far / (near - far);
        return new(w, 0, 0, 0, 0, h, 0, 0, 0, 0, r, -1, 0, 0, r * near, 0);
    }

    /// <summary>A right-handed orthographic projection to depth 0 (near) to 1 (far).</summary>
    public static Matrix4x4 OrthographicRh(float left, float right, float bottom, float top, float near, float far)
    {
        var rcpWidth = 1f / (right - left);
        var rcpHeight = 1f / (top - bottom);
        var r = 1f / (near - far);
        return new(rcpWidth + rcpWidth, 0, 0, 0, 0, rcpHeight + rcpHeight, 0, 0, 0, 0, r, 0, -(left + right) * rcpWidth, -(top + bottom) * rcpHeight, r * near, 1);
    }

    /// <summary>A right-handed view from <paramref name="eye"/> towards <paramref name="center"/>.</summary>
    public static Matrix4x4 LookAtRh(Vector3 eye, Vector3 center, Vector3 up)
    {
        var f = Normalize(center - eye);
        var s = Normalize(Vector3.Cross(f, up));
        var u = Vector3.Cross(s, f);
        return new(s.X, u.X, -f.X, 0, s.Y, u.Y, -f.Y, 0, s.Z, u.Z, -f.Z, 0, -Vector3.Dot(eye, s), -Vector3.Dot(eye, u), Vector3.Dot(eye, f), 1);
    }

    /// <summary>The vector times the reciprocal of its length (no check, as glam's <c>normalize</c>).</summary>
    public static Vector3 Normalize(Vector3 v) => v * (1f / v.Length());

    /// <summary>The vector at length 1, or <paramref name="fallback"/> when it has no finite length.</summary>
    public static Vector3 NormalizeOr(Vector3 v, Vector3 fallback)
    {
        var recip = 1f / v.Length();
        return float.IsFinite(recip) && recip > 0 ? v * recip : fallback;
    }

    public static Vector3 NormalizeOrZero(Vector3 v) => NormalizeOr(v, Vector3.Zero);

    /// <summary>The inverse; not finite when there is none.</summary>
    public static Matrix4x4 Inverse(Matrix4x4 m) => Matrix4x4.Invert(m, out var inverse) ? inverse : NaN;

    private static readonly Matrix4x4 NaN = new(float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN,
        float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN);

    public static bool IsFinite(Matrix4x4 m) =>
        float.IsFinite(m.M11) && float.IsFinite(m.M12) && float.IsFinite(m.M13) && float.IsFinite(m.M14)
        && float.IsFinite(m.M21) && float.IsFinite(m.M22) && float.IsFinite(m.M23) && float.IsFinite(m.M24)
        && float.IsFinite(m.M31) && float.IsFinite(m.M32) && float.IsFinite(m.M33) && float.IsFinite(m.M34)
        && float.IsFinite(m.M41) && float.IsFinite(m.M42) && float.IsFinite(m.M43) && float.IsFinite(m.M44);

    /// <summary>A point moved by the matrix, then divided by its w.</summary>
    public static Vector3 ProjectPoint(Matrix4x4 m, Vector3 p)
    {
        var v = Vector4.Transform(new Vector4(p, 1), m);
        return new Vector3(v.X, v.Y, v.Z) / v.W;
    }

    /// <summary>A point moved by an affine matrix.</summary>
    public static Vector3 TransformPoint(Matrix4x4 m, Vector3 p) => Vector3.Transform(p, m);

    /// <summary>A direction turned and scaled by the matrix, not moved.</summary>
    public static Vector3 TransformVector(Matrix4x4 m, Vector3 v) => Vector3.TransformNormal(v, m);

    /// <summary>A double-precision matrix (columns, <c>M · p</c>) in single precision.</summary>
    public static Matrix4x4 FromDouble(DMat4 m) => new(
        (float)m.XAxis.X, (float)m.XAxis.Y, (float)m.XAxis.Z, (float)m.XAxis.W,
        (float)m.YAxis.X, (float)m.YAxis.Y, (float)m.YAxis.Z, (float)m.YAxis.W,
        (float)m.ZAxis.X, (float)m.ZAxis.Y, (float)m.ZAxis.Z, (float)m.ZAxis.W,
        (float)m.WAxis.X, (float)m.WAxis.Y, (float)m.WAxis.Z, (float)m.WAxis.W);

    /// <summary>A single-precision matrix in double precision.</summary>
    public static DMat4 ToDouble(Matrix4x4 m) => new(new(m.M11, m.M12, m.M13, m.M14), new(m.M21, m.M22, m.M23, m.M24), new(m.M31, m.M32, m.M33, m.M34), new(m.M41, m.M42, m.M43, m.M44));

    /// <summary>The upper 3 x 3 of <paramref name="model"/>, inverted and transposed, for normals (itself when it has no inverse).</summary>
    public static Matrix4x4 NormalMatrix(Matrix4x4 model)
    {
        var m = new Matrix4x4(model.M11, model.M12, model.M13, 0, model.M21, model.M22, model.M23, 0, model.M31, model.M32, model.M33, 0, 0, 0, 0, 1);
        var det = m.M11 * (m.M22 * m.M33 - m.M23 * m.M32) - m.M12 * (m.M21 * m.M33 - m.M23 * m.M31) + m.M13 * (m.M21 * m.M32 - m.M22 * m.M31);
        return MathF.Abs(det) < 1e-12f ? m : Matrix4x4.Transpose(Inverse(m));
    }

    /// <summary>Its 16 numbers as glam keeps them (column by column), which is how System.Numerics keeps them too.</summary>
    public static void Write(Span<float> into, Matrix4x4 m)
    {
        into[0] = m.M11; into[1] = m.M12; into[2] = m.M13; into[3] = m.M14;
        into[4] = m.M21; into[5] = m.M22; into[6] = m.M23; into[7] = m.M24;
        into[8] = m.M31; into[9] = m.M32; into[10] = m.M33; into[11] = m.M34;
        into[12] = m.M41; into[13] = m.M42; into[14] = m.M43; into[15] = m.M44;
    }

    /// <summary>The order of <c>f32::total_cmp</c>: -NaN, -inf … -0, +0 … inf, NaN.</summary>
    public static int TotalOrder(float a, float b)
    {
        static int Key(float v)
        {
            var bits = BitConverter.SingleToInt32Bits(v);
            return bits ^ (int)((uint)(bits >> 31) >> 1);
        }
        return Key(a).CompareTo(Key(b));
    }
}

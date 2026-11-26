namespace Bayleaf.Core.Geometry;

/// <summary>A rotation as a unit quaternion, in double precision.</summary>
public readonly record struct DQuat(double X, double Y, double Z, double W)
{
    public static readonly DQuat Identity = new(0, 0, 0, 1);

    public static DQuat operator *(DQuat a, DQuat b) => new(
        a.W * b.X + a.X * b.W + a.Y * b.Z - a.Z * b.Y,
        a.W * b.Y - a.X * b.Z + a.Y * b.W + a.Z * b.X,
        a.W * b.Z + a.X * b.Y - a.Y * b.X + a.Z * b.W,
        a.W * b.W - a.X * b.X - a.Y * b.Y - a.Z * b.Z);

    public static DQuat RotationX(double angle) => new(Math.Sin(angle / 2), 0, 0, Math.Cos(angle / 2));

    public static DQuat RotationY(double angle) => new(0, Math.Sin(angle / 2), 0, Math.Cos(angle / 2));

    public static DQuat RotationZ(double angle) => new(0, 0, Math.Sin(angle / 2), Math.Cos(angle / 2));

    /// <summary>
    /// The rotation of Euler angles (radians) in the engine's order: about X, then the turned Y,
    /// then the twice-turned Z (as a matrix, Rx · Ry · Rz).
    /// </summary>
    public static DQuat FromEulerXyz(double x, double y, double z) => RotationX(x) * RotationY(y) * RotationZ(z);

    /// <summary>The rotation matrix's entry at <paramref name="row"/>, <paramref name="column"/>.</summary>
    public double Entry(int row, int column) => (row, column) switch
    {
        (0, 0) => 1 - 2 * (Y * Y + Z * Z),
        (0, 1) => 2 * (X * Y - W * Z),
        (0, 2) => 2 * (X * Z + W * Y),
        (1, 0) => 2 * (X * Y + W * Z),
        (1, 1) => 1 - 2 * (X * X + Z * Z),
        (1, 2) => 2 * (Y * Z - W * X),
        (2, 0) => 2 * (X * Z - W * Y),
        (2, 1) => 2 * (Y * Z + W * X),
        _ => 1 - 2 * (X * X + Y * Y),
    };

    /// <summary>The Euler angles (radians) of <see cref="FromEulerXyz"/> that give this rotation.</summary>
    public (double X, double Y, double Z) ToEulerXyz()
    {
        var sinY = Math.Clamp(Entry(0, 2), -1.0, 1.0);
        var y = Math.Asin(sinY);
        if (Math.Abs(sinY) < 1 - 1e-12)
        {
            return (Math.Atan2(-Entry(1, 2), Entry(2, 2)), y, Math.Atan2(-Entry(0, 1), Entry(0, 0)));
        }
        // Looking straight along Y: X and Z turn about the same axis, so Z is taken as 0.
        return (Math.Atan2(Entry(2, 1), Entry(1, 1)), y, 0);
    }

    /// <summary>The rotation of an orthonormal matrix given by its entries <c>m[row, column]</c>.</summary>
    public static DQuat FromRotationMatrix(Func<int, int, double> m)
    {
        var trace = m(0, 0) + m(1, 1) + m(2, 2);
        if (trace > 0)
        {
            var s = Math.Sqrt(trace + 1) * 2;
            return Normalize(new((m(2, 1) - m(1, 2)) / s, (m(0, 2) - m(2, 0)) / s, (m(1, 0) - m(0, 1)) / s, s / 4));
        }
        if (m(0, 0) > m(1, 1) && m(0, 0) > m(2, 2))
        {
            var s = Math.Sqrt(1 + m(0, 0) - m(1, 1) - m(2, 2)) * 2;
            return Normalize(new(s / 4, (m(0, 1) + m(1, 0)) / s, (m(0, 2) + m(2, 0)) / s, (m(2, 1) - m(1, 2)) / s));
        }
        if (m(1, 1) > m(2, 2))
        {
            var s = Math.Sqrt(1 + m(1, 1) - m(0, 0) - m(2, 2)) * 2;
            return Normalize(new((m(0, 1) + m(1, 0)) / s, s / 4, (m(1, 2) + m(2, 1)) / s, (m(0, 2) - m(2, 0)) / s));
        }
        var t = Math.Sqrt(1 + m(2, 2) - m(0, 0) - m(1, 1)) * 2;
        return Normalize(new((m(0, 2) + m(2, 0)) / t, (m(1, 2) + m(2, 1)) / t, t / 4, (m(1, 0) - m(0, 1)) / t));
    }

    private static DQuat Normalize(DQuat q)
    {
        var length = Math.Sqrt(q.X * q.X + q.Y * q.Y + q.Z * q.Z + q.W * q.W);
        return length > 0 ? new(q.X / length, q.Y / length, q.Z / length, q.W / length) : Identity;
    }
}

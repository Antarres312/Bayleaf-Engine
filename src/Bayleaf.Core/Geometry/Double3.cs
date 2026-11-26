namespace Bayleaf.Core.Geometry;

/// <summary>Three doubles: positions, which keep millimetre accuracy far from the origin.</summary>
public readonly record struct Double3(double X, double Y, double Z)
{
    public static readonly Double3 Zero = new(0, 0, 0);
    public static readonly Double3 One = new(1, 1, 1);

    public double Length => Math.Sqrt(Dot(this));

    public double Dot(Double3 other) => X * other.X + Y * other.Y + Z * other.Z;

    public Double3 Cross(Double3 o) => new(Y * o.Z - Z * o.Y, Z * o.X - X * o.Z, X * o.Y - Y * o.X);

    public static Double3 operator +(Double3 a, Double3 b) => new(a.X + b.X, a.Y + b.Y, a.Z + b.Z);

    public static Double3 operator -(Double3 a, Double3 b) => new(a.X - b.X, a.Y - b.Y, a.Z - b.Z);

    public static Double3 operator *(Double3 a, double s) => new(a.X * s, a.Y * s, a.Z * s);

    public static Double3 operator *(double s, Double3 a) => a * s;

    public bool IsFinite => double.IsFinite(X) && double.IsFinite(Y) && double.IsFinite(Z);

    public double[] ToArray() => [X, Y, Z];

    public static Double3 FromArray(IReadOnlyList<double> values) => new(values[0], values[1], values[2]);

    public Double3 Map(Func<double, double> f) => new(f(X), f(Y), f(Z));
}

/// <summary>Four doubles: a column of a <see cref="DMat4"/>.</summary>
public readonly record struct Double4(double X, double Y, double Z, double W)
{
    public Double3 Xyz => new(X, Y, Z);

    public static Double4 operator *(Double4 a, double s) => new(a.X * s, a.Y * s, a.Z * s, a.W * s);

    public static Double4 operator +(Double4 a, Double4 b) => new(a.X + b.X, a.Y + b.Y, a.Z + b.Z, a.W + b.W);

    public double this[int i] => i switch
    {
        0 => X,
        1 => Y,
        2 => Z,
        _ => W,
    };
}

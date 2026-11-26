using System.Diagnostics.CodeAnalysis;
using System.Globalization;
using System.Numerics;

namespace Bayleaf.Script.Sdk;

/// <summary>
/// Three doubles: a position, which keeps millimetre accuracy far from the world's centre (beyond
/// about 10 km single precision does not). Each part is a reference into the value, so
/// <c>pose.Position.Y += 1</c> changes it in place.
/// </summary>
public struct Vector3d : IEquatable<Vector3d>
{
    private double x;
    private double y;
    private double z;

    public Vector3d(double x, double y, double z) => (this.x, this.y, this.z) = (x, y, z);

    [UnscopedRef]
    public ref double X => ref x;

    [UnscopedRef]
    public ref double Y => ref y;

    [UnscopedRef]
    public ref double Z => ref z;

    public static Vector3d Zero => default;

    public readonly double Length => Math.Sqrt((x * x) + (y * y) + (z * z));

    public readonly double Dot(Vector3d other) => (x * other.x) + (y * other.y) + (z * other.z);

    public readonly bool IsFinite => double.IsFinite(x) && double.IsFinite(y) && double.IsFinite(z);

    public static Vector3d operator +(Vector3d a, Vector3d b) => new(a.x + b.x, a.y + b.y, a.z + b.z);

    public static Vector3d operator -(Vector3d a, Vector3d b) => new(a.x - b.x, a.y - b.y, a.z - b.z);

    public static Vector3d operator -(Vector3d a) => new(-a.x, -a.y, -a.z);

    public static Vector3d operator *(Vector3d a, double s) => new(a.x * s, a.y * s, a.z * s);

    public static Vector3d operator *(double s, Vector3d a) => a * s;

    public static Vector3d operator /(Vector3d a, double s) => new(a.x / s, a.y / s, a.z / s);

    public static bool operator ==(Vector3d a, Vector3d b) => a.Equals(b);

    public static bool operator !=(Vector3d a, Vector3d b) => !a.Equals(b);

    public static implicit operator Vector3d(Vector3 v) => new(v.X, v.Y, v.Z);

    public static explicit operator Vector3(Vector3d v) => new((float)v.x, (float)v.y, (float)v.z);

    public static Vector3d Add(Vector3d a, Vector3d b) => a + b;

    public static Vector3d Subtract(Vector3d a, Vector3d b) => a - b;

    public static Vector3d Negate(Vector3d a) => -a;

    public static Vector3d Multiply(Vector3d a, double s) => a * s;

    public static Vector3d Divide(Vector3d a, double s) => a / s;

    public static Vector3d FromVector3(Vector3 v) => v;

    public readonly Vector3 ToVector3() => new((float)x, (float)y, (float)z);

    public readonly bool Equals(Vector3d other) => x.Equals(other.x) && y.Equals(other.y) && z.Equals(other.z);

    public override readonly bool Equals(object? obj) => obj is Vector3d other && Equals(other);

    public override readonly int GetHashCode() => HashCode.Combine(x, y, z);

    public override readonly string ToString() => string.Create(CultureInfo.InvariantCulture, $"({x}, {y}, {z})");
}

/// <summary>
/// Where an object is, relative to its parent: its position, its turn (degrees about X, then Y, then Z)
/// and its scale. Each part is a reference into the pose, so a pose in <see cref="Many{T}.Poses"/>
/// changes in place (<c>poses[i].Rotation.Y += 90 * seconds</c>); <see cref="GameScript.Pose"/> gives a
/// copy, to change and give back.
/// </summary>
public struct Pose : IEquatable<Pose>
{
    private Vector3d position;
    private Vector3 rotation;
    private Vector3 scale;

    public Pose(Vector3d position, Vector3 rotation, Vector3 scale) => (this.position, this.rotation, this.scale) = (position, rotation, scale);

    [UnscopedRef]
    public ref Vector3d Position => ref position;

    /// <summary>Degrees about X, then Y, then Z.</summary>
    [UnscopedRef]
    public ref Vector3 Rotation => ref rotation;

    [UnscopedRef]
    public ref Vector3 Scale => ref scale;

    /// <summary>At the centre, not turned, at scale 1.</summary>
    public static Pose Initial => new(default, Vector3.Zero, Vector3.One);

    /// <summary>Whether every number is finite; the engine refuses a pose that is not.</summary>
    public readonly bool IsFinite => position.IsFinite && float.IsFinite(rotation.X) && float.IsFinite(rotation.Y) && float.IsFinite(rotation.Z)
        && float.IsFinite(scale.X) && float.IsFinite(scale.Y) && float.IsFinite(scale.Z);

    public static bool operator ==(Pose a, Pose b) => a.Equals(b);

    public static bool operator !=(Pose a, Pose b) => !a.Equals(b);

    public readonly bool Equals(Pose other) => position.Equals(other.position) && rotation.Equals(other.rotation) && scale.Equals(other.scale);

    public override readonly bool Equals(object? obj) => obj is Pose other && Equals(other);

    public override readonly int GetHashCode() => HashCode.Combine(position, rotation, scale);
}

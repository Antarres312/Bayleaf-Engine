using System.Numerics;
using System.Runtime.InteropServices;

using Bayleaf.Scene;
using Bayleaf.Scene.Components;

namespace Bayleaf.Render;

/// <summary>
/// A vertex as the GPU reads it: position, normal, texture coordinates (0 to 1 across a face, (0, 0)
/// at the top left of an image), and for a skinned model the four bindings it follows (indices into
/// its skin's matrices) and their weights. 56 bytes, laid out as the Rust renderer lays them out.
/// </summary>
[StructLayout(LayoutKind.Sequential, Pack = 4)]
internal struct Vertex
{
    public Vector3 Position;
    public Vector3 Normal;
    public Vector2 Uv;
    public ushort J0;
    public ushort J1;
    public ushort J2;
    public ushort J3;
    public Vector4 Weights;

    /// <summary>A vertex that follows no joints.</summary>
    public static Vertex Plain(Vector3 position, Vector3 normal, Vector2 uv) => new() { Position = position, Normal = normal, Uv = uv };
}

/// <summary>Built-in shapes, generated in code: unit size, centred on the origin.</summary>
internal static class Shapes
{
    public static (Vertex[] Vertices, uint[] Indices) Build(Shape shape) => shape switch
    {
        Shape.Cube => Cube(),
        Shape.Sphere => Sphere(32, 16),
        Shape.Plane => Square(Vector3.UnitY, (u, v) => new Vector3(u, 0, -v)),
        _ => Square(Vector3.UnitZ, (u, v) => new Vector3(u, v, 0)),
    };

    private static readonly (float U, float V)[] Corners = [(-0.5f, -0.5f), (0.5f, -0.5f), (0.5f, 0.5f), (-0.5f, 0.5f)];

    /// <summary>One square face of side 1 facing <paramref name="normal"/>; <paramref name="place"/> maps (u, v) in -0.5 to 0.5 to a position.</summary>
    private static (Vertex[], uint[]) Square(Vector3 normal, Func<float, float, Vector3> place) =>
        ([.. Corners.Select(c => Vertex.Plain(place(c.U, c.V), normal, new Vector2(c.U + 0.5f, 0.5f - c.V)))], [0, 1, 2, 0, 2, 3]);

    private static (Vertex[], uint[]) Cube()
    {
        // (normal, tangent u, tangent v) per face; u × v = normal keeps the winding counter-clockwise.
        (Vector3 N, Vector3 U, Vector3 V)[] faces =
        [
            (Vector3.UnitX, -Vector3.UnitZ, Vector3.UnitY),
            (-Vector3.UnitX, Vector3.UnitZ, Vector3.UnitY),
            (Vector3.UnitY, Vector3.UnitX, -Vector3.UnitZ),
            (-Vector3.UnitY, Vector3.UnitX, Vector3.UnitZ),
            (Vector3.UnitZ, Vector3.UnitX, Vector3.UnitY),
            (-Vector3.UnitZ, -Vector3.UnitX, Vector3.UnitY),
        ];
        var vertices = new List<Vertex>(24);
        var indices = new List<uint>(36);
        foreach (var (n, u, v) in faces)
        {
            var first = (uint)vertices.Count;
            foreach (var (su, sv) in Corners)
            {
                vertices.Add(Vertex.Plain(n * 0.5f + u * su + v * sv, n, new Vector2(su + 0.5f, 0.5f - sv)));
            }
            indices.AddRange([first, first + 1, first + 2, first, first + 2, first + 3]);
        }
        return ([.. vertices], [.. indices]);
    }

    private static (Vertex[], uint[]) Sphere(uint segments, uint rings)
    {
        var vertices = new List<Vertex>();
        for (var ring = 0u; ring <= rings; ring++)
        {
            var theta = MathF.PI * ring / rings;
            // Exact poles: sin(π) in single precision is not 0, which would leave sliver triangles there.
            var (sinT, cosT) = ring == 0 ? (0f, 1f) : ring == rings ? (0f, -1f) : MathF.SinCos(theta);
            for (var segment = 0u; segment <= segments; segment++)
            {
                var phi = MathF.Tau * segment / segments;
                var normal = new Vector3(sinT * MathF.Cos(phi), cosT, -sinT * MathF.Sin(phi));
                vertices.Add(Vertex.Plain(normal * 0.5f, normal, new Vector2((float)segment / segments, (float)ring / rings)));
            }
        }
        var indices = new List<uint>();
        var stride = segments + 1;
        for (var ring = 0u; ring < rings; ring++)
        {
            for (var segment = 0u; segment < segments; segment++)
            {
                var a = ring * stride + segment;
                var b = a + stride;
                indices.AddRange([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        return ([.. vertices], [.. indices]);
    }
}

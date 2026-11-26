using System.Numerics;

namespace Bayleaf.Assets;

/// <summary>A node at rest.</summary>
public readonly record struct SkeletonNode(int? Parent, Vector3 Translation, Quaternion Rotation, Vector3 Scale)
{
    public NodeTransform Transform => new(Translation, Rotation, Scale);
}

/// <summary>What vertices are bound to: a node, and the matrix from the bound pose into its space.</summary>
public readonly record struct Binding(int Node, Matrix4x4 InverseBind);

/// <summary>Up to four bindings of a vertex, by index into <see cref="Skeleton.Bindings"/>.</summary>
public readonly record struct BindingSet(ushort B0, ushort B1, ushort B2, ushort B3)
{
    public ushort this[int index] => index switch
    {
        0 => B0,
        1 => B1,
        2 => B2,
        _ => B3,
    };
}

/// <summary>Which part of a node's place a channel moves.</summary>
public enum ChannelProperty
{
    Translation,
    Rotation,
    Scale,
}

/// <summary>Keys of one property of one node.</summary>
/// <param name="Node">The node it moves.</param>
/// <param name="Property">What it moves.</param>
/// <param name="Step">Holds each key until the next instead of moving between them.</param>
/// <param name="Times">Seconds, rising.</param>
/// <param name="Values">x, y, z (and w for a rotation) of each key.</param>
public sealed record AnimationChannel(int Node, ChannelProperty Property, bool Step, IReadOnlyList<float> Times, IReadOnlyList<Vector4> Values);

/// <summary>A model's animation.</summary>
/// <param name="Name">Its name in the file (<c>Animation 2</c> for the second when it has none).</param>
/// <param name="Duration">Seconds, to its last key.</param>
/// <param name="Channels">What it moves.</param>
public sealed record ModelAnimation(string Name, float Duration, IReadOnlyList<AnimationChannel> Channels);

/// <summary>A node's place against its parent: scaled, turned, then moved.</summary>
public readonly record struct NodeTransform(Vector3 Translation, Quaternion Rotation, Vector3 Scale)
{
    public static readonly NodeTransform Identity = new(Vector3.Zero, Quaternion.Identity, Vector3.One);

    /// <summary>The matrix: a point is <c>Vector3.Transform(p, Matrix())</c>.</summary>
    public Matrix4x4 Matrix() => FMath.FromScaleRotationTranslation(Scale, Rotation, Translation);

    /// <summary>From this (0) to <paramref name="other"/> (1) by <paramref name="t"/>: places and scales in a straight line, the turn along the shorter arc.</summary>
    public NodeTransform Mix(NodeTransform other, float t) =>
        new(Translation + (other.Translation - Translation) * t, FMath.Normalize(FMath.Slerp(Rotation, other.Rotation, t)), Scale + (other.Scale - Scale) * t);
}

/// <summary>Every node's place against its parent, by node index.</summary>
public sealed record Pose(NodeTransform[] Locals)
{
    /// <summary><paramref name="t"/> as a mixing factor: 0 to 1, and 0 for a number that is not one.</summary>
    private static float Factor(float t) => float.IsFinite(t) ? Math.Clamp(t, 0f, 1f) : 0f;

    /// <summary>This pose mixed towards <paramref name="other"/> by <paramref name="t"/> (0 all this, 1 all the other), node by node.</summary>
    public Pose Mix(Pose other, float t)
    {
        t = Factor(t);
        return new([.. Locals.Select((a, i) => i < other.Locals.Length ? a.Mix(other.Locals[i], t) : a)]);
    }

    /// <summary>Like <see cref="Mix"/>, each node by <paramref name="t"/> times its weight in <paramref name="mask"/> (a node the mask leaves at 0 keeps this pose's place).</summary>
    public Pose MixMasked(Pose other, float t, IReadOnlyList<float> mask)
    {
        t = Factor(t);
        return new([.. Locals.Select((a, i) => i < other.Locals.Length && i < mask.Count && mask[i] > 0 ? a.Mix(other.Locals[i], t * Math.Min(mask[i], 1f)) : a)]);
    }
}

/// <summary>
/// A model's skeleton and animations (glTF): every node of the model's scene can move, and each
/// vertex follows up to four of them. A part without a skin follows its own node entirely; a
/// skinned part follows its skin's joints by its weights. An animation moves nodes by keys of
/// translation, rotation and scale over time; <see cref="Posed"/> gives the model's vertices at a
/// moment of one. Cubic spline keys are read as their values, joined in straight lines.
/// </summary>
/// <remarks>
/// A <see cref="Assets.Pose"/> is every node's place against its parent. Poses mix, also on part of the
/// skeleton only (with a mask from <see cref="Mask"/>: a joint and its children). <see cref="World"/>
/// turns a pose into each node's matrix in the model, and <see cref="SkinMatrices"/> into the
/// matrices vertices follow: the renderer sends those to the GPU, which moves the vertices there;
/// <see cref="SkinOnCpu"/> does the same on the CPU. Matrices are System.Numerics': a point is
/// <c>Vector3.Transform(p, m)</c>, and in <c>a * b</c> the transform a applies first.
/// </remarks>
public sealed class Skeleton
{
    /// <summary>Most keys one model's animations may hold in all.</summary>
    public const int MaxKeys = 4_000_000;

    /// <summary>Every node of the model, by its index in the file.</summary>
    public required IReadOnlyList<SkeletonNode> Nodes { get; init; }

    /// <summary>The nodes in an order with each parent before its children.</summary>
    public required IReadOnlyList<int> Order { get; init; }

    public required IReadOnlyList<Binding> Bindings { get; init; }

    /// <summary>For each vertex of the model: up to four bindings and their weights (adding up to 1).</summary>
    public required IReadOnlyList<BindingSet> VertexBindings { get; init; }

    public required IReadOnlyList<Vector4> VertexWeights { get; init; }

    /// <summary>The vertices in the bound pose, before the import scale and axes.</summary>
    public required IReadOnlyList<Vector3> Positions { get; init; }

    public required IReadOnlyList<Vector3> Normals { get; init; }

    /// <summary>The import scale and axes, applied after the pose.</summary>
    public required Matrix4x4 Root { get; init; }

    public required IReadOnlyList<ModelAnimation> Animations { get; init; }

    /// <summary>Each node's name in the file, by index (<c>Node 4</c> for the fourth when it has none).</summary>
    public required IReadOnlyList<string> Names { get; init; }

    /// <summary>
    /// For each binding, the box (smallest and largest corner) round the bound-pose vertices that
    /// follow it, so a posed model's box is found without moving every vertex.
    /// </summary>
    public IReadOnlyList<(Vector3 Min, Vector3 Max)> BindingBounds { get; private set; } = [];

    /// <summary>Fills <see cref="BindingBounds"/> from the vertices.</summary>
    internal Skeleton WithBounds()
    {
        BindingBounds = BoundsOfBindings();
        return this;
    }

    /// <summary>The animation called <paramref name="name"/>, or the first when the name is empty.</summary>
    public int? Animation(string name)
    {
        if (name.Length == 0)
        {
            return Animations.Count > 0 ? 0 : null;
        }
        for (var i = 0; i < Animations.Count; i++)
        {
            if (Animations[i].Name == name)
            {
                return i;
            }
        }
        return null;
    }

    /// <summary>Every node at rest.</summary>
    public Pose RestPose() => new([.. Nodes.Select(n => n.Transform)]);

    /// <summary>The node called <paramref name="name"/>.</summary>
    public int? NodeNamed(string name)
    {
        for (var i = 0; i < Names.Count; i++)
        {
            if (Names[i] == name)
            {
                return i;
            }
        }
        return null;
    }

    /// <summary>
    /// 1 for the node called <paramref name="joint"/> and every node under it, 0 for the others:
    /// what a layer that plays on that part of the skeleton moves. All 1 when the name is empty;
    /// null when no node has that name.
    /// </summary>
    public float[]? Mask(string joint)
    {
        var mask = new float[Nodes.Count];
        if (joint.Length == 0)
        {
            Array.Fill(mask, 1f);
            return mask;
        }
        if (NodeNamed(joint) is not { } top)
        {
            return null;
        }
        // Parents come before their children in the order, so one pass reaches every descendant.
        foreach (var i in Order)
        {
            var parentInside = i < Nodes.Count && Nodes[i].Parent is { } p && p < mask.Length && mask[p] > 0;
            if ((i == top || parentInside) && i < mask.Length)
            {
                mask[i] = 1f;
            }
        }
        return mask;
    }

    /// <summary>
    /// The time <paramref name="animation"/> is at after <paramref name="time"/> seconds: wrapped
    /// round when looping, else held at its end; 0 for an animation of no length.
    /// </summary>
    public float ClipTime(int? animation, float time, bool looping)
    {
        var duration = animation is { } a && a >= 0 && a < Animations.Count ? Animations[a].Duration : 0f;
        if (!(duration > 0 && float.IsFinite(time)))
        {
            return 0f;
        }
        return looping ? FMath.RemEuclid(time, duration) : Math.Clamp(time, 0f, duration);
    }

    /// <summary>
    /// The animation's keys at <paramref name="time"/> seconds written over <paramref name="pose"/>
    /// (the nodes it does not move keep their place). The time is used as given: wrap or hold it
    /// first (<see cref="ClipTime"/>).
    /// </summary>
    public void SampleInto(int animation, float time, Pose pose)
    {
        if (animation < 0 || animation >= Animations.Count)
        {
            return;
        }
        foreach (var channel in Animations[animation].Channels)
        {
            if (channel.Node < 0 || channel.Node >= pose.Locals.Length || Sample(channel, time) is not { } v)
            {
                continue;
            }
            ref var node = ref pose.Locals[channel.Node];
            node = channel.Property switch
            {
                ChannelProperty.Translation => node with { Translation = new(v.X, v.Y, v.Z) },
                ChannelProperty.Rotation => node with { Rotation = FMath.Normalize(new Quaternion(v.X, v.Y, v.Z, v.W)) },
                _ => node with { Scale = new(v.X, v.Y, v.Z) },
            };
        }
    }

    /// <summary>The pose the animation gives at <paramref name="time"/> seconds: the rest pose with its keys written over it.</summary>
    public Pose Sampled(int animation, float time)
    {
        var pose = RestPose();
        SampleInto(animation, time, pose);
        return pose;
    }

    /// <summary>Each node's matrix into the model's space (before the import scale and axes) in <paramref name="pose"/>.</summary>
    public Matrix4x4[] World(Pose pose)
    {
        var world = new Matrix4x4[Nodes.Count];
        Array.Fill(world, Matrix4x4.Identity);
        foreach (var i in Order)
        {
            if (i < 0 || i >= world.Length)
            {
                continue;
            }
            var own = i < pose.Locals.Length ? pose.Locals[i].Matrix() : Matrix4x4.Identity;
            world[i] = Nodes[i].Parent is { } p && p >= 0 && p < world.Length ? own * world[p] : own;
        }
        return world;
    }

    /// <summary>Each node's matrix into the model's space at <paramref name="time"/> seconds into the animation, or at rest.</summary>
    public Matrix4x4[] PoseAt(int? animation, float time) => World(animation is { } a ? Sampled(a, time) : RestPose());

    /// <summary>
    /// The matrix each binding moves its vertices by, from the bound pose to the posed model (with
    /// the import scale and axes), given <paramref name="world"/> from <see cref="World"/>: what the
    /// vertex shader reads.
    /// </summary>
    public Matrix4x4[] SkinMatrices(IReadOnlyList<Matrix4x4> world) =>
        [.. Bindings.Select(b => b.InverseBind * (b.Node >= 0 && b.Node < world.Count ? world[b.Node] : Matrix4x4.Identity) * Root)];

    /// <summary>
    /// The box round the model posed by <paramref name="matrices"/> (from <see cref="SkinMatrices"/>):
    /// each binding's own box moved by its matrix, together. It may be larger than the model, never smaller.
    /// </summary>
    public (Vector3 Min, Vector3 Max)? SkinnedBounds(IReadOnlyList<Matrix4x4> matrices)
    {
        var (min, max) = (new Vector3(float.PositiveInfinity), new Vector3(float.NegativeInfinity));
        for (var b = 0; b < Math.Min(BindingBounds.Count, matrices.Count); b++)
        {
            var (lo, hi) = BindingBounds[b];
            if (lo.X > hi.X)
            {
                continue;
            }
            for (var corner = 0; corner < 8; corner++)
            {
                var p = Vector3.Transform(new Vector3((corner & 1) == 0 ? lo.X : hi.X, (corner & 2) == 0 ? lo.Y : hi.Y, (corner & 4) == 0 ? lo.Z : hi.Z), matrices[b]);
                min = FMath.Min(min, p);
                max = FMath.Max(max, p);
            }
        }
        static bool Finite(Vector3 v) => float.IsFinite(v.X) && float.IsFinite(v.Y) && float.IsFinite(v.Z);
        return Finite(min) && Finite(max) ? (min, max) : null;
    }

    /// <summary>
    /// The box round the vertices that follow each binding, in the bound pose; a binding no vertex
    /// follows gets an empty box (its smallest corner above its largest).
    /// </summary>
    public (Vector3 Min, Vector3 Max)[] BoundsOfBindings()
    {
        var bounds = new (Vector3 Min, Vector3 Max)[Bindings.Count];
        Array.Fill(bounds, (new Vector3(float.PositiveInfinity), new Vector3(float.NegativeInfinity)));
        for (var i = 0; i < Positions.Count; i++)
        {
            var bound = i < VertexBindings.Count ? VertexBindings[i] : default;
            var weights = i < VertexWeights.Count ? VertexWeights[i] : default;
            for (var k = 0; k < 4; k++)
            {
                if (weights[k] > 0 && bound[k] < bounds.Length)
                {
                    ref var box = ref bounds[bound[k]];
                    box = (FMath.Min(box.Min, Positions[i]), FMath.Max(box.Max, Positions[i]));
                }
            }
        }
        return bounds;
    }

    /// <summary>
    /// <paramref name="mesh"/> with its vertices where the animation puts them at <paramref name="time"/>
    /// (wrapped round when looping, else held at the end), under the id and the version given.
    /// </summary>
    public MeshData Posed(MeshData mesh, int? animation, float time, bool looping, ulong id, ulong version)
    {
        time = ClipTime(animation, time, looping);
        return SkinOnCpu(mesh, SkinMatrices(PoseAt(animation, time)), id, version);
    }

    /// <summary>
    /// <paramref name="mesh"/> with each vertex moved by its bindings' <paramref name="matrices"/> by
    /// their weights, under the id and the version given: what the GPU does when it skins, done on the CPU.
    /// </summary>
    public MeshData SkinOnCpu(MeshData mesh, IReadOnlyList<Matrix4x4> matrices, ulong id, ulong version)
    {
        var positions = new Vector3[Positions.Count];
        var normals = new Vector3[Positions.Count];
        for (var i = 0; i < Positions.Count; i++)
        {
            var bound = i < VertexBindings.Count ? VertexBindings[i] : default;
            var weights = i < VertexWeights.Count ? VertexWeights[i] : Vector4.UnitX;
            var m = default(Matrix4x4);
            for (var k = 0; k < 4; k++)
            {
                if (weights[k] > 0)
                {
                    m += FMath.Scale(bound[k] < matrices.Count ? matrices[bound[k]] : Root, weights[k]);
                }
            }
            positions[i] = Vector3.Transform(Positions[i], m);
            var n = i < Normals.Count ? Normals[i] : Vector3.UnitY;
            normals[i] = FMath.NormalizeOr(Vector3.TransformNormal(n, FMath.NormalMatrix(m)), Vector3.UnitY);
        }
        var (min, max) = MeshData.Bounds(positions);
        return mesh with { Id = id, Version = version, Positions = positions, Normals = normals, Min = min, Max = max };
    }

    private static Vector4? Sample(AnimationChannel channel, float time) => SampleKeys(channel.Property, channel.Step, channel.Times, channel.Values, time);

    /// <summary>
    /// The value keys of <paramref name="property"/> at <paramref name="times"/> (rising) with
    /// <paramref name="values"/> give at <paramref name="time"/>, as a channel's are read: held at
    /// the ends, held from key to key when stepping, else between the two keys round it (a
    /// rotation along the shorter arc).
    /// </summary>
    public static Vector4? SampleKeys(ChannelProperty property, bool step, IReadOnlyList<float> times, IReadOnlyList<Vector4> values, float time)
    {
        if (times.Count == 0)
        {
            return null;
        }
        Vector4? Value(int i) => i < values.Count ? values[i] : null;
        if (time <= times[0])
        {
            return Value(0);
        }
        if (time >= times[^1])
        {
            return Value(times.Count - 1);
        }
        // The first key after the time (the times rise).
        int lo = 0, hi = times.Count;
        while (lo < hi)
        {
            var mid = (lo + hi) / 2;
            if (times[mid] <= time)
            {
                lo = mid + 1;
            }
            else
            {
                hi = mid;
            }
        }
        var next = lo;
        var before = next - 1;
        if (before < 0 || Value(before) is not { } a || Value(next) is not { } b || next >= times.Count)
        {
            return null;
        }
        if (step)
        {
            return a;
        }
        var span = times[next] - times[before];
        var t = span > 0 ? (time - times[before]) / span : 0f;
        if (property == ChannelProperty.Rotation)
        {
            var q = FMath.Slerp(FMath.Normalize(new Quaternion(a.X, a.Y, a.Z, a.W)), FMath.Normalize(new Quaternion(b.X, b.Y, b.Z, b.W)), t);
            return new Vector4(q.X, q.Y, q.Z, q.W);
        }
        return a + (b - a) * t;
    }
}

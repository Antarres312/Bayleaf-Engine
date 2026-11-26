// The game: the benchmark's three scripts and its scene, written against Runtime.cs the way game code
// would be (bench/runtime/rust/src/game.rs and bench/runtime/cpp/game.cpp are the same game). Each
// script runs in both styles with the same body: in an array of its type (typed) and boxed behind
// the interface (virtual).

namespace Bayleaf.Bench.Runtime;

/// Rotation Y += speed x dt.
internal struct Spin(float speed) : IScript
{
    public readonly void Start(ref Me me)
    {
    }

    public readonly void Update(ref Me me, in Frame frame)
    {
        ref var y = ref me.TransformMut().Ry;
        y = (y + (speed * frame.Dt)) % 360f;
    }
}

/// Position Y = base + height x sin(time x speed).
internal struct Bob(float speed, float height, float baseY) : IScript
{
    public readonly void Start(ref Me me)
    {
    }

    // Positions are kept in double, set from single precision as Bayleaf's scripts set them.
    public readonly void Update(ref Me me, in Frame frame) =>
        me.TransformMut().Py = baseY + (height * MathF.Sin(frame.Seconds * speed));
}

/// Position X and Z ease towards the object named Target, found at start.
internal struct Follow(float rate) : IScript
{
    private Entity target;

    public void Start(ref Me me) => target = me.World.Find("Target");

    public readonly void Update(ref Me me, in Frame frame)
    {
        if (!me.World.Alive(target))
        {
            return;
        }
        var k = MathF.Min(rate * frame.Dt, 1f);
        ref readonly var goal = ref me.World.TransformOf(target);
        float goalX = (float)goal.Px, goalZ = (float)goal.Pz;
        ref var p = ref me.TransformMut();
        var here = (float)p.Px;
        p.Px = here + ((goalX - here) * k);
        here = (float)p.Pz;
        p.Pz = here + ((goalZ - here) * k);
    }
}

internal static class Game
{
    // Per-object property values: the same formulas as bench/src/main.rs.
    private static float SpinSpeed(int i) => 30f + ((i % 7) * 10f);

    private static float BobSpeed(int i) => 1f + ((i % 5) * 0.25f);

    private static float BobHeight(int i) => 0.25f + ((i % 3) * 0.1f);

    private static float FollowRate(int i) => 0.05f + ((i % 4) * 0.05f);

    private static void Attach<T>(Runtime runtime, Entity e, T script, bool typed)
        where T : struct, IScript
    {
        if (typed)
        {
            runtime.Add(e, script);
        }
        else
        {
            runtime.AddVirtual(e, script);
        }
    }

    /// The scene of bench/src/main.rs: the target first, then a grid of objects, each with a mesh, a
    /// collider, a tag, a light on every tenth, and the three scripts.
    public static void Build(Runtime runtime, int n, bool typed)
    {
        var world = runtime.World;
        world.Create(new Id(0, 1), "Target");
        var side = Math.Max((int)Math.Ceiling(Math.Sqrt(n)), 1);
        for (var i = 0; i < n; i++)
        {
            var id = new Id(unchecked(0x9e37_79b9_7f4a_7c15UL * (ulong)(i + 1)), (ulong)i + 2);
            var e = world.Create(id, $"Object {i}");
            ref var t = ref world.TransformMut(e);
            t.Px = ((i % side) * 2.0) - side;
            t.Pz = ((i / side) * 2.0) - side;
            world.AddMesh(e, Mesh.Default);
            world.AddCollider(e, Collider.Default);
            world.Tag(e, "bench");
            if (i % 10 == 0)
            {
                world.AddLight(e, PointLight.Default);
            }
            Attach(runtime, e, new Spin(SpinSpeed(i)), typed);
            Attach(runtime, e, new Bob(BobSpeed(i), BobHeight(i), 0.5f), typed);
            Attach(runtime, e, new Follow(FollowRate(i)), typed);
        }
    }
}

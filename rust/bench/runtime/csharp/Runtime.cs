// A small game runtime for the comparison in bench/README.md ("The same runtime in C++ and Rust"):
// the design a fast Bayleaf runtime would have, written here the way C# 14 on .NET 10 would write it
// (bench/runtime/rust/src/runtime.rs and bench/runtime/cpp/runtime.hpp are the same runtime).
//
// - Components in packed arrays, an object is an index with a generation.
// - Objects are found by a 128-bit id or a name through hash maps, at start, not per step.
// - Writing a transform marks the object moved, once per step; world matrices are worked out for
//   the moved objects only, and the list of them is what a renderer or an editor would read.
// - Game scripts are structs kept in one array per script type; the runtime makes one virtual call
//   per type per step ("typed"), and inside it the script's update is a direct call, since the type
//   is a struct the compiler specialises for. For comparison, the usual heap object per script
//   behind an interface call per object ("virtual") runs on the same world.

using System.Runtime.InteropServices;

namespace Bayleaf.Bench.Runtime;

internal readonly record struct Id(ulong Hi, ulong Lo);

/// Position in double, rotation (degrees) and scale in single precision: 48 bytes, as in Rust and C++.
[StructLayout(LayoutKind.Sequential)]
internal struct Transform
{
    public double Px, Py, Pz;
    public float Rx, Ry, Rz;
    public float Sx, Sy, Sz;

    public static Transform Default => new() { Sx = 1, Sy = 1, Sz = 1 };
}

internal struct Mesh
{
    public int Shape;
    public float R, G, B, A;
    public float Metallic, Roughness;

    public static Mesh Default => new() { Shape = 0, R = 0.8f, G = 0.8f, B = 0.8f, A = 1f, Metallic = 0f, Roughness = 0.5f };
}

internal struct Collider
{
    public int Shape;
    public float Sx, Sy, Sz;
    public float Friction, Restitution;
    public bool Trigger;

    public static Collider Default => new() { Shape = 0, Sx = 1, Sy = 1, Sz = 1, Friction = 0.5f, Restitution = 0f, Trigger = false };
}

internal struct PointLight
{
    public float R, G, B;
    public float Intensity, Range;

    public static PointLight Default => new() { R = 1, G = 1, B = 1, Intensity = 1, Range = 10 };
}

/// An object: its place in the arrays, and the generation that place had when the handle was made.
internal readonly record struct Entity(uint Index, uint Generation);

/// The step's time step, and the time since start in double and, converted once per step, in single
/// precision. Scripts read `Seconds` rather than converting `Time` themselves: .NET's JIT converts
/// into a register a previous object's sine still holds, so each object waited for the last one's
/// (Bob took 0.076 ms at 5000 objects instead of 0.028). The values are the same.
internal readonly record struct Frame(float Dt, double Time)
{
    public float Seconds { get; } = (float)Time;
}

/// A growable array whose items are reached by reference (a `List<T>` hands out copies).
internal sealed class Column<T>
{
    private T[] items = new T[16];

    public int Count { get; private set; }

    public ref T this[int index] => ref items[index];

    public void Add(T item)
    {
        if (Count == items.Length)
        {
            Array.Resize(ref items, items.Length * 2);
        }
        items[Count++] = item;
    }

    public Span<T> AsSpan() => items.AsSpan(0, Count);
}

internal sealed class World
{
    private readonly Column<Id> ids = new();
    private readonly Column<string> names = new();
    // The arrays every step touches are held here directly, one load away, not behind a
    // `Column`; `count` of their places are in use.
    private Transform[] transforms = new Transform[16];
    // Sixteen doubles per object, column-major.
    private double[] world = new double[16 * 16];
    private uint[] generation = new uint[16];
    private bool[] movedFlag = new bool[16];
    // Each object is in it at most once, so it never needs more places than there are objects.
    private uint[] moved = new uint[16];
    private int count;
    private int movedCount;
    private readonly List<(uint Owner, Mesh Mesh)> meshes = [];
    private readonly List<(uint Owner, Collider Collider)> colliders = [];
    private readonly List<(uint Owner, PointLight Light)> lights = [];
    private readonly List<(uint Owner, string Tag)> tags = [];
    private readonly Dictionary<Id, uint> byId = [];
    private readonly Dictionary<string, uint> byName = new(StringComparer.Ordinal);

    public int Count => count;

    public Entity Create(Id id, string name)
    {
        var index = (uint)count;
        byId.TryAdd(id, index);
        byName.TryAdd(name, index);
        ids.Add(id);
        names.Add(name);
        if (count == transforms.Length)
        {
            var places = count * 2;
            Array.Resize(ref transforms, places);
            Array.Resize(ref world, places * 16);
            Array.Resize(ref generation, places);
            Array.Resize(ref movedFlag, places);
            Array.Resize(ref moved, places);
        }
        transforms[count] = Transform.Default;
        generation[count] = 1;
        count++;
        Mark(index);
        return new Entity(index, 1);
    }

    public bool Alive(Entity e) => e.Index < (uint)count && generation[e.Index] == e.Generation;

    /// The first object of that name; generation 0 (never alive) when there is none.
    public Entity Find(string name) => byName.TryGetValue(name, out var i) ? new Entity(i, generation[(int)i]) : default;

    public Entity FindId(Id id) => byId.TryGetValue(id, out var i) ? new Entity(i, generation[(int)i]) : default;

    public ref readonly Transform TransformOf(Entity e) => ref transforms[(int)e.Index];

    /// For writing: the object counts as moved this step.
    public ref Transform TransformMut(Entity e)
    {
        Mark(e.Index);
        return ref transforms[(int)e.Index];
    }

    public ReadOnlySpan<double> WorldOf(Entity e) => world.AsSpan((int)e.Index * 16, 16);

    public void AddMesh(Entity e, Mesh c) => meshes.Add((e.Index, c));

    public void AddCollider(Entity e, Collider c) => colliders.Add((e.Index, c));

    public void AddLight(Entity e, PointLight c) => lights.Add((e.Index, c));

    public void Tag(Entity e, string tag) => tags.Add((e.Index, tag));

    /// World matrices of the objects moved since the last call. Returns what a consumer of the moved
    /// list (a renderer, an editor's viewport) would take: here, the sum of their world x positions.
    public double FinishStep()
    {
        var taken = 0.0;
        foreach (var index in moved.AsSpan(0, movedCount))
        {
            var i = (int)index;
            var m = world.AsSpan(i * 16, 16);
            WorldMatrix(in transforms[i], m);
            taken += m[12];
            movedFlag[i] = false;
        }
        movedCount = 0;
        return taken;
    }

    /// Only forgets what moved (for timing the scripts alone).
    public void ForgetMoved()
    {
        foreach (var index in moved.AsSpan(0, movedCount))
        {
            movedFlag[index] = false;
        }
        movedCount = 0;
    }

    public double Checksum()
    {
        var sum = 0.0;
        var all = transforms.AsSpan(0, count);
        for (var i = 0; i < all.Length; i++)
        {
            ref readonly var t = ref all[i];
            sum += t.Px + t.Py + t.Pz + (double)t.Ry + world[i * 16];
        }
        return sum;
    }

    /// A sum over every component that only holds data, and the target found by its id: equal across
    /// languages when the scenes are the same.
    public double DataSum()
    {
        var sum = 0.0;
        foreach (var (_, m) in meshes)
        {
            sum += m.Shape + (double)(m.R + m.G + m.B + m.A + m.Metallic + m.Roughness);
        }
        foreach (var (_, c) in colliders)
        {
            sum += c.Shape + (double)(c.Sx + c.Sy + c.Sz + c.Friction + c.Restitution) + (c.Trigger ? 1.0 : 0.0);
        }
        foreach (var (_, l) in lights)
        {
            sum += (double)(l.R + l.G + l.B + l.Intensity + l.Range);
        }
        foreach (var (_, tag) in tags)
        {
            sum += tag.Length;
        }
        return sum + WorldOf(FindId(new Id(0, 1)))[15];
    }

    private void Mark(uint index)
    {
        ref var flag = ref movedFlag[index];
        if (!flag)
        {
            flag = true;
            moved[movedCount++] = index;
        }
    }

    /// Translation * rotation (X, then Y, then Z, degrees) * scale, column-major (as bench/cpp/bench.cpp).
    private static void WorldMatrix(in Transform t, Span<double> m)
    {
        const double rad = Math.PI / 180.0;
        double ax = t.Rx * rad, ay = t.Ry * rad, az = t.Rz * rad;
        double cx = Math.Cos(ax), sx = Math.Sin(ax), cy = Math.Cos(ay), sy = Math.Sin(ay), cz = Math.Cos(az), sz = Math.Sin(az);
        // Written out: the three columns of the rotation, each times its axis' scale.
        double x = t.Sx, y = t.Sy, z = t.Sz;
        m[15] = 1.0;
        m[0] = cy * cz * x;
        m[1] = cy * sz * x;
        m[2] = -sy * x;
        m[3] = 0.0;
        m[4] = ((sx * sy * cz) - (cx * sz)) * y;
        m[5] = ((sx * sy * sz) + (cx * cz)) * y;
        m[6] = sx * cy * y;
        m[7] = 0.0;
        m[8] = ((cx * sy * cz) + (sx * sz)) * z;
        m[9] = ((cx * sy * sz) - (sx * cz)) * z;
        m[10] = cx * cy * z;
        m[11] = 0.0;
        m[12] = t.Px;
        m[13] = t.Py;
        m[14] = t.Pz;
    }
}

/// What a script works with: the world and its own object.
internal struct Me(World world, Entity entity)
{
    public World World { get; } = world;

    public Entity Entity { get; set; } = entity;

    public readonly ref Transform TransformMut() => ref World.TransformMut(Entity);
}

/// A game script. In a [`Typed`] array of structs it is the "typed" style; boxed behind the interface,
/// the "virtual". Both methods are written out in every script: a default method would box a struct.
internal interface IScript
{
    void Start(ref Me me);

    void Update(ref Me me, in Frame frame);
}

/// "typed": every script of one type in one array; one virtual call per type per step.
internal abstract class Store
{
    public abstract void StartAll(World world);

    public abstract void UpdateAll(World world, in Frame frame);
}

internal sealed class Typed<T> : Store
    where T : struct, IScript
{
    public Column<Entity> Owners { get; } = new();

    public Column<T> Items { get; } = new();

    public override void StartAll(World world)
    {
        var owners = Owners.AsSpan();
        var items = Items.AsSpan();
        var me = new Me(world, default);
        for (var i = 0; i < items.Length; i++)
        {
            me.Entity = owners[i];
            items[i].Start(ref me);
        }
    }

    public override void UpdateAll(World world, in Frame frame)
    {
        var owners = Owners.AsSpan();
        var items = Items.AsSpan();
        var me = new Me(world, default);
        for (var i = 0; i < items.Length; i++)
        {
            me.Entity = owners[i];
            items[i].Update(ref me, in frame);
        }
    }
}

internal sealed class Runtime
{
    private readonly List<Store> stores = [];
    private readonly Dictionary<Type, int> slots = [];
    private readonly List<List<IScript>> owned = [];

    public World World { get; } = new();

    /// Typed: the script joins the array of its type.
    public void Add<T>(Entity owner, T script)
        where T : struct, IScript
    {
        if (!slots.TryGetValue(typeof(T), out var slot))
        {
            stores.Add(new Typed<T>());
            slot = stores.Count - 1;
            slots.Add(typeof(T), slot);
        }
        if (stores[slot] is Typed<T> store)
        {
            store.Owners.Add(owner);
            store.Items.Add(script);
        }
    }

    /// Virtual: the object owns its scripts, each a heap object.
    public void AddVirtual(Entity owner, IScript script)
    {
        var index = (int)owner.Index;
        while (owned.Count <= index)
        {
            owned.Add([]);
        }
        owned[index].Add(script);
    }

    public void Start()
    {
        foreach (var store in stores)
        {
            store.StartAll(World);
        }
        var me = new Me(World, default);
        for (var i = 0; i < owned.Count; i++)
        {
            me.Entity = new Entity((uint)i, 1);
            foreach (var script in owned[i])
            {
                script.Start(ref me);
            }
        }
    }

    public void Update(in Frame frame)
    {
        foreach (var store in CollectionsMarshal.AsSpan(stores))
        {
            store.UpdateAll(World, in frame);
        }
        var me = new Me(World, default);
        var all = CollectionsMarshal.AsSpan(owned);
        for (var i = 0; i < all.Length; i++)
        {
            me.Entity = new Entity((uint)i, 1);
            foreach (var script in CollectionsMarshal.AsSpan(all[i]))
            {
                script.Update(ref me, in frame);
            }
        }
    }
}

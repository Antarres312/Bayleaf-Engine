namespace Bayleaf.Ecs.Tests;

internal readonly record struct Angle(float Value);

internal readonly record struct Spin(float Value);

internal readonly record struct Health(float Value);

internal readonly record struct Regen(float Value);

internal readonly record struct Lit;

/// <summary>Systems with declared access, views and the parallel schedule.</summary>
public class ParallelTests
{
    private const float Dt = 1f / 60f;

    /// <summary>Entities spread over several archetypes, more of them than one job of a parallel loop takes.</summary>
    private static void Populate(World world, int n)
    {
        for (var i = 0; i < n; i++)
        {
            var e = world.Spawn().Value;
            world.Insert(e, new Position(i));
            world.Insert(e, new Velocity(i % 7 * 0.5f));
            if (i % 3 == 0)
            {
                world.Insert(e, new Angle(0));
                world.Insert(e, new Spin(i * 0.01f));
            }
            if (i % 2 == 0)
            {
                world.Insert(e, new Health(i % 90f));
                world.Insert(e, new Regen(0.5f));
            }
            if (i % 10 == 0)
            {
                world.Insert(e, new Lit());
            }
        }
    }

    /// <summary>The structural step both schedules share: one entity more, one light less.</summary>
    private static void Restructure(World w)
    {
        if (w.Spawn().TryGet(out var e, out _))
        {
            w.Insert(e, new Position(-1));
            w.Insert(e, new Velocity(3));
        }
        var lit = w.Query<Lit>().Select(r => r.Entity).ToList();
        if (lit.Count > 0)
        {
            w.Remove<Lit>(lit.Min());
        }
    }

    private static Schedule ViewSchedule()
    {
        var s = new Schedule();
        s.Add("move", new Access().Write<Position>().Read<Velocity>(), v => v.ForEach((Entity _, ref Position p, in Velocity vel) => p = new Position(p.Value + vel.Value * Dt)));
        s.Add("spin", new Access().Write<Angle>().Read<Spin>(), v => v.ForEach((Entity _, ref Angle a, in Spin sp) => a = new Angle(MathF.Sin(a.Value + sp.Value))));
        s.Add("heal", new Access().Write<Health>().Read<Regen>(), v => v.ForEach((Entity _, ref Health h, in Regen r) => h = new Health(MathF.Min(h.Value + r.Value, 100))));
        s.Add("drag", new Access().Write<Velocity>(), v => v.ForEach((Entity _, ref Velocity vel) => vel = new Velocity(vel.Value * 0.99f)));
        s.Add("follow", new Access().Write<Angle>().Read<Position>(), v => v.ForEach((Entity _, ref Angle a, in Position p) => a = new Angle(a.Value * 0.5f + p.Value * 0.001f)));
        s.Add("restructure", Restructure);
        s.Add("move back", new Access().Write<Position>().Read<Velocity>(), v =>
        {
            foreach (var (e, vel) in v.Query<Velocity>().ToList())
            {
                v.Update(e, (ref Position p) => p = new Position(p.Value - vel.Value * Dt * 0.5f));
            }
        });
        return s;
    }

    /// <summary>Changes every entity's <typeparamref name="TA"/> from its <typeparamref name="TB"/>, through the whole world.</summary>
    private static void Each<TA, TB>(World w, Func<TA, TB, TA> f)
    {
        foreach (var (e, b) in w.Query<TB>().ToList())
        {
            w.Update(e, (ref TA a) => a = f(a, b));
        }
    }

    /// <summary>The same systems as <see cref="ViewSchedule"/> on the whole world, one after another.</summary>
    private static Schedule ReferenceSchedule()
    {
        var s = new Schedule();
        s.Add("move", w => Each<Position, Velocity>(w, (p, vel) => new Position(p.Value + vel.Value * Dt)));
        s.Add("spin", w => Each<Angle, Spin>(w, (a, sp) => new Angle(MathF.Sin(a.Value + sp.Value))));
        s.Add("heal", w => Each<Health, Regen>(w, (h, r) => new Health(MathF.Min(h.Value + r.Value, 100))));
        s.Add("drag", w =>
        {
            foreach (var (e, _) in w.Query<Velocity>().ToList())
            {
                w.Update(e, (ref Velocity vel) => vel = new Velocity(vel.Value * 0.99f));
            }
        });
        s.Add("follow", w => Each<Angle, Position>(w, (a, p) => new Angle(a.Value * 0.5f + p.Value * 0.001f)));
        s.Add("restructure", Restructure);
        s.Add("move back", w => Each<Position, Velocity>(w, (p, vel) => new Position(p.Value - vel.Value * Dt * 0.5f)));
        return s;
    }

    /// <summary>Every component's exact bits, by entity.</summary>
    private static List<List<(Entity, uint)>> Snapshot(World w)
    {
        static List<(Entity, uint)> Bits<T>(World w, Func<T, float> f) => w.Query<T>().Select(r => (r.Entity, BitConverter.SingleToUInt32Bits(f(r.Value)))).Order().ToList();
        return [Bits<Position>(w, c => c.Value), Bits<Velocity>(w, c => c.Value), Bits<Angle>(w, c => c.Value), Bits<Spin>(w, c => c.Value), Bits<Health>(w, c => c.Value), Bits<Lit>(w, _ => 1)];
    }

    [Fact]
    public void AParallelScheduleEndsWithTheSameWorldAsOneThread()
    {
        var worlds = Enumerable.Range(0, 3).Select(_ => new World()).ToList();
        foreach (var w in worlds)
        {
            Populate(w, 3000);
        }
        Assert.Equal(World.ParallelAvailable, worlds[2].SetParallel(true));
        var schedules = new[] { ReferenceSchedule(), ViewSchedule(), ViewSchedule() };
        for (var step = 0; step < 5; step++)
        {
            for (var i = 0; i < 3; i++)
            {
                schedules[i].Run(worlds[i]);
            }
        }
        var reference = Snapshot(worlds[0]);
        Assert.All(reference, c => Assert.NotEmpty(c));
        Assert.Equal(reference, Snapshot(worlds[1]));
        Assert.Equal(reference, Snapshot(worlds[2]));
    }

    [Fact]
    public void SystemsJoinTheEarliestBatchAfterWhatTheyConflictWith()
    {
        var s = new Schedule();
        static void None(View _)
        {
        }
        s.Add("a", new Access().Write<Position>().Read<Velocity>(), None);
        s.Add("b", new Access().Write<Angle>().Read<Spin>(), None);
        s.Add("c", new Access().Read<Position>().Write<Health>(), None);
        s.Add("d", _ => { });
        s.Add("e", new Access().Write<Velocity>(), None);
        s.Add("f", new Access().Write<Regen>(), None);
        s.Add("g", new Access().Read<Velocity>().Write<Spin>(), None);
        s.Add("h", new Access().Read<Velocity>().Write<Angle>(), None);
        Assert.Equal([["a", "b"], ["c"], ["d"], ["e", "f"], ["g", "h"]], s.Batches.Select(b => b.ToList()).ToList());
    }

    [Fact]
    public void ConflictingSystemsNeverRunTogether()
    {
        var world = new World();
        Populate(world, 2000);
        world.SetParallel(true);
        var (writers, readers, overlaps) = (new int[1], new int[1], new int[1]);
        var s = new Schedule();
        // Each writer is followed by readers, which share a batch and so run at the same time.
        for (var k = 0; k < 3; k++)
        {
            s.Add($"writer {k}", new Access().Write<Position>(), v =>
            {
                if (Interlocked.Increment(ref writers[0]) > 1 || Volatile.Read(ref readers[0]) > 0)
                {
                    Interlocked.Increment(ref overlaps[0]);
                }
                v.ForEach((Entity _, ref Position p) => p = new Position(p.Value + 1));
                Thread.Sleep(2);
                Interlocked.Decrement(ref writers[0]);
            });
            for (var j = 0; j < 3; j++)
            {
                s.Add($"reader {k}.{j}", new Access().Read<Position>(), v =>
                {
                    Interlocked.Increment(ref readers[0]);
                    if (Volatile.Read(ref writers[0]) > 0)
                    {
                        Interlocked.Increment(ref overlaps[0]);
                    }
                    _ = v.Query<Position>().Sum(r => r.Value.Value);
                    Thread.Sleep(2);
                    Interlocked.Decrement(ref readers[0]);
                });
            }
        }
        Assert.Equal(6, s.Batches.Count);
        for (var i = 0; i < 3; i++)
        {
            s.Run(world);
        }
        Assert.Equal(0, overlaps[0]);
    }

    [Fact]
    public void AViewSeesOnlyWhatItsAccessNames()
    {
        var world = new World();
        var e = world.Spawn().Value;
        world.Insert(e, new Position(1));
        world.Insert(e, new Velocity(2));
        var view = world.View(new Access().Read<Position>());
        Assert.True(view.TryGet<Position>(e, out _));
        Assert.False(view.Update(e, (ref Position p) => p = new Position(5)), "read-only");
        Assert.False(view.TryGet<Velocity>(e, out _), "not named");
        var visited = 0;
        view.ForEach((Entity _, ref Position _) => visited++);
        Assert.Equal(0, visited);
    }
}

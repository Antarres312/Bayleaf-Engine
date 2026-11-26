namespace Bayleaf.Ecs.Tests;

internal readonly record struct Position(float Value);

internal readonly record struct Velocity(float Value);

internal readonly record struct Label(string Value);

public class WorldTests
{
    [Fact]
    public void AddingAComponentKeepsTheOthers()
    {
        var world = new World();
        var e = world.Spawn().Value;
        world.Insert(e, new Position(1));
        world.Insert(e, new Velocity(2));
        world.Insert(e, new Label("a"));
        Assert.Equal(new Position(1), world.Get<Position>(e).Value);
        Assert.Equal(new Velocity(2), world.Get<Velocity>(e).Value);
        Assert.Equal(new Label("a"), world.Get<Label>(e).Value);
    }

    [Fact]
    public void RemovingAComponentReturnsItAndKeepsTheOthers()
    {
        var world = new World();
        var e = world.Spawn().Value;
        world.Insert(e, new Position(1));
        world.Insert(e, new Velocity(2));
        Assert.Equal(new Position(1), world.Remove<Position>(e).Value.Value);
        Assert.False(world.Has<Position>(e));
        Assert.Equal(new Velocity(2), world.Get<Velocity>(e).Value);
        Assert.False(world.Remove<Position>(e).Value.HasValue);
    }

    [Fact]
    public void MovingOneEntityDoesNotDisturbItsNeighbours()
    {
        var world = new World();
        var entities = Enumerable.Range(0, 4).Select(_ => world.Spawn().Value).ToList();
        for (var i = 0; i < entities.Count; i++)
        {
            world.Insert(entities[i], new Position(i));
        }
        // Moving the first entity to another archetype swaps the last row into its place.
        world.Insert(entities[0], new Velocity(9));
        world.Despawn(entities[1]);
        foreach (var i in new[] { 0, 2, 3 })
        {
            Assert.Equal(new Position(i), world.Get<Position>(entities[i]).Value);
        }
    }

    [Fact]
    public void ADespawnedHandleStaysDeadAfterItsSlotIsReused()
    {
        var world = new World();
        var old = world.Spawn().Value;
        world.Insert(old, new Position(1));
        world.Despawn(old);
        var reused = world.Spawn().Value;
        Assert.False(world.IsAlive(old));
        Assert.True(world.IsAlive(reused));
        Assert.False(world.Get<Position>(old).HasValue);
        Assert.True(world.Insert(old, new Position(2)).IsError);
    }

    [Fact]
    public void QueryVisitsExactlyTheMatchingEntities()
    {
        var world = new World();
        var (a, b, c) = (world.Spawn().Value, world.Spawn().Value, world.Spawn().Value);
        world.Insert(a, new Position(1));
        world.Insert(b, new Position(2));
        world.Insert(b, new Velocity(1));
        world.Insert(c, new Velocity(1));
        Assert.Equal(new[] { a, b }.Order(), world.Query<Position>().Select(r => r.Entity).Order());
    }

    [Fact]
    public void ScheduleRunsSystemsInOrder()
    {
        var world = new World();
        var e = world.Spawn().Value;
        world.Insert(e, new Position(0));
        world.Insert(e, new Velocity(2));
        var schedule = new Schedule();
        schedule.Add("double velocity", w => w.ForEach((Entity _, ref Velocity v) => v = new Velocity(v.Value * 2)));
        schedule.Add("integrate", w =>
        {
            foreach (var (entity, v) in w.Query<Velocity>().ToList())
            {
                w.Update(entity, (ref Position p) => p = new Position(p.Value + v.Value));
            }
        });
        schedule.Run(world);
        // Integration sees the doubled velocity only if it runs second.
        Assert.True(world.Get<Position>(e).Value.Value > 2);
    }
}

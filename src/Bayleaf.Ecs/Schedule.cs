namespace Bayleaf.Ecs;

/// <summary>
/// Systems run against a <see cref="World"/>, in order. A system added with <see cref="Add(string, Action{World})"/>
/// gets the whole world and runs alone, on the calling thread, after everything before it and before
/// everything after it: structural changes and other writes that must be ordered go there. A system
/// added with <see cref="Add(string, Access, Action{View})"/> declares an <see cref="Access"/> and gets
/// a <see cref="View"/> of those columns. When the world's parallel switch is on, such systems run in
/// batches: a system joins the earliest batch after every earlier system it conflicts with (and after
/// the last whole-world system), and the systems of a batch run at once. Systems that conflict keep
/// their order and systems that do not touch different columns, so the world ends the same as when
/// every system runs in turn.
/// </summary>
public sealed class Schedule
{
    private readonly List<(string Name, Action<World>? World, Access? Access, Action<View>? View)> systems = [];

    /// <summary>Each batch: one whole-world system, or view systems that may run at once.</summary>
    private readonly List<(bool World, List<int> Systems)> batches = [];

    /// <summary>Appends a system that gets the whole world. It runs alone, after every system added before it.</summary>
    public void Add(string name, Action<World> system)
    {
        batches.Add((true, [systems.Count]));
        systems.Add((name, system, null, null));
    }

    /// <summary>
    /// Appends a system that touches only the components <paramref name="access"/> names. It runs after
    /// every earlier system it conflicts with, and may run at the same time as others.
    /// </summary>
    public void Add(string name, Access access, Action<View> system)
    {
        var index = systems.Count;
        var floor = batches.FindLastIndex(b => b.World) + 1;
        var after = floor;
        for (var b = floor; b < batches.Count; b++)
        {
            if (!batches[b].World && batches[b].Systems.Exists(i => systems[i].Access is { } other && other.ConflictsWith(access)))
            {
                after = b + 1;
            }
        }
        if (after < batches.Count && !batches[after].World)
        {
            batches[after].Systems.Add(index);
        }
        else
        {
            batches.Add((false, [index]));
        }
        systems.Add((name, null, access, system));
    }

    public IEnumerable<string> Names => systems.Select(s => s.Name);

    /// <summary>The systems' names in the batches they run in when parallel is on, in order.</summary>
    public IReadOnlyList<IReadOnlyList<string>> Batches => batches.Select(b => (IReadOnlyList<string>)b.Systems.Select(i => systems[i].Name).ToList()).ToList();

    /// <summary>Runs every system once: in the order added, or in batches when the world's parallel switch is on.</summary>
    public void Run(World world)
    {
        if (!world.Parallel)
        {
            foreach (var system in systems)
            {
                if (system.World is { } run)
                {
                    run(world);
                }
                else if (system.Access is { } access && system.View is { } view)
                {
                    view(world.View(access));
                }
            }
            return;
        }
        foreach (var (isWorld, ids) in batches)
        {
            if (isWorld)
            {
                systems[ids[0]].World?.Invoke(world);
                continue;
            }
            var accesses = ids.Select(i => systems[i].Access ?? new Access()).ToList();
            var views = View.Split(world, accesses);
            Pool.Run(true, ids.Select((system, at) => (Run: systems[system].View, View: views[at])).ToList(), job => job.Run?.Invoke(job.View));
        }
    }
}

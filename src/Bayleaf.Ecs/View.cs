namespace Bayleaf.Ecs;

/// <summary>
/// What one system may touch: the columns its <see cref="Access"/> names, in every archetype that
/// has them. Components it did not name are not there: loops skip them and lookups find nothing.
/// Views of systems that do not conflict hold disjoint written columns, so they can be used on
/// different threads at once.
/// </summary>
public sealed class View
{
    /// <summary>Fewest rows a job of a parallel loop gets: below this, handing rows to another thread costs more than it saves.</summary>
    private const int MinRowsPerJob = 256;

    private readonly World world;

    /// <summary>By archetype index; null for archetypes with none of the view's types.</summary>
    private readonly List<Part?> parts;

    private sealed class Part(List<Entity> entities)
    {
        public List<Entity> Entities { get; } = entities;

        public Dictionary<int, (IColumn Column, bool Write)> Columns { get; } = [];

        public Column<T>? Read<T>() => Columns.TryGetValue(ComponentType<T>.Id, out var c) ? c.Column as Column<T> : null;

        public Column<T>? Write<T>() => Columns.TryGetValue(ComponentType<T>.Id, out var c) && c.Write ? c.Column as Column<T> : null;
    }

    private View(World world, List<Part?> parts, bool parallel)
    {
        this.world = world;
        this.parts = parts;
        Parallel = parallel;
    }

    /// <summary>
    /// Splits <paramref name="world"/> into one view per access. Accesses must not conflict with
    /// each other; where they do, a written column goes to the first access that writes it and no other.
    /// </summary>
    internal static View[] Split(World world, IReadOnlyList<Access> accesses)
    {
        var perView = accesses.Select(_ => new List<Part?>()).ToArray();
        foreach (var archetype in world.Archetypes)
        {
            var archetypeParts = accesses.Select(_ => new Part(archetype.Entities)).ToArray();
            foreach (var (typeId, column) in archetype.Columns)
            {
                var writer = -1;
                for (var i = 0; i < accesses.Count; i++)
                {
                    if (accesses[i].Writes(typeId))
                    {
                        writer = i;
                        break;
                    }
                }
                if (writer >= 0)
                {
                    archetypeParts[writer].Columns[typeId] = (column, true);
                    continue;
                }
                for (var i = 0; i < accesses.Count; i++)
                {
                    if (accesses[i].Reads(typeId))
                    {
                        archetypeParts[i].Columns[typeId] = (column, false);
                    }
                }
            }
            for (var i = 0; i < accesses.Count; i++)
            {
                perView[i].Add(archetypeParts[i].Columns.Count > 0 ? archetypeParts[i] : null);
            }
        }
        return perView.Select(p => new View(world, p, world.Parallel)).ToArray();
    }

    /// <summary>Whether this view's loops run on the thread pool.</summary>
    public bool Parallel { get; }

    /// <summary>Every entity that has a <typeparamref name="T"/>, if the view reads or writes it (copies).</summary>
    public IEnumerable<(Entity Entity, T Value)> Query<T>()
    {
        foreach (var part in parts)
        {
            if (part?.Read<T>() is { } column)
            {
                for (var row = 0; row < column.Count; row++)
                {
                    yield return (part.Entities[row], column[row]);
                }
            }
        }
    }

    public bool TryGet<T>(Entity entity, out T value)
    {
        if (world.LocationOf(entity) is { } loc && parts[loc.Archetype]?.Read<T>() is { } column && loc.Row < column.Count)
        {
            value = column[loc.Row];
            return true;
        }
        value = default!;
        return false;
    }

    /// <summary>Changes the entity's <typeparamref name="T"/> in place, if the view writes it; false otherwise.</summary>
    public bool Update<T>(Entity entity, Mutation<T> change)
    {
        if (world.LocationOf(entity) is { } loc && parts[loc.Archetype]?.Write<T>() is { } column && loc.Row < column.Count)
        {
            change(ref column[loc.Row]);
            return true;
        }
        return false;
    }

    /// <summary>
    /// Calls <paramref name="action"/> for every entity that has a <typeparamref name="T"/> the view
    /// writes, on the thread pool when parallel is on. Each call sees only its own entity's component,
    /// so the result does not depend on the threads.
    /// </summary>
    public void ForEach<T>(RowAction<T> action)
    {
        var rows = RowsPerJob(Parallel, parts.Sum(p => p?.Read<T>()?.Count ?? 0));
        var jobs = new List<(Part Part, Column<T> Column, int Start, int Count)>();
        foreach (var part in parts)
        {
            if (part?.Write<T>() is { } column)
            {
                for (var start = 0; start < column.Count; start += rows)
                {
                    jobs.Add((part, column, start, Math.Min(rows, column.Count - start)));
                }
            }
        }
        Pool.Run(Parallel, jobs, job =>
        {
            var span = job.Column.Span;
            for (var row = job.Start; row < job.Start + job.Count; row++)
            {
                action(job.Part.Entities[row], ref span[row]);
            }
        });
    }

    /// <summary>
    /// Calls <paramref name="action"/> for every entity that has both a <typeparamref name="TWrite"/>
    /// the view writes and a <typeparamref name="TRead"/> it reads, on the thread pool when parallel is on.
    /// </summary>
    public void ForEach<TWrite, TRead>(RowAction<TWrite, TRead> action)
    {
        if (ComponentType<TWrite>.Id == ComponentType<TRead>.Id)
        {
            return;
        }
        var rows = RowsPerJob(Parallel, parts.Sum(p => p?.Read<TWrite>() is { } w && p.Read<TRead>() is not null ? w.Count : 0));
        var jobs = new List<(Part Part, Column<TWrite> Written, Column<TRead> Read, int Start, int Count)>();
        foreach (var part in parts)
        {
            if (part?.Write<TWrite>() is { } written && part.Read<TRead>() is { } read)
            {
                for (var start = 0; start < written.Count; start += rows)
                {
                    jobs.Add((part, written, read, start, Math.Min(rows, written.Count - start)));
                }
            }
        }
        Pool.Run(Parallel, jobs, job =>
        {
            var w = job.Written.Span;
            var r = job.Read.Span;
            for (var row = job.Start; row < job.Start + job.Count; row++)
            {
                action(job.Part.Entities[row], ref w[row], in r[row]);
            }
        });
    }

    /// <summary>Rows per job: all of an archetype's on one thread, else enough jobs to keep every thread busy, but none under <see cref="MinRowsPerJob"/>.</summary>
    private static int RowsPerJob(bool parallel, int total)
    {
        if (!parallel)
        {
            return int.MaxValue;
        }
        var jobs = Math.Max(1, Pool.Threads * 4);
        return Math.Max(MinRowsPerJob, (total + jobs - 1) / jobs);
    }
}

/// <summary>The thread pool, which runs jobs at once when asked to and there is more than one.</summary>
internal static class Pool
{
    public static int Threads => Environment.ProcessorCount;

    public static void Run<TJob>(bool parallel, IReadOnlyList<TJob> jobs, Action<TJob> run)
    {
        if (parallel && jobs.Count > 1)
        {
            System.Threading.Tasks.Parallel.For(0, jobs.Count, new ParallelOptions { MaxDegreeOfParallelism = Threads }, i => run(jobs[i]));
            return;
        }
        foreach (var job in jobs)
        {
            run(job);
        }
    }
}

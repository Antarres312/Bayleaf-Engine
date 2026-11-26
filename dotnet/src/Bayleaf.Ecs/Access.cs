namespace Bayleaf.Ecs;

/// <summary>
/// The component types a system reads and writes. Two systems conflict when one writes a type the
/// other reads or writes; systems that do not conflict may run at the same time.
/// </summary>
public sealed class Access
{
    private readonly List<(int Id, string Name)> reads = [];
    private readonly List<(int Id, string Name)> writes = [];

    public Access()
    {
    }

    private Access(Access from)
    {
        reads.AddRange(from.reads);
        writes.AddRange(from.writes);
    }

    /// <summary>Adds read access to <typeparamref name="T"/>. A type that is also written counts as written.</summary>
    public Access Read<T>()
    {
        var next = new Access(this);
        if (!next.reads.Exists(r => r.Id == ComponentType<T>.Id))
        {
            next.reads.Add((ComponentType<T>.Id, typeof(T).FullName ?? typeof(T).Name));
        }
        return next;
    }

    /// <summary>Adds write access to <typeparamref name="T"/>, which includes reading it.</summary>
    public Access Write<T>()
    {
        var next = new Access(this);
        if (!next.writes.Exists(w => w.Id == ComponentType<T>.Id))
        {
            next.writes.Add((ComponentType<T>.Id, typeof(T).FullName ?? typeof(T).Name));
        }
        return next;
    }

    internal bool Reads(int typeId) => reads.Exists(r => r.Id == typeId) || Writes(typeId);

    internal bool Writes(int typeId) => writes.Exists(w => w.Id == typeId);

    public bool Reads<T>() => Reads(ComponentType<T>.Id);

    public bool Writes<T>() => Writes(ComponentType<T>.Id);

    /// <summary>Whether this and <paramref name="other"/> must not run at the same time.</summary>
    public bool ConflictsWith(Access other) => writes.Exists(w => other.Reads(w.Id)) || other.writes.Exists(w => Reads(w.Id));

    public override string ToString() => $"reads [{string.Join(", ", reads.Select(r => r.Name))}], writes [{string.Join(", ", writes.Select(w => w.Name))}]";
}

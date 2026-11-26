namespace Bayleaf.Ecs;

/// <summary>A small number for each component type, given the first time the type is used.</summary>
internal static class ComponentType
{
    private static int next;

    public static int Next() => Interlocked.Increment(ref next);
}

/// <summary>The number of <typeparamref name="T"/>.</summary>
internal static class ComponentType<T>
{
    public static readonly int Id = ComponentType.Next();
}

/// <summary>One archetype's values of one component type, row by row.</summary>
internal interface IColumn
{
    int TypeId { get; }

    int Count { get; }

    IColumn EmptyLike();

    /// <summary>Moves the value at <paramref name="row"/> to the end of <paramref name="destination"/>, filling the row with the last value.</summary>
    void MoveRow(int row, IColumn destination);

    void DropRow(int row);
}

internal sealed class Column<T> : IColumn
{
    private T[] items = [];

    public int TypeId => ComponentType<T>.Id;

    public int Count { get; private set; }

    public Span<T> Span => items.AsSpan(0, Count);

    public ref T this[int row] => ref items[row];

    public IColumn EmptyLike() => new Column<T>();

    public void Add(T value)
    {
        if (Count == items.Length)
        {
            Array.Resize(ref items, Math.Max(4, items.Length * 2));
        }
        items[Count++] = value;
    }

    /// <summary>Takes the value at <paramref name="row"/> out, filling the row with the last value.</summary>
    public T SwapRemove(int row)
    {
        var value = items[row];
        Count--;
        items[row] = items[Count];
        items[Count] = default!;
        return value;
    }

    public void MoveRow(int row, IColumn destination)
    {
        if (row < Count && destination is Column<T> typed)
        {
            typed.Add(SwapRemove(row));
        }
    }

    public void DropRow(int row)
    {
        if (row < Count)
        {
            SwapRemove(row);
        }
    }
}

/// <summary>The sorted component types of an archetype, as a key.</summary>
internal readonly struct TypeSet : IEquatable<TypeSet>
{
    private readonly int[] types;

    public TypeSet(IEnumerable<int> types) => this.types = [.. types.Order()];

    public IReadOnlyList<int> Types => types;

    public bool Equals(TypeSet other) => types.AsSpan().SequenceEqual(other.types);

    public override bool Equals(object? obj) => obj is TypeSet other && Equals(other);

    public override int GetHashCode()
    {
        var hash = new HashCode();
        foreach (var t in types)
        {
            hash.Add(t);
        }
        return hash.ToHashCode();
    }
}

internal sealed class Archetype
{
    public Dictionary<int, IColumn> Columns { get; } = [];

    public List<Entity> Entities { get; } = [];

    public Column<T>? Column<T>() => Columns.TryGetValue(ComponentType<T>.Id, out var c) ? c as Column<T> : null;
}

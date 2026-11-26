using Bayleaf.Core;

namespace Bayleaf.Ecs;

/// <summary>Handle to an entity. A despawned entity's handle stays invalid even after its slot is reused.</summary>
public readonly record struct Entity : IComparable<Entity>
{
    internal Entity(uint index, uint generation)
    {
        Index = index;
        Generation = generation;
    }

    public uint Index { get; }

    public uint Generation { get; }

    public int CompareTo(Entity other) => (Index, Generation).CompareTo((other.Index, other.Generation));

    public override string ToString() => $"Entity {{ index: {Index}, generation: {Generation} }}";

    public static bool operator <(Entity left, Entity right) => left.CompareTo(right) < 0;

    public static bool operator >(Entity left, Entity right) => left.CompareTo(right) > 0;

    public static bool operator <=(Entity left, Entity right) => left.CompareTo(right) <= 0;

    public static bool operator >=(Entity left, Entity right) => left.CompareTo(right) >= 0;
}

public enum EcsErrorKind
{
    NoSuchEntity,
    TooManyEntities,
}

/// <summary>A world operation that could not be done.</summary>
public sealed record EcsError(EcsErrorKind Kind, Entity Entity)
{
    public override string ToString() => Kind == EcsErrorKind.NoSuchEntity ? $"entity {Entity} does not exist" : "entity limit reached";
}

/// <summary>Calls with an entity's component to change.</summary>
public delegate void RowAction<T>(Entity entity, ref T value);

/// <summary>Calls with an entity's component to change and another to read.</summary>
public delegate void RowAction<TWrite, TRead>(Entity entity, ref TWrite written, in TRead read);

/// <summary>Changes a component in place.</summary>
public delegate void Mutation<T>(ref T value);

/// <summary>
/// Storage for every entity and component. Entities with the same set of component types share an
/// archetype, which stores one column per type; adding or removing a component moves the entity's
/// row to the matching archetype. Systems that declare an <see cref="Access"/> get a
/// <see cref="View"/> of just those columns, so the <see cref="Schedule"/> can run systems that do not
/// conflict at once, and a view's row loops can split their rows over threads, when
/// <see cref="SetParallel"/> turns it on (off by default, with the same results either way).
/// </summary>
public sealed class World
{
    private readonly List<Slot> slots = [];
    private readonly Stack<uint> free = new();
    private readonly Dictionary<TypeSet, int> byTypes = new() { [new TypeSet([])] = 0 };

    internal List<Archetype> Archetypes { get; } = [new Archetype()];

    internal record struct Location(int Archetype, int Row);

    internal record struct Slot(uint Generation, Location? Location);

    /// <summary>Whether this build can run systems and row loops on a thread pool.</summary>
    public static bool ParallelAvailable => true;

    public bool Parallel { get; private set; }

    /// <summary>Turns running on the thread pool on or off. Returns whether it is now on.</summary>
    public bool SetParallel(bool on)
    {
        Parallel = on && ParallelAvailable;
        return Parallel;
    }

    /// <summary>Creates an entity with no components.</summary>
    public Result<Entity, EcsError> Spawn()
    {
        uint index;
        if (free.Count > 0)
        {
            index = free.Pop();
        }
        else
        {
            if (slots.Count >= int.MaxValue)
            {
                return Result.Fail(new EcsError(EcsErrorKind.TooManyEntities, default));
            }
            index = (uint)slots.Count;
            slots.Add(new Slot(0, null));
        }
        var entity = new Entity(index, slots[(int)index].Generation);
        var empty = Archetypes[0];
        empty.Entities.Add(entity);
        SetLocation(entity, new Location(0, empty.Entities.Count - 1));
        return entity;
    }

    /// <summary>Removes an entity and all its components.</summary>
    public Result<Unit, EcsError> Despawn(Entity entity)
    {
        if (LocationOf(entity) is not { } loc)
        {
            return Result.Fail(new EcsError(EcsErrorKind.NoSuchEntity, entity));
        }
        foreach (var column in Archetypes[loc.Archetype].Columns.Values)
        {
            column.DropRow(loc.Row);
        }
        RemoveEntityRow(loc);
        var slot = slots[(int)entity.Index];
        slots[(int)entity.Index] = new Slot(unchecked(slot.Generation + 1), null);
        free.Push(entity.Index);
        return Unit.Value;
    }

    public bool IsAlive(Entity entity) => LocationOf(entity) is not null;

    /// <summary>Number of live entities.</summary>
    public int Count => Archetypes.Sum(a => a.Entities.Count);

    public bool IsEmpty => Count == 0;

    /// <summary>Adds <paramref name="value"/> to <paramref name="entity"/>, replacing a component of the same type.</summary>
    public Result<Unit, EcsError> Insert<T>(Entity entity, T value)
    {
        if (LocationOf(entity) is not { } loc)
        {
            return Result.Fail(new EcsError(EcsErrorKind.NoSuchEntity, entity));
        }
        var source = Archetypes[loc.Archetype];
        if (source.Column<T>() is { } existing && loc.Row < existing.Count)
        {
            existing[loc.Row] = value;
            return Unit.Value;
        }
        var typeId = ComponentType<T>.Id;
        var types = new TypeSet(source.Columns.Keys.Append(typeId));
        var destination = ArchetypeFor(types, loc.Archetype, static () => new Column<T>(), null);
        MoveEntity(entity, loc, destination, null);
        Archetypes[destination].Column<T>()?.Add(value);
        return Unit.Value;
    }

    /// <summary>Removes and returns the component of type <typeparamref name="T"/>, if the entity has one.</summary>
    public Result<Maybe<T>, EcsError> Remove<T>(Entity entity)
    {
        if (LocationOf(entity) is not { } loc)
        {
            return Result.Fail(new EcsError(EcsErrorKind.NoSuchEntity, entity));
        }
        var source = Archetypes[loc.Archetype];
        var typeId = ComponentType<T>.Id;
        if (source.Column<T>() is not { } column)
        {
            return default(Maybe<T>);
        }
        var destination = ArchetypeFor(new TypeSet(source.Columns.Keys.Where(t => t != typeId)), loc.Archetype, null, typeId);
        var taken = loc.Row < column.Count ? new Maybe<T>(column.SwapRemove(loc.Row)) : default;
        MoveEntity(entity, loc, destination, typeId);
        return taken;
    }

    public bool Has<T>(Entity entity) => LocationOf(entity) is { } loc && Archetypes[loc.Archetype].Column<T>() is not null;

    public bool TryGet<T>(Entity entity, out T value)
    {
        if (LocationOf(entity) is { } loc && Archetypes[loc.Archetype].Column<T>() is { } column && loc.Row < column.Count)
        {
            value = column[loc.Row];
            return true;
        }
        value = default!;
        return false;
    }

    /// <summary>The component, or the type's default when the entity has none.</summary>
    public Maybe<T> Get<T>(Entity entity) => TryGet<T>(entity, out var value) ? new Maybe<T>(value) : default;

    /// <summary>Changes the entity's <typeparamref name="T"/> in place; false when it has none.</summary>
    public bool Update<T>(Entity entity, Mutation<T> change)
    {
        if (LocationOf(entity) is { } loc && Archetypes[loc.Archetype].Column<T>() is { } column && loc.Row < column.Count)
        {
            change(ref column[loc.Row]);
            return true;
        }
        return false;
    }

    /// <summary>Every entity that has a <typeparamref name="T"/>, with the component (a copy).</summary>
    public IEnumerable<(Entity Entity, T Value)> Query<T>()
    {
        foreach (var archetype in Archetypes)
        {
            if (archetype.Column<T>() is { } column)
            {
                for (var row = 0; row < column.Count; row++)
                {
                    yield return (archetype.Entities[row], column[row]);
                }
            }
        }
    }

    /// <summary>A view of the columns <paramref name="access"/> names, as a system with that access gets.</summary>
    public View View(Access access) => Ecs.View.Split(this, [access])[0];

    /// <summary>Calls <paramref name="action"/> for every entity that has a <typeparamref name="T"/>, on the thread pool when parallel is on.</summary>
    public void ForEach<T>(RowAction<T> action) => View(new Access().Write<T>()).ForEach(action);

    /// <summary>Calls <paramref name="action"/> for every entity that has both components, on the thread pool when parallel is on.</summary>
    public void ForEach<TWrite, TRead>(RowAction<TWrite, TRead> action) => View(new Access().Write<TWrite>().Read<TRead>()).ForEach(action);

    internal Location? LocationOf(Entity entity) =>
        entity.Index < slots.Count && slots[(int)entity.Index] is { Location: { } loc } slot && slot.Generation == entity.Generation ? loc : null;

    private void SetLocation(Entity entity, Location? location)
    {
        if (entity.Index < slots.Count)
        {
            slots[(int)entity.Index] = slots[(int)entity.Index] with { Location = location };
        }
    }

    /// <summary>Finds or makes the archetype for <paramref name="types"/>, its columns like <paramref name="template"/>'s plus <paramref name="extra"/>, less <paramref name="without"/>.</summary>
    private int ArchetypeFor(TypeSet types, int template, Func<IColumn>? extra, int? without)
    {
        if (byTypes.TryGetValue(types, out var index))
        {
            return index;
        }
        var archetype = new Archetype();
        foreach (var (typeId, column) in Archetypes[template].Columns)
        {
            if (typeId != without)
            {
                archetype.Columns[typeId] = column.EmptyLike();
            }
        }
        if (extra?.Invoke() is { } added)
        {
            archetype.Columns[added.TypeId] = added;
        }
        Archetypes.Add(archetype);
        byTypes[types] = Archetypes.Count - 1;
        return Archetypes.Count - 1;
    }

    /// <summary>Moves every column value of <paramref name="entity"/> but <paramref name="skip"/> to <paramref name="destination"/>.</summary>
    private void MoveEntity(Entity entity, Location loc, int destination, int? skip)
    {
        if (destination == loc.Archetype)
        {
            return;
        }
        var source = Archetypes[loc.Archetype];
        var target = Archetypes[destination];
        foreach (var (typeId, column) in source.Columns)
        {
            if (typeId != skip && target.Columns.TryGetValue(typeId, out var targetColumn))
            {
                column.MoveRow(loc.Row, targetColumn);
            }
        }
        target.Entities.Add(entity);
        var newRow = target.Entities.Count - 1;
        RemoveEntityRow(loc);
        SetLocation(entity, new Location(destination, newRow));
    }

    /// <summary>Removes the entity at <paramref name="loc"/> and fixes the location of the entity swapped into its row.</summary>
    private void RemoveEntityRow(Location loc)
    {
        var entities = Archetypes[loc.Archetype].Entities;
        if (loc.Row >= entities.Count)
        {
            return;
        }
        var last = entities.Count - 1;
        entities[loc.Row] = entities[last];
        entities.RemoveAt(last);
        if (loc.Row < entities.Count)
        {
            SetLocation(entities[loc.Row], loc);
        }
    }
}

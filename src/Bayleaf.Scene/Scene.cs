using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Ecs;
using Bayleaf.Scene.Components;

namespace Bayleaf.Scene;

/// <summary>Where an object is kept in its scene (<see cref="Scene.Key"/>), to reach it without a lookup by id.</summary>
public readonly record struct ObjectKey(StableId Id, Entity Entity);

/// <summary>
/// The objects of one scene and their hierarchy. Every object is an ECS entity with a
/// <see cref="StableId"/>, a <see cref="Name"/> and a <see cref="Transform"/>. The parent and child
/// order lives in the scene, not in the ECS, so reordering never moves component data.
/// </summary>
public sealed class Scene
{
    /// <summary>Most objects one paste may add.</summary>
    public const int MaxImport = 100_000;

    /// <summary>Where the next scene's structure count starts: far apart, so no two scenes' counts meet.</summary>
    private static long nextStructure;

    private readonly Dictionary<StableId, Entity> entities = [];
    private readonly Dictionary<StableId, StableId?> parents = [];
    private readonly Dictionary<StableId, List<StableId>> children = [];
    private readonly List<StableId> roots = [];

    /// <summary>The objects whose transforms were written, in order; the first one's place in every write so far.</summary>
    private readonly List<StableId> transformLog = [];
    private ulong transformLogStart;

    public Scene()
    {
        Structure = (ulong)(Interlocked.Add(ref nextStructure, 1L << 40) - (1L << 40));
    }

    public ComponentRegistry Registry { get; } = ComponentRegistry.Builtin();

    /// <summary>The ECS world, for systems that read component data directly.</summary>
    public World World { get; } = new();

    /// <summary>The scene's own settings.</summary>
    public SceneSettings Settings { get; private set; } = SceneSettings.Default;

    /// <summary>
    /// A number that changes whenever anything but a transform does: an object comes or goes, is
    /// renamed or moved in the hierarchy, or a component is added, removed or changed. Equal numbers
    /// mean the same scene with the same structure.
    /// </summary>
    public ulong Structure { get; private set; }

    /// <summary>Changes the scene's own settings, after checking them.</summary>
    public Result<Unit, SceneError> SetSettings(SceneSettings settings)
    {
        var valid = settings.Validate();
        if (valid.IsError)
        {
            return Result.Fail(SceneError.InvalidValue("settings", valid.Error));
        }
        Settings = settings;
        return Unit.Value;
    }

    /// <summary>Where the log of transform writes stands now; give it to <see cref="TransformsWrittenSince"/> later to learn what moved in between.</summary>
    public ulong TransformMark => transformLogStart + (ulong)transformLog.Count;

    /// <summary>
    /// The objects whose transforms were written since <paramref name="mark"/>, in the order written;
    /// null when the log no longer reaches back that far, so the caller looks at every object.
    /// </summary>
    public IReadOnlyList<StableId>? TransformsWrittenSince(ulong mark)
    {
        if (mark < transformLogStart || mark - transformLogStart > (ulong)transformLog.Count)
        {
            return null;
        }
        return transformLog.GetRange((int)(mark - transformLogStart), transformLog.Count - (int)(mark - transformLogStart));
    }

    private void TransformWritten(StableId id)
    {
        // Past twice the objects (and a margin), looking at every object is cheaper than the log: it
        // starts again, and marks from before say so.
        if (transformLog.Count >= 2 * entities.Count + 1024)
        {
            transformLogStart += (ulong)transformLog.Count;
            transformLog.Clear();
        }
        transformLog.Add(id);
    }

    public int ObjectCount => entities.Count;

    public bool Contains(StableId id) => entities.ContainsKey(id);

    /// <summary>Children of <paramref name="parent"/> in order; null lists the root objects.</summary>
    public IReadOnlyList<StableId> Children(StableId? parent) => parent is { } p ? children.TryGetValue(p, out var list) ? list : [] : roots;

    private List<StableId> ChildList(StableId? parent)
    {
        if (parent is not { } p)
        {
            return roots;
        }
        if (!children.TryGetValue(p, out var list))
        {
            list = [];
            children[p] = list;
        }
        return list;
    }

    /// <summary>Parent of <paramref name="id"/>; null for a root object or an unknown id.</summary>
    public StableId? Parent(StableId id) => parents.TryGetValue(id, out var parent) ? parent : null;

    public string? NameOf(StableId id) => entities.TryGetValue(id, out var entity) && World.TryGet<Name>(entity, out var name) ? name.Value : null;

    /// <summary>Typed read of one component of an object.</summary>
    public bool TryGet<T>(StableId id, out T value)
    {
        if (entities.TryGetValue(id, out var entity))
        {
            return World.TryGet(entity, out value);
        }
        value = default!;
        return false;
    }

    public Maybe<T> Get<T>(StableId id) => TryGet<T>(id, out var value) ? new Maybe<T>(value) : default;

    /// <summary>Typed write of one component, checked like a value from a file or command.</summary>
    public Result<Unit, SceneError> Set<T>(StableId id, T value)
        where T : ISceneComponent<T>
    {
        var valid = value.Validate();
        if (valid.IsError)
        {
            return Result.Fail(SceneError.InvalidValue(T.Key, valid.Error));
        }
        var isTransform = T.Key == Transform.Key;
        if (!isTransform)
        {
            Structure++;
        }
        if (!entities.TryGetValue(id, out var entity))
        {
            return Result.Fail(SceneError.UnknownObject(id));
        }
        var inserted = World.Insert(entity, value);
        if (inserted.IsError)
        {
            return Result.Fail(SceneError.Internal(inserted.Error.ToString()));
        }
        if (isTransform)
        {
            TransformWritten(id);
        }
        return Unit.Value;
    }

    /// <summary>Where <paramref name="id"/> is kept, to set its transform again and again without looking it up. It stays valid while the object is in the scene.</summary>
    public ObjectKey? Key(StableId id) => entities.TryGetValue(id, out var entity) ? new ObjectKey(id, entity) : null;

    /// <summary>
    /// Sets an object's transform as <see cref="Set{T}"/> does, the fast way for many in a row. False
    /// when the object is no longer in the scene or the transform is not finite.
    /// </summary>
    public bool SetTransform(ObjectKey key, Transform value)
    {
        // The check Validate makes, without the list it builds for its message.
        if (!value.IsFinite)
        {
            return false;
        }
        // Insert replaces the transform where it is kept, with no closure made per write: play mode
        // writes every moved object's transform each step.
        if (World.Insert(key.Entity, value).IsError)
        {
            return false;
        }
        TransformWritten(key.Id);
        return true;
    }

    private DMat4 LocalMatrix(StableId id) => TryGet<Transform>(id, out var t) ? t.Matrix() : DMat4.Identity;

    /// <summary>Local-to-world matrix of every object, parents before children in hierarchy order.</summary>
    public IReadOnlyList<(StableId Id, DMat4 Matrix)> WorldMatrices()
    {
        var output = new List<(StableId, DMat4)>(entities.Count);
        var stack = new Stack<(StableId Id, DMat4 Parent)>(roots.AsEnumerable().Reverse().Select(id => (id, DMat4.Identity)));
        while (stack.TryPop(out var item))
        {
            var world = item.Parent * LocalMatrix(item.Id);
            output.Add((item.Id, world));
            foreach (var child in Children(item.Id).Reverse())
            {
                stack.Push((child, world));
            }
        }
        return output;
    }

    /// <summary>Local-to-world matrix of one object.</summary>
    public DMat4? WorldMatrix(StableId id)
    {
        if (!Contains(id))
        {
            return null;
        }
        var matrix = DMat4.Identity;
        StableId? current = id;
        while (current is { } obj)
        {
            matrix = LocalMatrix(obj) * matrix;
            current = Parent(obj);
        }
        return matrix;
    }

    /// <summary>Creates an object with default components, appended to <paramref name="parent"/>'s children.</summary>
    public Result<StableId, SceneError> CreateObject(StableId? parent, string name)
    {
        var id = StableId.Random();
        var inserted = InsertObject(id, parent);
        if (!inserted.TryGet(out var entity, out var error))
        {
            return Result.Fail(error);
        }
        foreach (var kind in Registry.Kinds.Where(k => !k.Removable))
        {
            Structure++;
            var added = kind.AddDefault(World, entity);
            if (added.IsError)
            {
                return Result.Fail(SceneError.Internal(added.Error));
            }
        }
        var named = World.Insert(entity, new Name(name));
        return named.IsOk ? id : Result.Fail(SceneError.Internal(named.Error.ToString()));
    }

    /// <summary>Deletes <paramref name="id"/> and all its descendants. Returns how many objects were deleted.</summary>
    public Result<int, SceneError> DeleteObject(StableId id)
    {
        if (!Contains(id))
        {
            return Result.Fail(SceneError.UnknownObject(id));
        }
        Structure++;
        ChildList(Parent(id)).Remove(id);
        var subtree = Subtree(id);
        foreach (var obj in subtree)
        {
            if (entities.Remove(obj, out var entity))
            {
                var despawned = World.Despawn(entity);
                if (despawned.IsError)
                {
                    return Result.Fail(SceneError.Internal(despawned.Error.ToString()));
                }
            }
            parents.Remove(obj);
            children.Remove(obj);
        }
        return subtree.Count;
    }

    /// <summary>Moves <paramref name="id"/> under <paramref name="newParent"/> at <paramref name="index"/> among its new siblings (appended when null or too large).</summary>
    public Result<Unit, SceneError> Reparent(StableId id, StableId? newParent, int? index)
    {
        if (!Contains(id))
        {
            return Result.Fail(SceneError.UnknownObject(id));
        }
        if (newParent is { } parent)
        {
            if (!Contains(parent))
            {
                return Result.Fail(SceneError.UnknownObject(parent));
            }
            if (Subtree(id).Contains(parent))
            {
                return Result.Fail(SceneError.Cycle);
            }
        }
        Structure++;
        ChildList(Parent(id)).Remove(id);
        var siblings = ChildList(newParent);
        var at = Math.Min(index ?? siblings.Count, siblings.Count);
        siblings.Insert(at, id);
        parents[id] = newParent;
        return Unit.Value;
    }

    /// <summary>
    /// Like <see cref="Reparent"/>, but rewrites the object's transform so it stays where it is in the
    /// world. Under a parent with rotation and uneven scale the result is the closest position, rotation
    /// and scale; a shear cannot be represented. Under a parent with zero scale the transform is left as it was.
    /// </summary>
    public Result<Unit, SceneError> ReparentKeepingWorld(StableId id, StableId? newParent, int? index)
    {
        if (WorldMatrix(id) is not { } world)
        {
            return Result.Fail(SceneError.UnknownObject(id));
        }
        var moved = Reparent(id, newParent, index);
        if (moved.IsError)
        {
            return moved;
        }
        var parentWorld = newParent is { } p && WorldMatrix(p) is { } m ? m : DMat4.Identity;
        if (Math.Abs(parentWorld.Determinant()) < 1e-12)
        {
            return Unit.Value;
        }
        // Round away float noise such as 0.49999997 or -0.0 left by the matrix maths.
        var transform = Transform.FromMatrix(parentWorld.Inverse() * world);
        static float Round4(float v) => MathF.Round(v * 1e4f, MidpointRounding.AwayFromZero) / 1e4f + 0f;
        transform = new Transform(
            transform.Position.Map(v => Math.Round(v * 1e6, MidpointRounding.AwayFromZero) / 1e6 + 0.0),
            new(Round4(transform.Rotation.X), Round4(transform.Rotation.Y), Round4(transform.Rotation.Z)),
            new(Round4(transform.Scale.X), Round4(transform.Scale.Y), Round4(transform.Scale.Z)));
        if (transform.IsFinite && entities.TryGetValue(id, out var entity))
        {
            var inserted = World.Insert(entity, transform);
            if (inserted.IsError)
            {
                return Result.Fail(SceneError.Internal(inserted.Error.ToString()));
            }
            TransformWritten(id);
        }
        return Unit.Value;
    }

    /// <summary>The components <paramref name="id"/> carries, as (name, label) in inspector order, without reading their values.</summary>
    public IReadOnlyList<(string Name, string Label)> ComponentKinds(StableId id) =>
        entities.TryGetValue(id, out var entity) ? Registry.Kinds.Where(k => k.Has(World, entity)).Select(k => (k.Name, k.Label)).ToList() : [];

    /// <summary>Components of <paramref name="id"/> as (name, value) in inspector order.</summary>
    public Result<IReadOnlyList<(string Name, JsonValue Value)>, SceneError> Components(StableId id)
    {
        if (!entities.TryGetValue(id, out var entity))
        {
            return Result.Fail(SceneError.UnknownObject(id));
        }
        return Registry.Kinds.Select(k => (k.Name, Value: k.Get(World, entity))).Where(c => c.Value is not null).Select(c => (c.Name, c.Value!)).ToList();
    }

    public Result<JsonValue, SceneError> Component(StableId id, string component)
    {
        if (!entities.TryGetValue(id, out var entity))
        {
            return Result.Fail(SceneError.UnknownObject(id));
        }
        if (Registry.Find(component) is not { } kind)
        {
            return Result.Fail(SceneError.UnknownComponent(component));
        }
        return kind.Get(World, entity) is { } value ? value : Result.Fail(SceneError.MissingComponent(id, component));
    }

    /// <summary>Replaces the value of a component the object already has.</summary>
    public Result<Unit, SceneError> SetComponent(StableId id, string component, JsonValue value)
    {
        var current = Component(id, component);
        return current.IsError ? current.AsFailure() : WriteComponent(id, component, value);
    }

    /// <summary>Adds a component with its default value.</summary>
    public Result<Unit, SceneError> AddComponent(StableId id, string component)
    {
        if (!entities.TryGetValue(id, out var entity))
        {
            return Result.Fail(SceneError.UnknownObject(id));
        }
        if (Registry.Find(component) is not { } kind)
        {
            return Result.Fail(SceneError.UnknownComponent(component));
        }
        if (kind.Get(World, entity) is not null)
        {
            return Result.Fail(SceneError.DuplicateComponent(id, component));
        }
        return kind.AddDefault(World, entity).MapError(SceneError.Internal);
    }

    public Result<Unit, SceneError> RemoveComponent(StableId id, string component)
    {
        if (!entities.TryGetValue(id, out var entity))
        {
            return Result.Fail(SceneError.UnknownObject(id));
        }
        if (Registry.Find(component) is not { } kind)
        {
            return Result.Fail(SceneError.UnknownComponent(component));
        }
        if (!kind.Removable)
        {
            return Result.Fail(SceneError.NotRemovable(component));
        }
        Structure++;
        return kind.Remove(World, entity) ? Unit.Value : Result.Fail(SceneError.MissingComponent(id, component));
    }

    private SortedDictionary<string, JsonValue> ComponentMap(StableId id)
    {
        var map = new SortedDictionary<string, JsonValue>(Utf8Order.Instance);
        if (Components(id).TryGet(out var list, out _))
        {
            foreach (var (name, value) in list)
            {
                map[name] = value;
            }
        }
        return map;
    }

    /// <summary>The scene as plain data, objects in depth-first hierarchy order.</summary>
    public SceneDoc ToDoc()
    {
        var objects = new List<ObjectDoc>(entities.Count);
        var stack = new Stack<StableId>(roots.AsEnumerable().Reverse());
        while (stack.TryPop(out var id))
        {
            objects.Add(new ObjectDoc(id, Parent(id), ComponentMap(id)));
            foreach (var child in Children(id).Reverse())
            {
                stack.Push(child);
            }
        }
        return new SceneDoc(SceneDoc.FormatName, SceneDoc.CurrentVersion, Settings, objects);
    }

    /// <summary>Builds a scene from plain data. Parents must come before their children.</summary>
    public static Result<Scene, SceneError> FromDoc(SceneDoc doc)
    {
        if (doc.Format != SceneDoc.FormatName)
        {
            return Result.Fail(SceneError.Format($"not a Bayleaf scene (format \"{doc.Format}\")"));
        }
        if (doc.Version is < 1 or > SceneDoc.CurrentVersion)
        {
            return Result.Fail(SceneError.Format($"unsupported version {doc.Version}"));
        }
        if (doc.Version == 1)
        {
            doc = MigrateV1(doc);
        }
        var scene = new Scene();
        var settings = scene.SetSettings(doc.Settings);
        if (settings.IsError)
        {
            return settings.AsFailure();
        }
        foreach (var obj in doc.Objects)
        {
            if (scene.Contains(obj.Id))
            {
                return Result.Fail(SceneError.Format($"object {obj.Id} appears twice"));
            }
            if (obj.Parent is { } parent && !scene.Contains(parent))
            {
                return Result.Fail(SceneError.Format($"object {obj.Id} is listed before its parent {parent}"));
            }
            var added = scene.AddObject(obj.Id, obj.Parent, obj.Components);
            if (added.IsError)
            {
                return added.AsFailure();
            }
        }
        return scene;
    }

    /// <summary>Adds an object with <paramref name="components"/>, then the required ones it lacks.</summary>
    private Result<Unit, SceneError> AddObject(StableId id, StableId? parent, SortedDictionary<string, JsonValue> components)
    {
        var inserted = InsertObject(id, parent);
        if (!inserted.TryGet(out var entity, out var error))
        {
            return Result.Fail(error);
        }
        foreach (var (name, value) in components)
        {
            var written = WriteComponent(id, name, value);
            if (written.IsError)
            {
                return written;
            }
        }
        foreach (var kind in Registry.Kinds.Where(k => !k.Removable))
        {
            if (kind.Get(World, entity) is null)
            {
                var added = kind.AddDefault(World, entity);
                if (added.IsError)
                {
                    return Result.Fail(SceneError.Internal(added.Error));
                }
            }
        }
        return Unit.Value;
    }

    /// <summary>
    /// <paramref name="roots"/> and all their descendants as plain data, parents first, for copying.
    /// The roots are listed without a parent; a root inside another root is copied once, with that root.
    /// </summary>
    public Result<IReadOnlyList<ObjectDoc>, SceneError> Export(IReadOnlyList<StableId> roots)
    {
        var objects = new List<ObjectDoc>();
        foreach (var root in roots)
        {
            if (!Contains(root))
            {
                return Result.Fail(SceneError.UnknownObject(root));
            }
            if (roots.Any(other => other != root && IsInside(root, other)) || objects.Exists(o => o.Id == root))
            {
                continue;
            }
            var stack = new Stack<StableId>([root]);
            while (stack.TryPop(out var id))
            {
                objects.Add(new ObjectDoc(id, id == root ? null : Parent(id), ComponentMap(id)));
                foreach (var child in Children(id).Reverse())
                {
                    stack.Push(child);
                }
            }
        }
        return objects;
    }

    /// <summary>
    /// Adds copies of <paramref name="objects"/> (as <see cref="Export"/> gives them) with new ids.
    /// Objects without a parent go under <paramref name="parent"/>, starting at <paramref name="index"/>
    /// among its children (appended when null). Returns the new ids of those top objects. Components
    /// are checked as when loading a file.
    /// </summary>
    public Result<IReadOnlyList<StableId>, SceneError> Import(IReadOnlyList<ObjectDoc> objects, StableId? parent, int? index)
    {
        if (parent is { } p && !Contains(p))
        {
            return Result.Fail(SceneError.UnknownObject(p));
        }
        if (objects.Count > MaxImport)
        {
            return Result.Fail(SceneError.Format($"at most {MaxImport} objects can be pasted at once"));
        }
        var newIds = new Dictionary<StableId, StableId>();
        var tops = new List<StableId>();
        foreach (var obj in objects)
        {
            if (newIds.ContainsKey(obj.Id))
            {
                return Result.Fail(SceneError.Format($"object {obj.Id} appears twice"));
            }
            StableId? newParent = parent;
            if (obj.Parent is { } old)
            {
                if (!newIds.TryGetValue(old, out var mapped))
                {
                    return Result.Fail(SceneError.Format($"object {obj.Id} is listed before its parent {old}"));
                }
                newParent = mapped;
            }
            var id = StableId.Random();
            var added = AddObject(id, newParent, obj.Components);
            if (added.IsError)
            {
                return added.AsFailure();
            }
            newIds[obj.Id] = id;
            if (obj.Parent is null)
            {
                tops.Add(id);
            }
        }
        if (index is { } at)
        {
            for (var offset = 0; offset < tops.Count; offset++)
            {
                var moved = Reparent(tops[offset], parent, at + offset);
                if (moved.IsError)
                {
                    return moved.AsFailure();
                }
            }
        }
        return tops;
    }

    /// <summary>True when <paramref name="id"/> lies somewhere below <paramref name="ancestor"/>.</summary>
    public bool IsInside(StableId id, StableId ancestor)
    {
        var current = Parent(id);
        while (current is { } parent)
        {
            if (parent == ancestor)
            {
                return true;
            }
            current = Parent(parent);
        }
        return false;
    }

    public static Result<Scene, SceneError> Load(string path)
    {
        var text = Files.ReadText(path);
        if (text.IsError)
        {
            return Result.Fail(SceneError.Io($"cannot read {path}: {text.Error}"));
        }
        var doc = SceneDoc.FromText(text.Value);
        return doc.IsOk ? FromDoc(doc.Value) : Result.Fail(SceneError.Format(doc.Error));
    }

    /// <summary>Writes the scene to <paramref name="path"/> through a temporary file, so a failed write keeps the old file.</summary>
    public Result<Unit, SceneError> Save(string path) => Files.WriteReplacing(path, Path.ChangeExtension(path, "bscene.tmp"), ToDoc().ToText()).MapError(SceneError.Io);

    private Result<Entity, SceneError> InsertObject(StableId id, StableId? parent)
    {
        if (parent is { } p && !Contains(p))
        {
            return Result.Fail(SceneError.UnknownObject(p));
        }
        Structure++;
        var spawned = World.Spawn();
        if (!spawned.TryGet(out var entity, out var error))
        {
            return Result.Fail(SceneError.Internal(error.ToString()));
        }
        var inserted = World.Insert(entity, id);
        if (inserted.IsError)
        {
            return Result.Fail(SceneError.Internal(inserted.Error.ToString()));
        }
        entities[id] = entity;
        parents[id] = parent;
        ChildList(parent).Add(id);
        return entity;
    }

    private Result<Unit, SceneError> WriteComponent(StableId id, string component, JsonValue value)
    {
        if (!entities.TryGetValue(id, out var entity))
        {
            return Result.Fail(SceneError.UnknownObject(id));
        }
        if (Registry.Find(component) is not { } kind)
        {
            return Result.Fail(SceneError.UnknownComponent(component));
        }
        var isTransform = kind.Name == Transform.Key;
        if (!isTransform)
        {
            Structure++;
        }
        var set = kind.Set(World, entity, value);
        if (set.IsError)
        {
            return Result.Fail(SceneError.InvalidValue(component, set.Error));
        }
        if (isTransform)
        {
            TransformWritten(id);
        }
        return Unit.Value;
    }

    /// <summary><paramref name="id"/> and all its descendants.</summary>
    private HashSet<StableId> Subtree(StableId id)
    {
        var found = new HashSet<StableId>();
        var stack = new Stack<StableId>([id]);
        while (stack.TryPop(out var next))
        {
            if (found.Add(next))
            {
                foreach (var child in Children(next))
                {
                    stack.Push(child);
                }
            }
        }
        return found;
    }

    /// <summary>Version 1 had one <c>script</c> component per object; version 2 has a <c>scripts</c> list.</summary>
    private static SceneDoc MigrateV1(SceneDoc old)
    {
        var objects = old.Objects.Select(o =>
        {
            var copy = o.Copy();
            if (copy.Components.Remove("script", out var script))
            {
                copy.Components["scripts"] = new JsonArray([script]);
            }
            return copy;
        }).ToList();
        return old with { Version = SceneDoc.CurrentVersion, Objects = objects };
    }
}

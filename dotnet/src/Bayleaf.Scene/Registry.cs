using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Ecs;
using Bayleaf.Scene.Components;

using EnvironmentComponent = Bayleaf.Scene.Components.Environment;

namespace Bayleaf.Scene;

/// <summary>One component type known to the scene, reached by name with JSON values.</summary>
public sealed class ComponentKind
{
    private Func<World, Entity, bool> has = null!;
    private Func<World, Entity, JsonValue?> get = null!;
    private Func<World, Entity, JsonValue, Result<Unit, string>> set = null!;
    private Func<World, Entity, Result<Unit, string>> addDefault = null!;
    private Func<JsonValue> defaultValue = null!;
    private Func<World, Entity, bool> remove = null!;
    private Func<string, IReadOnlyList<string>> options = null!;

    private ComponentKind()
    {
    }

    /// <summary>Key in scene files and command parameters.</summary>
    public string Name { get; private init; } = "";

    /// <summary>English label, used as the translation key.</summary>
    public string Label { get; private init; } = "";

    public bool Removable { get; private init; }

    /// <summary>False for components the generic Add Component list leaves out.</summary>
    public bool Listed { get; private init; }

    /// <summary>Where Add Component lists it: folders, then its name (<c>Light/Point Light</c>); empty for the top.</summary>
    public string Menu { get; private init; } = "";

    public static ComponentKind Of<T>()
        where T : ISceneComponent<T> => new()
    {
        Name = T.Key,
        Label = T.Label,
        Removable = T.Removable,
        Listed = T.Listed,
        Menu = T.Menu,
        has = static (world, entity) => world.Has<T>(entity),
        get = static (world, entity) => world.TryGet<T>(entity, out var value) ? value.Write().Canonical() : null,
        set = static (world, entity, value) =>
        {
            var parsed = JsonRead.Try(() => T.Read(value));
            if (parsed.IsError)
            {
                return Result.Fail(parsed.Error);
            }
            var valid = parsed.Value.Validate();
            if (valid.IsError)
            {
                return valid;
            }
            return world.Insert(entity, parsed.Value).MapError(e => e.ToString());
        },
        addDefault = static (world, entity) => world.Insert(entity, T.Initial).MapError(e => e.ToString()),
        defaultValue = static () => T.Initial.Write().Canonical(),
        remove = static (world, entity) => world.Remove<T>(entity) is { IsOk: true, Value.HasValue: true },
        options = T.Options,
    };

    /// <summary>Allowed values of a text field, empty when any text is allowed.</summary>
    public IReadOnlyList<string> Options(string field) => options(field);

    /// <summary>The value a new component of this type starts with.</summary>
    public JsonValue DefaultValue() => defaultValue();

    internal bool Has(World world, Entity entity) => has(world, entity);

    internal JsonValue? Get(World world, Entity entity) => get(world, entity);

    internal Result<Unit, string> Set(World world, Entity entity, JsonValue value) => set(world, entity, value);

    internal Result<Unit, string> AddDefault(World world, Entity entity) => addDefault(world, entity);

    internal bool Remove(World world, Entity entity) => remove(world, entity);
}

/// <summary>Every component type the scene can store, in inspector order.</summary>
public sealed class ComponentRegistry
{
    private readonly List<ComponentKind> kinds;

    private ComponentRegistry(List<ComponentKind> kinds) => this.kinds = kinds;

    public static ComponentRegistry Builtin() => new(
    [
        ComponentKind.Of<Name>(),
        ComponentKind.Of<Transform>(),
        ComponentKind.Of<Mesh>(),
        ComponentKind.Of<Sprite>(),
        ComponentKind.Of<Camera>(),
        ComponentKind.Of<DirectionalLight>(),
        ComponentKind.Of<PointLight>(),
        ComponentKind.Of<SpotLight>(),
        ComponentKind.Of<EnvironmentComponent>(),
        ComponentKind.Of<RigidBody>(),
        ComponentKind.Of<Collider>(),
        ComponentKind.Of<CharacterBody>(),
        ComponentKind.Of<Joint>(),
        ComponentKind.Of<ModelAnimation>(),
        ComponentKind.Of<KeyframeAnimation>(),
        ComponentKind.Of<FollowJoint>(),
        ComponentKind.Of<AudioSource>(),
        ComponentKind.Of<ParticleEmitter>(),
        ComponentKind.Of<GameInterface>(),
        ComponentKind.Of<StreamedArea>(),
        ComponentKind.Of<PlayerControl>(),
        ComponentKind.Of<GameRules>(),
        ComponentKind.Of<PlayerStart>(),
        ComponentKind.Of<InputMap>(),
        ComponentKind.Of<Scripts>(),
        ComponentKind.Of<Tags>(),
        ComponentKind.Of<Prefab>(),
    ]);

    public IReadOnlyList<ComponentKind> Kinds => kinds;

    public ComponentKind? Find(string name) => kinds.Find(k => k.Name == name);
}

using System.Numerics;
using System.Reflection;

using Bayleaf.Script.Sdk;

namespace Bayleaf.Script;

/// <summary>A loaded script type: how to make one, the fields its properties are, and which of the engine's calls it has.</summary>
internal sealed class ScriptType
{
    private const BindingFlags Instance = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;

    private ScriptType(Type type, ScriptDescription description, Dictionary<string, FieldInfo> fields, Action<ManyGroup, float>? many)
    {
        Type = type;
        Description = description;
        Fields = fields;
        Many = many;
        Advances = Overrides(type, nameof(GameScript.Advance), [typeof(float)]);
        Receives = Overrides(type, nameof(GameScript.Receive), [typeof(Message).MakeByRefType()]);
    }

    public Type Type { get; }

    public ScriptDescription Description { get; }

    /// <summary>Each property's field, by the property's name.</summary>
    public Dictionary<string, FieldInfo> Fields { get; }

    /// <summary>It steps per object (it overrides <see cref="GameScript.Advance"/>).</summary>
    public bool Advances { get; }

    public bool Receives { get; }

    /// <summary>Its <c>AdvanceMany</c>, when it steps all its objects in one call.</summary>
    public Action<ManyGroup, float>? Many { get; }

    public static ScriptType Of(Type type, ScriptDescription description)
    {
        var fields = new Dictionary<string, FieldInfo>(StringComparer.Ordinal);
        foreach (var field in description.Fields)
        {
            for (var t = type; t is not null && t != typeof(GameScript); t = t.BaseType)
            {
                if (t.GetField(field.Field, Instance | BindingFlags.DeclaredOnly) is { } info)
                {
                    fields[field.Declared.Name] = info;
                    break;
                }
            }
        }
        Action<ManyGroup, float>? many = null;
        if (description.StepsMany && typeof(IAdvanceMany<>).MakeGenericType(type).IsAssignableFrom(type))
        {
            many = typeof(ScriptType).GetMethod(nameof(InvokeMany), BindingFlags.Static | BindingFlags.NonPublic)!.MakeGenericMethod(type).CreateDelegate<Action<ManyGroup, float>>();
        }
        return new ScriptType(type, description, fields, many);
    }

    private static bool Overrides(Type type, string name, Type[] parameters) =>
        type.GetMethod(name, Instance, null, parameters, null) is { } method && method.GetBaseDefinition().DeclaringType == typeof(GameScript) && method.DeclaringType != typeof(GameScript);

    /// <summary>A new script of this type: its constructor runs (script code).</summary>
    public GameScript Create() => (GameScript)Activator.CreateInstance(Type)!;

    private static void InvokeMany<T>(ManyGroup group, float seconds)
        where T : GameScript, IAdvanceMany<T>
    {
        var scripts = (T[])group.Scripts;
        T.AdvanceMany(new Many<T>(new ReadOnlySpan<T>(scripts, 0, group.Count), new Span<Pose>(group.Poses, 0, group.Count)), seconds);
    }

    /// <summary>A field's value as up to three numbers, as another script reads a script's property; null for text.</summary>
    public static Vector3? Numbers(object? value) => value switch
    {
        float f => new Vector3(f, 0f, 0f),
        double d => new Vector3((float)d, 0f, 0f),
        int i => new Vector3(i, 0f, 0f),
        bool b => new Vector3(b ? 1f : 0f, 0f, 0f),
        Vector3 v => v,
        _ => null,
    };
}

/// <summary>The running objects of one script type that steps them all in one call: their scripts, where they are in the view, and their poses for the call.</summary>
internal sealed class ManyGroup(ScriptType type, Array scripts, int[] slots, int[] views)
{
    public ScriptType Type { get; } = type;

    /// <summary>The scripts, as an array of the script's own type.</summary>
    public Array Scripts { get; } = scripts;

    public int[] Slots { get; } = slots;

    /// <summary>Each one's object's place in the view.</summary>
    public int[] Views { get; } = views;

    public Pose[] Poses { get; } = new Pose[slots.Length];

    public int Count => Slots.Length;
}

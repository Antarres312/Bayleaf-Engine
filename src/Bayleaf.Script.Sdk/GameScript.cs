namespace Bayleaf.Script.Sdk;

/// <summary>
/// A game script: one class per <c>.cs</c> file, deriving from this, with a public constructor that
/// takes nothing. Every script on every object gets its own instance. Its properties are fields marked
/// <see cref="ScriptPropertyAttribute"/>: the engine sets them from the scene (else their initial
/// values) before <see cref="Prepare"/>, and again whenever they change.
/// </summary>
/// <remarks>
/// The engine calls, in order: <see cref="PropertiesChanged"/>, then <see cref="Prepare"/> of every
/// script loaded together (a scene when play starts, a placed prefab), then <see cref="Begin"/> of
/// each; then every step <see cref="Advance"/> (or <see cref="IAdvanceMany{TSelf}.AdvanceMany"/> once for all of
/// a script's objects), and <see cref="Receive"/> for each message after the step. An exception stops
/// that script for the rest of play, with its reason in the console; a step that runs too long stops
/// play mode.
/// </remarks>
public abstract class GameScript
{
    internal IWorld? World { get; set; }

    /// <summary>This script's place in the engine's host.</summary>
    internal int Slot { get; set; } = -1;

    /// <summary>Its object's place in the host's packed poses, kept by the host while the scene's structure stays.</summary>
    internal int Place { get; set; } = -1;

    /// <summary>The host's packed poses (one reference, so a script stays small: a step goes through every script).</summary>
    internal PoseStore Store { get; set; } = PoseStore.Empty;

    /// <summary>Where the pose of a script whose object is gone points: written, it goes nowhere.</summary>
    private static Pose nowhere;

    private IWorld Engine => World ?? Runtime.Need();

    /// <summary>The object this script is on.</summary>
    public SceneObject Self => Engine.Self(Slot);

    /// <summary>
    /// The object's pose relative to its parent, as this step has it: a reference into the engine's
    /// packed poses, so <c>Pose.Rotation.Y += 90 * seconds</c> turns the object, and later scripts of the
    /// step see it (<c>var pose = Pose; … Pose = pose;</c> works too). The poses that changed reach the
    /// scene when the step ends; a pose that is not finite is refused then, and the object stays where
    /// it was. Inside <see cref="IAdvanceMany{TSelf}.AdvanceMany"/> the pose given in
    /// <see cref="Many{T}.Poses"/> is the one that counts.
    /// </summary>
    public ref Pose Pose
    {
        get
        {
            var store = Store;
            var poses = store.Poses;
            var touched = store.Touched;
            var at = Place;
            if ((uint)at < (uint)poses.Length && (uint)at < (uint)touched.Length)
            {
                touched[at] = 1;
                return ref poses[at];
            }
            nowhere = Sdk.Pose.Initial;
            return ref nowhere;
        }
    }

    /// <summary>Writes to the editor console, labelled with the object and the script.</summary>
    public void Log(string text) => Engine.Log(Slot, text);

    /// <summary>Seconds since play mode started, as <see cref="Game.Time"/> gives it; the quicker of the two inside a script's own calls.</summary>
    protected double Time => Engine.Time;

    /// <summary>Its properties were set (before <see cref="Prepare"/>, and again when the Inspector or another script changes one): read what depends on them.</summary>
    protected internal virtual void PropertiesChanged()
    {
    }

    /// <summary>Once, when the object is ready; scripts loaded together all prepare before any begins.</summary>
    protected internal virtual void Prepare()
    {
    }

    /// <summary>Once, after every script loaded with it was prepared: when play mode starts, or after the scripts reload.</summary>
    protected internal virtual void Begin()
    {
    }

    /// <summary>Every simulation step.</summary>
    protected internal virtual void Advance(float seconds)
    {
    }

    /// <summary>A message to this object's scripts (or to every object's), after the step.</summary>
    protected internal virtual void Receive(in Message message)
    {
    }
}

/// <summary>
/// A script that steps all of its objects in one call instead of one call each: the engine gives the
/// scripts and their poses as two spans of the same length, and keeps each pose as the call leaves it.
/// </summary>
public interface IAdvanceMany<TSelf>
    where TSelf : GameScript, IAdvanceMany<TSelf>
{
    static abstract void AdvanceMany(Many<TSelf> objects, float seconds);
}

/// <summary>Every running object of one script type: each one's script and its pose, by the same index.</summary>
public readonly ref struct Many<T>
    where T : GameScript
{
    internal Many(ReadOnlySpan<T> scripts, Span<Pose> poses)
    {
        Scripts = scripts;
        Poses = poses;
    }

    public ReadOnlySpan<T> Scripts { get; }

    /// <summary>Each object's pose, to read and change in place.</summary>
    public Span<Pose> Poses { get; }

    public int Count => Scripts.Length;
}

/// <summary>Makes a field a property of the script: the Inspector shows it, the scene sets it.</summary>
/// <remarks>
/// A <see cref="float"/> or <see cref="double"/> field is a number, <see cref="int"/> a whole number,
/// <see cref="bool"/> true/false, <see cref="System.Numerics.Vector3"/> a vector (a colour with
/// <see cref="Color"/>), <see cref="string"/> text (or a file with <see cref="File"/>) and
/// <see cref="SceneObject"/> an object, by its name. The field's initial value, a constant, is the
/// default. The name in scene files is the field's name starting in lower case, unless given.
/// </remarks>
[AttributeUsage(AttributeTargets.Field, AllowMultiple = false)]
public sealed class ScriptPropertyAttribute : Attribute
{
    public ScriptPropertyAttribute()
    {
    }

    public ScriptPropertyAttribute(string name) => Name = name;

    /// <summary>The name in scene files; the field's name starting in lower case when left out.</summary>
    public string? Name { get; }

    public string? Label { get; set; }

    /// <summary>Said on hover and to screen readers.</summary>
    public string? Hint { get; set; }

    /// <summary>Lowest value; set with <see cref="High"/> for a range.</summary>
    public double Low { get; set; } = double.NaN;

    public double High { get; set; } = double.NaN;

    /// <summary>A slider over the range.</summary>
    public bool Slider { get; set; }

    public bool ReadOnly { get; set; }

    /// <summary>Choices for a whole number (by place, 0 first) or a text, separated by <c>|</c> (<c>"Walk|Run"</c>).</summary>
    public string? Choices { get; set; }

    /// <summary>A <see cref="System.Numerics.Vector3"/> that is a colour (0 to 1).</summary>
    public bool Color { get; set; }

    /// <summary>A <see cref="string"/> that names a file of the project: <c>prefab</c>, <c>sound</c>, <c>material</c> or <c>data</c>.</summary>
    public string? File { get; set; }
}

/// <summary>Where Add Component lists the script: 1 to 4 names separated by <c>/</c> (<c>Light/Flicker</c>); without it, under Scripts.</summary>
[AttributeUsage(AttributeTargets.Class, AllowMultiple = false)]
public sealed class ScriptMenuAttribute(string path) : Attribute
{
    public string Path { get; } = path;
}

/// <summary>An event the script declares, so the editor offers it to other scripts and the interface: its name and its values, each <c>name:type</c>.</summary>
[AttributeUsage(AttributeTargets.Class, AllowMultiple = true)]
public sealed class ScriptEventAttribute(string name, params string[] values) : Attribute
{
    public string Name { get; } = name;

    public IReadOnlyList<string> Values { get; } = values;
}

/// <summary>The host's packed poses, one per object of the scene by its place, and its marks of those scripts reached this step (it writes only those that changed). The host makes a new one when the scene's structure changes.</summary>
internal sealed class PoseStore(Pose[] poses, byte[] touched)
{
    public static readonly PoseStore Empty = new([], []);

    public Pose[] Poses { get; } = poses;

    public byte[] Touched { get; } = touched;
}

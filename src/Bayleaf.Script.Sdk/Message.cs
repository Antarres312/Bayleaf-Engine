using System.Numerics;

namespace Bayleaf.Script.Sdk;

/// <summary>What kind of value an event brings.</summary>
public enum EventValueKind
{
    Number,
    Whole,
    Flag,
    Vector,
    Text,
    SceneObject,
}

/// <summary>One value an event brings: a number, a whole number, true/false, a vector, text or an object.</summary>
public readonly struct EventValue : IEquatable<EventValue>
{
    private EventValue(EventValueKind kind, double number, Vector3 vector, string? text, SceneObject obj) =>
        (Kind, amount, triple, words, target) = (kind, number, vector, text, obj);

    private readonly double amount;
    private readonly Vector3 triple;
    private readonly string? words;
    private readonly SceneObject target;

    public EventValueKind Kind { get; }

    public static EventValue Of(float value) => new(EventValueKind.Number, value, default, null, default);

    public static EventValue Of(int value) => new(EventValueKind.Whole, value, default, null, default);

    public static EventValue Of(bool value) => new(EventValueKind.Flag, value ? 1 : 0, default, null, default);

    public static EventValue Of(Vector3 value) => new(EventValueKind.Vector, 0, value, null, default);

    public static EventValue Of(string value) => new(EventValueKind.Text, 0, default, value, default);

    public static EventValue Of(SceneObject value) => new(EventValueKind.SceneObject, 0, default, null, value);

    public static implicit operator EventValue(float value) => Of(value);

    public static implicit operator EventValue(int value) => Of(value);

    public static implicit operator EventValue(bool value) => Of(value);

    public static implicit operator EventValue(Vector3 value) => Of(value);

    public static implicit operator EventValue(string value) => Of(value);

    public static implicit operator EventValue(SceneObject value) => Of(value);

    public static EventValue FromSingle(float value) => Of(value);

    public static EventValue FromInt32(int value) => Of(value);

    public static EventValue FromBoolean(bool value) => Of(value);

    public static EventValue FromVector3(Vector3 value) => Of(value);

    public static EventValue FromString(string value) => Of(value);

    public static EventValue FromSceneObject(SceneObject value) => Of(value);

    /// <summary>As a number: a number or a whole number, 1 or 0 for true and false; 0 otherwise.</summary>
    public float Number => Kind is EventValueKind.Number or EventValueKind.Whole or EventValueKind.Flag ? (float)amount : 0f;

    public int Whole => Kind is EventValueKind.Whole or EventValueKind.Flag ? (int)amount : Kind == EventValueKind.Number ? (int)Math.Round(amount) : 0;

    public bool Flag => Kind is EventValueKind.Flag or EventValueKind.Whole or EventValueKind.Number && amount != 0;

    public Vector3 Vector => Kind == EventValueKind.Vector ? triple : default;

    public string Text => words ?? "";

    public SceneObject Target => target;

    public bool Equals(EventValue other) => Kind == other.Kind && amount.Equals(other.amount) && triple.Equals(other.triple) && words == other.words && target == other.target;

    public override bool Equals(object? obj) => obj is EventValue other && Equals(other);

    public override int GetHashCode() => HashCode.Combine(Kind, amount, triple, words, target);

    public static bool operator ==(EventValue a, EventValue b) => a.Equals(b);

    public static bool operator !=(EventValue a, EventValue b) => !a.Equals(b);
}

/// <summary>
/// A message a script hears in <see cref="GameScript.Receive"/>: its name and value, and for an event
/// the values it brings. Engine messages: <c>pressed:&lt;element&gt;</c> and <c>changed:&lt;element&gt;</c>
/// from the object's Interface, <c>gameWon</c> and <c>gameLost</c> when the game ends, and a clip's
/// marked moments by their names (with the object as the one value).
/// </summary>
public readonly struct Message(string name, float value, IReadOnlyList<EventValue> values) : IEquatable<Message>
{
    public string Name { get; } = name;

    public float Value { get; } = value;

    /// <summary>What an event brings; empty for a plain message.</summary>
    public IReadOnlyList<EventValue> Values { get; } = values;

    /// <summary>Whether the message has this name.</summary>
    public bool Is(string name) => string.Equals(Name, name, StringComparison.Ordinal);

    public bool Equals(Message other) => Name == other.Name && Value.Equals(other.Value) && ReferenceEquals(Values, other.Values);

    public override bool Equals(object? obj) => obj is Message other && Equals(other);

    public override int GetHashCode() => HashCode.Combine(Name, Value);

    public static bool operator ==(Message a, Message b) => a.Equals(b);

    public static bool operator !=(Message a, Message b) => !a.Equals(b);
}

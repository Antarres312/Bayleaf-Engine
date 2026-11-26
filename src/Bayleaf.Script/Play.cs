using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;

namespace Bayleaf.Script;

/// <summary>Why one of an object's scripts could not load or stopped, for the console.</summary>
public sealed record ScriptFault(StableId Owner, string Script, string Error);

/// <summary>A line a script wrote.</summary>
/// <param name="Error">A problem the engine reports about the script's call (something not in the C# engine yet), not the script's own text.</param>
public sealed record LogLine(StableId Owner, string Script, string Text, bool Error = false);

/// <summary>A gamepad: sticks and triggers (by <see cref="PlayInput.PadAxes"/>) and the buttons held (bit i: <see cref="PlayInput.PadButtons"/>[i]).</summary>
public sealed record Pad(IReadOnlyList<float> Axes, uint Buttons);

/// <summary>What the player holds, as scripts read it, until the next <c>play.input</c> or editor frame.</summary>
/// <param name="PointerAt">Over the game view, from (-1, -1) at the bottom left to (1, 1) at the top right; null outside it.</param>
/// <param name="Buttons">Mouse buttons held: bit 0 left, bit 1 right, bit 2 middle.</param>
/// <param name="OverInterface">The pointer is over an interface element that takes it (its clicks are the interface's).</param>
public sealed record PlayInput(IReadOnlySet<string> Keys, Vector2? PointerAt, int Buttons, bool OverInterface, Pad? Pad)
{
    public static readonly PlayInput None = new(new HashSet<string>(StringComparer.Ordinal), null, 0, false, null);

    public static readonly IReadOnlyList<string> PadAxes = ["LeftX", "LeftY", "RightX", "RightY", "LeftTrigger", "RightTrigger"];

    public static readonly IReadOnlyList<string> PadButtons =
        ["South", "East", "West", "North", "LeftBumper", "RightBumper", "LeftTrigger", "RightTrigger", "Select", "Start", "LeftStick", "RightStick", "Up", "Down", "Left", "Right"];
}

/// <summary>Whose value: the game's, a player's, or a saved one.</summary>
public enum ValueScope
{
    Game,
    Player,
    Saved,
}

/// <summary>
/// Named numbers the game keeps while it plays: the game's own, each player's (an object with Player
/// Control), the saved ones that last from one play to the next, and how the game ended. Everything
/// starts empty, except saved values the project kept.
/// </summary>
public sealed class GameState
{
    /// <summary>Most values in the game's state, in each player's, and saved.</summary>
    public const int MaxValues = 1024;

    public SortedDictionary<string, double> Game { get; } = new(Utf8Order.Instance);

    public SortedDictionary<StableId, SortedDictionary<string, double>> Players { get; } = [];

    public SortedDictionary<string, double> Saved { get; } = new(Utf8Order.Instance);

    /// <summary>True won, false lost; the first ending counts.</summary>
    public bool? Outcome { get; private set; }

    /// <summary>Whether a name and number may be kept: a name of 1 to 64 bytes and a finite number.</summary>
    public static bool Valid(string name, double value) => name.Length > 0 && Text.Bytes(name) <= 64 && double.IsFinite(value);

    /// <summary>Sets a value; false when the name is not 1 to 64 bytes, the number is not finite, or the state is full.</summary>
    public bool Set(ValueScope kind, StableId player, string name, double value)
    {
        if (!Valid(name, value))
        {
            return false;
        }
        var map = Map(kind, player, true)!;
        if (map.Count >= MaxValues && !map.ContainsKey(name))
        {
            return false;
        }
        map[name] = value;
        return true;
    }

    public double? Get(ValueScope kind, StableId player, string name) => Map(kind, player, false) is { } map && map.TryGetValue(name, out var v) ? v : null;

    /// <summary>Ends the game, unless it already ended; true when this call ended it.</summary>
    public bool End(bool won)
    {
        if (Outcome is not null)
        {
            return false;
        }
        Outcome = won;
        return true;
    }

    /// <summary>A copy, for a scene loaded in place of the playing one.</summary>
    public GameState Copy()
    {
        var copy = new GameState { Outcome = Outcome };
        foreach (var (k, v) in Game)
        {
            copy.Game[k] = v;
        }
        foreach (var (k, v) in Saved)
        {
            copy.Saved[k] = v;
        }
        foreach (var (id, values) in Players)
        {
            copy.Players[id] = new SortedDictionary<string, double>(values, Utf8Order.Instance);
        }
        return copy;
    }

    private SortedDictionary<string, double>? Map(ValueScope kind, StableId player, bool make)
    {
        switch (kind)
        {
            case ValueScope.Game:
                return Game;
            case ValueScope.Saved:
                return Saved;
            default:
                if (!Players.TryGetValue(player, out var map) && make)
                {
                    map = new SortedDictionary<string, double>(Utf8Order.Instance);
                    Players[player] = map;
                }
                return map;
        }
    }
}

/// <summary>What a script asks of an object's animation, done after the step as the <c>animation.*</c> commands do it.</summary>
public abstract record AnimationAction
{
    private AnimationAction()
    {
    }

    /// <summary>Play a clip on the whole skeleton, fading from what played.</summary>
    public sealed record Play(string Clip, float Fade, bool Looping) : AnimationAction;

    /// <summary>Fade to the rest pose.</summary>
    public sealed record Rest(float Fade) : AnimationAction;

    public sealed record Number(string Name, float Value) : AnimationAction;

    public sealed record Flag(string Name, bool Value) : AnimationAction;

    public sealed record Trigger(string Name) : AnimationAction;
}

/// <summary>What a script changes in an Interface element while the game plays.</summary>
public abstract record InterfaceChange
{
    private InterfaceChange()
    {
    }

    public sealed record Text(string Content) : InterfaceChange;

    public sealed record Value(double Number) : InterfaceChange;

    public sealed record Visible(bool Shown) : InterfaceChange;

    /// <summary>A new element named as the request's element, into the element named <paramref name="Parent"/> ("" for the root): a kind or a layout file used as a part.</summary>
    public sealed record Add(string Parent, string What) : InterfaceChange;

    public sealed record Remove : InterfaceChange;
}

/// <summary>Something a script asked for that the play loop does after the step, in order. <c>By</c> is the asking script's object.</summary>
public abstract record Request(StableId By)
{
    /// <summary>Place a copy of a prefab file at a world position.</summary>
    public sealed record Spawn(StableId By, string Prefab, Double3 Position) : Request(By);

    /// <summary>Delete an object and its children.</summary>
    public sealed record Destroy(StableId By, StableId Target) : Request(By);

    public sealed record Animation(StableId By, StableId Target, AnimationAction Action) : Request(By);

    /// <summary>Change an element of the object's Interface (an empty element: the whole interface).</summary>
    public sealed record InterfaceEdit(StableId By, StableId Target, string Element, InterfaceChange Change) : Request(By);

    /// <summary>Play the object's Keyframe Animation from its start, or stop it where it is.</summary>
    public sealed record Keyframes(StableId By, StableId Target, bool Play) : Request(By);

    /// <summary>Load a scene file: in place of the playing one, or added to it.</summary>
    public sealed record LoadScene(StableId By, string Path, bool Additive) : Request(By);

    public sealed record UnloadScene(StableId By, string Path) : Request(By);

    /// <summary>Set a field of a component: a number takes X, true/false whether X is not 0, a vector or colour its first parts.</summary>
    public sealed record SetField(StableId By, StableId Target, string Component, string Field, Vector3 Value) : Request(By);
}

/// <summary>One animated object as scripts read it: the animation state it is in, and its joints' places in the world, by name.</summary>
public sealed record AnimatedView(string? State, IReadOnlyList<string> Names, IReadOnlyList<(Double3 Position, Vector3 Rotation, Vector3 Scale)> Joints);

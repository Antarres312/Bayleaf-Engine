using System.Numerics;

namespace Bayleaf.Script.Sdk;

/// <summary>
/// The playing game: its time, its objects, what scripts ask of it (spawning, scenes, messages to every
/// object), its state (the game's values, each player's, saved ones) and the project's data files.
/// Calls are about the script whose code runs now.
/// </summary>
public static class Game
{
    /// <summary>Seconds since play mode started.</summary>
    public static double Time => Runtime.Need().Time;

    /// <summary>Writes to the editor console, labelled with the object and the script.</summary>
    public static void Log(string text) => Runtime.Need().Log(Runtime.Need().Current, text);

    /// <summary>The first object with this name, in hierarchy order, or <see cref="SceneObject.None"/>.</summary>
    public static SceneObject Find(string name) => Runtime.Need().Find(name);

    /// <summary>
    /// The <paramref name="index"/>-th object (from 0, in hierarchy order) that has a component (by its
    /// name or label), a tag, or runs a script (by its file name or the name the Inspector shows);
    /// case, spaces, <c>_</c> and <c>-</c> are ignored.
    /// </summary>
    public static SceneObject FindWith(string kind, int index = 0) => Runtime.Need().FindWith(kind, index);

    /// <summary>How many objects <see cref="FindWith"/> can find for that kind.</summary>
    public static int CountWith(string kind) => Runtime.Need().CountWith(kind);

    /// <summary>Places a copy of a prefab file (relative to the project folder) at a world position after the step; its scripts start then.</summary>
    public static bool Spawn(string prefab, Vector3d position) => Runtime.Need().Spawn(Runtime.Need().Current, prefab, position);

    /// <summary>Loads a scene file after the step: in place of the playing one (scripts start again, the game's state stays), or added to it.</summary>
    public static bool LoadScene(string path, bool add = false) => Runtime.Need().LoadScene(Runtime.Need().Current, path, add);

    /// <summary>Removes, after the step, the objects of the scenes added from that file while playing.</summary>
    public static bool UnloadScene(string path) => Runtime.Need().UnloadScene(Runtime.Need().Current, path);

    /// <summary>Sends every object's scripts a message, delivered after every script's step.</summary>
    public static bool SendToAll(string name, float value = 0f) => Runtime.Need().Send(Runtime.Need().Current, null, name, value, []);

    /// <summary>Triggers an event on every object.</summary>
    public static bool TriggerOnAll(string name, params EventValue[] values) => Runtime.Need().Send(Runtime.Need().Current, null, name, 0f, values);

    /// <summary>A value of the game's state, or null when it is not set. The state starts empty when play starts.</summary>
    public static double? Value(string name) => Runtime.Need().State(Runtime.Need().Current, StateScope.Game, 0, name);

    /// <summary>Sets a value of the game's state at once, seen by every script; false for an empty name, a number that is not finite or a full state.</summary>
    public static bool SetValue(string name, double value) => Runtime.Need().SetState(Runtime.Need().Current, StateScope.Game, 0, name, value);

    /// <summary>A value of a player's state (an object with Player Control; <see cref="SceneObject.None"/> for the first player).</summary>
    public static double? PlayerValue(SceneObject player, string name) => Runtime.Need().State(Runtime.Need().Current, StateScope.Player, player.Id, name);

    public static bool SetPlayerValue(SceneObject player, string name, double value) => Runtime.Need().SetState(Runtime.Need().Current, StateScope.Player, player.Id, name, value);

    /// <summary>A saved value: kept between plays in the project's <c>saves/progress.json</c>.</summary>
    public static double? SavedValue(string name) => Runtime.Need().State(Runtime.Need().Current, StateScope.Saved, 0, name);

    /// <summary>Sets a saved value; it is written to the project when play stops or the game ends.</summary>
    public static bool SetSavedValue(string name, double value) => Runtime.Need().SetState(Runtime.Need().Current, StateScope.Saved, 0, name, value);

    /// <summary>Ends the game, won or lost; true when this ended it. Every script then hears <c>gameWon</c> or <c>gameLost</c>.</summary>
    public static bool End(bool won) => Runtime.Need().EndGame(won);

    /// <summary>A data file of the project (<c>.bdata</c>, by its path from the project folder), as it was saved when play started.</summary>
    public static DataFile Data(string path) => new(path);

    /// <summary>Plays a sound file once from a world position. Sound comes with a later stage: false for now.</summary>
    public static bool PlaySoundAt(string path, Vector3d position, float volume = 1f) => Runtime.Need().PlaySoundAt(Runtime.Need().Current, path, position, volume);

    /// <summary>The first object a ray hits within <paramref name="distance"/>. Rays come with physics in a later stage: null for now.</summary>
    public static RayHit? Raycast(Vector3d origin, Vector3 direction, double distance = 1000) => Runtime.Need().Raycast(Runtime.Need().Current, origin, direction, distance);
}

/// <summary>The keyboard, mouse and gamepad as the game reads them, and the input actions of the Input Maps.</summary>
public static class Controls
{
    /// <summary>Whether the key is held (names like <c>W</c>, <c>Space</c>, <c>ArrowUp</c>).</summary>
    public static bool KeyDown(string key) => Runtime.Need().KeyDown(key);

    /// <summary>The pointer over the game view, from -1 to 1 (x right, y up); null when it is outside.</summary>
    public static Vector2? PointerPosition => Runtime.Need().Pointer;

    /// <summary>Whether a mouse button is held: 0 left, 1 right, 2 middle.</summary>
    public static bool MouseDown(int button) => Runtime.Need().MouseDown(button);

    /// <summary>Whether the pointer is over an interface element that takes it (its clicks are the interface's then).</summary>
    public static bool PointerOverInterface => Runtime.Need().PointerOverInterface;

    public static bool PadConnected => Runtime.Need().PadConnected;

    /// <summary>0 left stick x, 1 left stick y, 2 right stick x, 3 right stick y (-1 to 1), 4 left trigger, 5 right trigger (0 to 1).</summary>
    public static float PadAxis(int index) => Runtime.Need().PadAxis(index);

    /// <summary>0 South, 1 East, 2 West, 3 North, 4 LeftBumper … 15 Right.</summary>
    public static bool PadButton(int index) => Runtime.Need().PadButton(index);

    /// <summary>
    /// An input action's value by its kind: a trigger 1 in the step it goes down, held 1 while down, a
    /// value how far it is pressed (0 to 1). <c>Jump</c> in the active map or <c>Player/Jump</c>; the
    /// script's object's own Input Map answers before the project's.
    /// </summary>
    public static float Action(string name) => Runtime.Need().Action(Runtime.Need().Current, name);

    /// <summary>Whether the action is down now (more than half pressed, or its trigger fired).</summary>
    public static bool ActionHeld(string name) => Action(name) > 0.5f;

    /// <summary>Makes a map of the project's Input Map active.</summary>
    public static bool SetInputMap(string map) => Runtime.Need().SetInputMap(Runtime.Need().Current, 0, map);
}

/// <summary>One value of a data file's field: up to three numbers, or text.</summary>
internal readonly record struct DataValue(Vector3 Numbers, string? Text);

/// <summary>
/// A data file as play read it when it started: each field by name, the shape's default where the file
/// sets none. A file the project lacks, or a field its shape lacks, reads as null (counts as -1).
/// </summary>
public readonly struct DataFile(string path) : IEquatable<DataFile>
{
    public string Path { get; } = path;

    public float? Number(string field) => Runtime.Need().Data(Path, field, -1, null)?.Numbers.X;

    public int? Whole(string field) => Number(field) is { } n ? (int)MathF.Round(n) : null;

    public bool? Flag(string field) => Number(field) is { } n ? n != 0 : null;

    public Vector3? Vector(string field) => Runtime.Need().Data(Path, field, -1, null)?.Numbers;

    /// <summary>A text field (text, an object's name, a file's path).</summary>
    public string? Text(string field) => Runtime.Need().Data(Path, field, -1, null)?.Text;

    /// <summary>How many items a list field has, or entries a map field; -1 for none.</summary>
    public int Count(string field) => Runtime.Need().DataCount(Path, field);

    public float? NumberAt(string field, int index) => Runtime.Need().Data(Path, field, index, null)?.Numbers.X;

    public string? TextAt(string field, int index) => Runtime.Need().Data(Path, field, index, null)?.Text;

    /// <summary>The <paramref name="index"/>-th key (in order) of a map field, as the scene writes keys.</summary>
    public string? KeyAt(string field, int index) => Runtime.Need().DataKey(Path, field, index);

    public float? NumberFor(string field, string key) => Runtime.Need().Data(Path, field, -1, key)?.Numbers.X;

    public string? TextFor(string field, string key) => Runtime.Need().Data(Path, field, -1, key)?.Text;

    public bool Equals(DataFile other) => Path == other.Path;

    public override bool Equals(object? obj) => obj is DataFile other && Equals(other);

    public override int GetHashCode() => Path?.GetHashCode(StringComparison.Ordinal) ?? 0;

    public static bool operator ==(DataFile a, DataFile b) => a.Equals(b);

    public static bool operator !=(DataFile a, DataFile b) => !a.Equals(b);
}

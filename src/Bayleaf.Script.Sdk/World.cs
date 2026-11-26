using System.Numerics;

namespace Bayleaf.Script.Sdk;

/// <summary>What a script asks of an object that is done after the step.</summary>
internal enum Ask
{
    Destroy,
    PlayKeyframes,
    StopKeyframes,
    PlaySound,
    StopSound,
    PlayParticles,
    StopParticles,
}

/// <summary>What a script asks of an object's animation, done after the step.</summary>
internal enum AnimationAsk
{
    Play,
    Stop,
    Number,
    Flag,
    Trigger,
}

/// <summary>What a script changes in an object's Interface, done after the step.</summary>
internal enum InterfaceAsk
{
    Text,
    Value,
    Show,
    Add,
    Remove,
}

/// <summary>What a script asks of an object's physics body or Character, done after the step (physics comes with stage 5).</summary>
internal enum BodyAsk
{
    Push,
    Velocity,
    Walk,
    Jump,
}

/// <summary>Whose value of the game's state: the game's, a player's or a saved one.</summary>
internal enum StateScope
{
    Game,
    Player,
    Saved,
}

/// <summary>
/// The engine's side of a script's calls (the script host in Bayleaf.Script implements it). A script
/// reaches it through <see cref="GameScript"/>, <see cref="SceneObject"/>, <see cref="Game"/> and
/// <see cref="Controls"/>; <c>slot</c> is the calling script's place in the host. Every request is
/// checked here and done after the step, as <c>docs/script-abi.md</c> says.
/// </summary>
internal interface IWorld
{
    double Time { get; }

    /// <summary>The slot of the script whose code runs now (for <see cref="Game"/>'s calls).</summary>
    int Current { get; }

    void Log(int slot, string text);

    SceneObject Self(int slot);

    Pose OwnPose(int slot);

    void SetOwnPose(int slot, in Pose pose);

    bool Exists(UInt128 id);

    string? NameOf(UInt128 id);

    Pose? PoseOf(UInt128 id);

    bool SetPoseOf(int slot, UInt128 id, in Pose pose);

    SceneObject Find(string name);

    SceneObject FindWith(string kind, int index);

    int CountWith(string kind);

    bool HasComponent(UInt128 id, string component);

    Vector3? Field(UInt128 id, string component, string field);

    bool SetField(int slot, UInt128 id, string component, string field, Vector3 value);

    bool Send(int slot, UInt128? to, string name, float value, IReadOnlyList<EventValue> values);

    bool Ask(int slot, UInt128 id, Ask ask, int count);

    bool Animation(int slot, UInt128 id, AnimationAsk ask, string name, float value, bool flag);

    string? AnimationState(UInt128 id);

    Pose? Joint(UInt128 id, string joint);

    bool Interface(int slot, UInt128 id, InterfaceAsk ask, string element, string text, double value);

    double? InterfaceValue(UInt128 id, string element);

    string? InterfaceText(UInt128 id, string element);

    bool Body(int slot, UInt128 id, BodyAsk ask, Vector3 value);

    Vector3? Velocity(UInt128 id);

    bool Grounded(UInt128 id);

    bool Spawn(int slot, string prefab, Vector3d at);

    bool LoadScene(int slot, string path, bool add);

    bool UnloadScene(int slot, string path);

    bool PlaySoundAt(int slot, string path, Vector3d at, float volume);

    RayHit? Raycast(int slot, Vector3d origin, Vector3 direction, double distance);

    double? State(int slot, StateScope scope, UInt128 player, string name);

    bool SetState(int slot, StateScope scope, UInt128 player, string name, double value);

    bool EndGame(bool won);

    bool KeyDown(string key);

    Vector2? Pointer { get; }

    bool MouseDown(int button);

    bool PointerOverInterface { get; }

    bool PadConnected { get; }

    float PadAxis(int index);

    bool PadButton(int index);

    float Action(int slot, string name);

    bool SetInputMap(int slot, UInt128 id, string map);

    DataValue? Data(string file, string field, int index, string? key);

    int DataCount(string file, string field);

    string? DataKey(string file, string field, int index);
}

/// <summary>Where the engine is for the code running now; set by the host around each call into scripts.</summary>
internal static class Runtime
{
    [ThreadStatic]
    private static IWorld? world;

    internal static IWorld? World
    {
        get => world;
        set => world = value;
    }

    /// <summary>The engine, or an error that says scripts reach it only while the game plays.</summary>
    internal static IWorld Need() => world ?? throw new InvalidOperationException("a script reaches the game only while the engine runs it (in play mode, on the engine's script thread)");
}

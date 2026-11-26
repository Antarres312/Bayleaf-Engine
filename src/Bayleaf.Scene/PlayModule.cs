using Bayleaf.Core;

using Transform = Bayleaf.Scene.Components.Transform;

namespace Bayleaf.Scene;

/// <summary>
/// What play mode lets an opt-in module do (docs/csharp-redesign.md, "Opt-in modules"): read and change
/// the playing scene, place prefabs and add scenes as play mode does (their scripts start), read the
/// project's files through the session's confinement, end the game and write to the console. The
/// engine gives it to each <see cref="IPlayModule"/> call; it is good only during that call.
/// </summary>
public interface IPlayWorld
{
    /// <summary>The playing scene: a copy of the edited one, put back when play stops.</summary>
    Scene Scene { get; }

    /// <summary>Seconds since play mode started.</summary>
    double Time { get; }

    void Log(bool problem, string text);

    /// <summary>Places a linked copy of a prefab (relative to the project folder) at the top of the scene, <paramref name="place"/> setting its transform, and starts its scripts.</summary>
    Result<StableId, string> Place(string prefab, Func<Transform, Transform> place);

    /// <summary>Adds a scene's objects under <paramref name="parent"/> (or at the top) as play mode adds a loaded scene: their scripts start. Returns the new top objects.</summary>
    Result<IReadOnlyList<StableId>, string> AddScene(Scene scene, StableId? parent);

    /// <summary>The full path of a file of the project (relative to the project folder) that may be read, or why not.</summary>
    Result<string, string> Readable(string relative);

    /// <summary>Ends the game, won or lost; false when it had already ended.</summary>
    bool EndGame(bool won);
}

/// <summary>
/// An opt-in part of play mode: gameplay roles, world streaming, and later physics and sound. The
/// engine does not refer to any; an app turns one on by giving it to the session
/// (<c>session.Use(new StreamingModule())</c>), and play mode calls the modules in the order given.
/// </summary>
public interface IPlayModule
{
    /// <summary>A short name for messages, such as "world streaming".</summary>
    string Name { get; }

    /// <summary>Play mode started, or a scene was loaded in place of the playing one.</summary>
    void Started(IPlayWorld world);

    /// <summary>One play step, after the scripts ran and what they asked for was done.</summary>
    void Stepped(IPlayWorld world, float seconds);

    /// <summary>Play mode stopped; the module forgets the playing scene.</summary>
    void Stopped();
}

/// <summary>One Streamed Area that is not away, as <c>stream.areas</c> lists it.</summary>
/// <param name="State">waiting, reading, here or failed.</param>
public sealed record StreamedState(StableId Area, string Scene, string State, IReadOnlyList<StableId> Objects, string? Problem);

/// <summary>The world-streaming module, which <c>stream.areas</c> and <c>stream.wait</c> ask.</summary>
public interface IStreamingModule : IPlayModule
{
    /// <summary>Every Streamed Area that is not away, in id order.</summary>
    IReadOnlyList<StreamedState> Areas();

    /// <summary>Waits at most <paramref name="timeout"/> until no area is being read, adds those that arrived, and says how many are here and how many still busy.</summary>
    (int Here, int Busy) Wait(IPlayWorld world, TimeSpan timeout);
}

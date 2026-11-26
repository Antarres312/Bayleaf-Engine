using Bayleaf.Scene;
using Bayleaf.Scene.Components;

namespace Bayleaf.Gameplay;

/// <summary>
/// The gameplay roles in play mode: Game Rules place the player prefab at the Player Start when play
/// starts, put fallen players back there, and end the game when the time limit runs out; Player
/// Control's following camera trails the player. Steering the player's Character needs physics, which
/// comes with stage 5 of the C# port. An app turns it on with <c>session.Use(new GameplayModule())</c>.
/// </summary>
public sealed class GameplayModule : IPlayModule
{
    public string Name => "gameplay roles";

    public void Started(IPlayWorld world)
    {
        var scene = world.Scene;
        if (Rules.Of(scene) is not { } rules || rules.PlayerPrefab.Length == 0 || Rules.Players(scene).Count > 0)
        {
            return;
        }
        var start = Transform.FromMatrix(Rules.Start(scene));
        var placed = world.Place(rules.PlayerPrefab, t => start with { Scale = t.Scale });
        if (placed.IsError)
        {
            world.Log(true, $"Game Rules could not place the player {rules.PlayerPrefab}: {placed.Error}");
        }
    }

    public void Stepped(IPlayWorld world, float seconds)
    {
        var scene = world.Scene;
        // The camera follows after the player moved.
        if (Controller.FollowCamera(scene) is var (camera, transform))
        {
            scene.Set(camera, transform);
        }
        if (Rules.Of(scene) is not { } rules)
        {
            return;
        }
        foreach (var id in Rules.Fallen(scene, rules))
        {
            if (Rules.PlacedAtStart(scene, id) is { } placed)
            {
                scene.Set(id, placed);
                world.Log(false, $"[{scene.NameOf(id) ?? "Player"}] fell and starts over at the Player Start");
            }
        }
        if (Rules.TimeUp(rules, world.Time))
        {
            world.EndGame(rules.TimeUp == TimeUp.Win);
        }
    }

    public void Stopped()
    {
    }
}

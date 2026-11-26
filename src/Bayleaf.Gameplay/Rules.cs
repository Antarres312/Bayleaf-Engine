using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Scene.Components;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Gameplay;

/// <summary>Game Rules: who the player is, where it appears and starts over, and how long the game lasts.</summary>
public static class Rules
{
    /// <summary>The scene's Game Rules: those of the first object in the hierarchy that has them.</summary>
    public static GameRules? Of(SceneModel scene)
    {
        foreach (var (id, _) in scene.WorldMatrices())
        {
            if (scene.TryGet<GameRules>(id, out var rules))
            {
                return rules;
            }
        }
        return null;
    }

    /// <summary>Objects with Player Control, in hierarchy order.</summary>
    public static List<StableId> Players(SceneModel scene) => [.. scene.WorldMatrices().Select(m => m.Id).Where(id => scene.TryGet<PlayerControl>(id, out _))];

    /// <summary>The first Player Start's world matrix, else the world's centre.</summary>
    public static DMat4 Start(SceneModel scene)
    {
        foreach (var (id, matrix) in scene.WorldMatrices())
        {
            if (scene.TryGet<PlayerStart>(id, out _))
            {
                return matrix;
            }
        }
        return DMat4.Identity;
    }

    /// <summary>The local transform that puts <paramref name="player"/> at the Player Start, keeping its own scale.</summary>
    public static Transform? PlacedAtStart(SceneModel scene, StableId player)
    {
        var parent = scene.Parent(player) is { } p && scene.WorldMatrix(p) is { } m ? m : DMat4.Identity;
        if (!scene.TryGet<Transform>(player, out var own))
        {
            return null;
        }
        var placed = Transform.FromMatrix(parent.Inverse() * Start(scene)) with { Scale = own.Scale };
        return placed.IsFinite ? placed : null;
    }

    /// <summary>Players below the respawn height, when the rules respawn them.</summary>
    public static List<StableId> Fallen(SceneModel scene, GameRules rules) =>
        rules.Respawn ? [.. Players(scene).Where(id => scene.WorldMatrix(id) is { } m && m.WAxis.Y < rules.RespawnHeight)] : [];

    /// <summary>Whether the time limit has run out after <paramref name="seconds"/> of play.</summary>
    public static bool TimeUp(GameRules rules, double seconds) => rules.TimeLimit > 0f && seconds >= rules.TimeLimit;
}

using System.Numerics;

using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Gameplay.Tests;

public class GameplayTests
{
    private static JsonValue Json(string text) => JsonReader.Parse(text).Value;

    private static (SceneModel Scene, Core.StableId Hero) SceneWithCamera(float cameraRotationY)
    {
        var scene = new SceneModel();
        var hero = scene.CreateObject(null, "Hero").Value;
        Assert.True(scene.AddComponent(hero, "playerControl").IsOk);
        var camera = scene.CreateObject(null, "Camera").Value;
        Assert.True(scene.AddComponent(camera, "camera").IsOk);
        Assert.True(scene.SetComponent(camera, "camera", Json("""{"projection": "perspective", "fov": 60, "size": 5, "near": 0.1, "far": 100}""")).IsOk);
        Assert.True(scene.Set(camera, new Transform(new Double3(0, 3, 6), new Vector3(0, cameraRotationY, 0), Vector3.One)).IsOk);
        return (scene, hero);
    }

    [Fact]
    public void ForwardFollowsTheCameraAndTheHeroTurnsToWalk()
    {
        var (scene, hero) = SceneWithCamera(0f);
        static float Held(Core.StableId _, string name) => name == "MoveForward" ? 1f : 0f;
        var commands = Controller.Drive(scene, Held, (_, _) => false);
        var only = Assert.Single(commands);
        Assert.Equal(hero, only.Target);
        Assert.True(only.Walk.Z < -4.9f, "the camera looks down -Z, so forward is -Z at 5 m/s");
        Assert.True(Math.Abs(only.Facing!.Value) < 1e-3f, "facing -Z is no turn");
        Assert.Null(only.Jump);

        // The camera turned 90° to the left looks down -X; forward goes there too.
        (scene, _) = SceneWithCamera(90f);
        var turned = Controller.Drive(scene, Held, (_, name) => name == "Jump")[0];
        Assert.True(turned.Walk.X < -4.9f);
        Assert.Equal(5f, turned.Jump);
        var w = Controller.Drive(scene, (_, name) => name is "MoveForward" or "MoveRight" ? 1f : 0f, (_, _) => false)[0].Walk;
        Assert.True(Math.Abs(MathF.Sqrt((w.X * w.X) + (w.Z * w.Z)) - 5f) < 1e-3f, "diagonals are not faster");
    }

    [Fact]
    public void TheCameraTrailsTheHeroAndLooksAtIt()
    {
        var (scene, hero) = SceneWithCamera(0f);
        Assert.True(scene.Set(hero, new Transform(new Double3(10, 0, -20), Vector3.Zero, Vector3.One)).IsOk);
        var (camera, placed) = Controller.FollowCamera(scene)!.Value;
        Assert.True(scene.Set(camera, placed).IsOk);
        var eye = placed.Position;
        var heroAt = new Double3(10, 0, -20);
        var flat = new Double3(eye.X - heroAt.X, 0, eye.Z - heroAt.Z).Length;
        Assert.True(Math.Abs(flat - 6) < 1e-6 && Math.Abs(eye.Y - 3) < 1e-6, "6 m away and 3 m up");
        var looks = scene.WorldMatrix(camera)!.Value.TransformVector3(new Double3(0, 0, -1));
        var toHero = heroAt - eye;
        Assert.True(looks.Dot(toHero) / (looks.Length * toHero.Length) > 0.999, "it looks at the hero");
    }

    [Fact]
    public void PlayersStartAtThePlayerStartAndFallenOnesAreFound()
    {
        var scene = new SceneModel();
        Assert.Null(Rules.Of(scene));
        var referee = scene.CreateObject(null, "Rules").Value;
        Assert.True(scene.AddComponent(referee, "gameRules").IsOk);
        Assert.True(scene.SetComponent(referee, "gameRules", Json("""{"playerPrefab": "", "respawn": true, "respawnHeight": -10, "timeLimit": 30, "timeUp": "win"}""")).IsOk);
        var start = scene.CreateObject(null, "Start").Value;
        Assert.True(scene.AddComponent(start, "playerStart").IsOk);
        Assert.True(scene.Set(start, new Transform(new Double3(5, 2, -3), new Vector3(0, 90, 0), Vector3.One)).IsOk);
        var hero = scene.CreateObject(null, "Hero").Value;
        Assert.True(scene.AddComponent(hero, "playerControl").IsOk);
        Assert.True(scene.Set(hero, new Transform(new Double3(0, -50, 0), Vector3.Zero, new Vector3(2))).IsOk);

        var rules = Rules.Of(scene)!;
        Assert.Equal([hero], Rules.Fallen(scene, rules));
        var placed = Rules.PlacedAtStart(scene, hero)!.Value;
        Assert.True((placed.Position - new Double3(5, 2, -3)).Length < 1e-9);
        Assert.True(Math.Abs(placed.Rotation.Y - 90f) < 1e-3f, "it faces the way the start faces");
        Assert.Equal(new Vector3(2), placed.Scale);
        Assert.True(!Rules.TimeUp(rules, 29.9) && Rules.TimeUp(rules, 30.0));
    }
}

using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Engine.Tests;

/// <summary>
/// The <c>animation.*</c> commands outside play mode, on the generated arm model (Root, Arm under
/// it, Hand under Arm; Stretch moves Arm 1.5 along X, Raise moves it 1 up). The Rust engine's tests
/// that need play mode (a Model Animation playing when play starts, animation states, clip events,
/// root motion and scripts) come with the play-mode port; the fade and mix test runs here on the
/// editor's preview clock (<see cref="Session.Animate"/>) instead of play steps.
/// </summary>
public sealed class AnimationTests : IDisposable
{
    private readonly string dir = Path.Join(Path.GetTempPath(), $"bayleaf-animation-cs-{Guid.NewGuid():N}");
    private readonly CommandRegistry registry = CommandRegistry.WithBuiltins();
    private readonly Session session = new();
    private readonly string id;

    /// <summary>A saved scene with one object showing the arm model, with a Model Animation playing Stretch once.</summary>
    public AnimationTests()
    {
        Directory.CreateDirectory(dir);
        Assert.True(Assets.Fixtures.SkinnedArm(dir).IsOk);
        var mesh = """{"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "model": "arm.gltf"}""";
        var animation = """{"animation": "Stretch", "playing": true, "looping": false, "speed": 1}""";
        id = Run("scene.object.create", $$$"""{"name": "Arm", "components": {"mesh": {{{mesh}}}, "modelAnimation": {{{animation}}}}}""").Get("id")!.AsString()!;
        Run("scene.saveAs", $$"""{"path": {{new JsonString(Path.Join(dir, "arm.bscene")).ToCompact()}}}""");
    }

    public void Dispose() => Directory.Delete(dir, true);

    private Result<JsonValue, CommandError> Try(string command, string parameters) => registry.Execute(session, command, JsonReader.Parse(parameters).Value);

    private JsonValue Run(string command, string parameters)
    {
        var result = Try(command, parameters);
        Assert.True(result.IsOk, result.IsOk ? "" : $"{command}: {result.Error}");
        return result.Value;
    }

    private StableId Object => StableId.ParseHex(id).Value;

    /// <summary>Where the session's pose puts the Arm joint in the model.</summary>
    private Vector3 ArmJoint() => session.AnimationPoses[Object][1].Translation;

    [Fact]
    public void APreviewScrubsAndPlaysAClipOutsidePlayModeWithoutChangingTheScene()
    {
        var before = session.Scene.ToDoc();
        Run("animation.preview", $$"""{"id": "{{id}}", "clip": "Stretch", "time": 0.5, "playing": false}""");
        var scrubbed = ArmJoint().X;
        Assert.True(Math.Abs(scrubbed - 0.75f) < 1e-3f, $"half way through Stretch: {scrubbed}");
        session.Animate(1f);
        Assert.True(Math.Abs(ArmJoint().X - scrubbed) < 1e-6f && !session.AnimationPreviewing, "paused, it holds");
        Run("animation.preview", $$"""{"id": "{{id}}", "clip": "Stretch", "playing": true}""");
        Assert.True(session.AnimationPreviewing);
        session.Animate(0.25f);
        Assert.True(ArmJoint().X > scrubbed + 0.2f, "playing, it moves on");
        var shown = session.AnimationPreview(Object);
        Assert.Equal(("Stretch", true), (shown?.Clip, shown?.Playing ?? false));
        Assert.True(Try("animation.preview", $$"""{"id": "{{id}}", "clip": "Walk"}""").IsError);
        Run("animation.preview.stop", $$"""{"id": "{{id}}"}""");
        Assert.True(session.AnimationPoses.Count == 0 && session.AnimationPreview(Object) is null);
        Assert.Equal(before, session.Scene.ToDoc());
        Assert.False(session.Dirty, "a preview never marks the scene changed");
    }

    [Fact]
    public void ClipsFadeAndMixOnAnObject()
    {
        Run("animation.play", $$"""{"id": "{{id}}", "clip": "Stretch", "fade": 0, "loop": false}""");
        session.Animate(1f);
        Assert.True(ArmJoint().X > 1.4f, $"Stretch played to its end: {ArmJoint()}");
        var hand = Run("animation.joint.get", $$"""{"id": "{{id}}", "joint": "Hand"}""");
        var x = hand.Get("position")!.AsArray()![0].AsDouble();
        Assert.True(x > 2.4, $"the Hand joint is a unit past the stretched Arm, in the world: {hand.ToCompact()}");
        Assert.True(Try("animation.joint.get", $$"""{"id": "{{id}}", "joint": "Elbow"}""").IsError);

        Run("animation.play", $$"""{"id": "{{id}}", "clip": "Raise", "fade": 1, "loop": false}""");
        session.Animate(0.5f);
        var halfway = ArmJoint();
        Assert.True(halfway.X > 0.1f && halfway.X < 1.4f && halfway.Y > 0.05f && halfway.Y < 0.5f, $"half way through the fade: {halfway}");
        var state = Run("animation.get", $$"""{"id": "{{id}}"}""");
        var layer = state.Get("layers")!.AsArray()![0];
        Assert.Equal(JsonValue.From(true), layer.Get("fading"));
        Assert.Equal("Raise", layer.Get("motion")!.Get("clip")!.AsString());
        Assert.Contains(((JsonArray)state.Get("joints")!).Select(j => j.AsString()), j => j == "Hand");
        Assert.Equal(3, ((JsonArray)state.Get("clips")!).Count);

        Run("animation.play", $$"""{"id": "{{id}}", "mix": [{"clip": "Stretch", "at": 0}, {"clip": "Raise", "at": 1}], "by": "aim", "fade": 0, "loop": false}""");
        Run("animation.parameter.set", $$"""{"id": "{{id}}", "name": "aim", "value": 0.75}""");
        session.Animate(1f);
        var mixed = ArmJoint();
        Assert.True(mixed.Y > mixed.X && mixed.X > 0.1f, $"aim 0.75 is mostly Raise: {mixed}");

        var missing = Try("animation.play", $$"""{"id": "{{id}}", "clip": "Walk"}""").Error.Message;
        Assert.Contains("Stretch", missing, StringComparison.Ordinal);
        var elbow = Try("animation.play", $$"""{"id": "{{id}}", "clip": "Wave", "layer": "Elbow"}""").Error.Message;
        Assert.Contains("Hand", elbow, StringComparison.Ordinal);
        Assert.True(Try("animation.play", $$"""{"id": "{{id}}", "clip": "Wave", "fade": -1}""").IsError);
    }
}

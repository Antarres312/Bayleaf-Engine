using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;

namespace Bayleaf.Scene.Tests;

public class TimelineTests
{
    private static JsonValue Json(string text) => JsonReader.Parse(text).Value;

    private static Track TrackOf(Target target, params (float Time, string Value, Ease Ease)[] keys) =>
        new(target, keys.Select(k => new Key(k.Time, Json(k.Value), k.Ease)).ToList());

    private static double At(Track track, float time) => track.ValueAt(time)?.AsDouble() ?? double.NaN;

    [Fact]
    public void EachEaseReachesTheNextKeyItsOwnWay()
    {
        Track Keys(Ease ease) => TrackOf(Target.OfField("pointLight", "intensity"), (0, "0", ease), (2, "10", Ease.Linear));
        var (step, linear, smooth) = (Keys(Ease.Step), Keys(Ease.Linear), Keys(Ease.Smooth));
        Assert.Equal(0, At(step, 1.9f));
        Assert.Equal(10, At(step, 2f));
        Assert.True(Math.Abs(At(linear, 1f) - 5) < 1e-6, "linear is halfway at half the time");
        Assert.True(At(smooth, 0.5f) < At(linear, 0.5f), "smooth starts more gently than linear");
        Assert.True(At(smooth, 1.5f) > At(linear, 1.5f), "and ends more gently");
        Assert.True(Math.Abs(At(smooth, 1f) - 5) < 1e-6);
    }

    [Fact]
    public void ValuesHoldBeforeTheFirstKeyAndAfterTheLast()
    {
        var t = TrackOf(Target.OfField("transform", "position"), (1, "[1, 2, 3]", Ease.Linear), (2, "[3, 2, 1]", Ease.Linear));
        Assert.Equal(Json("[1, 2, 3]"), t.ValueAt(0));
        Assert.Equal(Json("[3, 2, 1]"), t.ValueAt(5));
        Assert.Equal(Json("[2.0, 2.0, 2.0]"), t.ValueAt(1.5f));
        var text = TrackOf(Target.OfProperty("a.wasm", "say"), (0, "\"hi\"", Ease.Linear), (1, "\"bye\"", Ease.Linear));
        Assert.Equal(Json("\"hi\""), text.ValueAt(0.9f));
    }

    [Fact]
    public void ALoopingTimelineWrapsAndOneThatPlaysOnceStopsAtItsEnd()
    {
        var timeline = new Timeline("door.bprefab", 2);
        Assert.Equal(2, timeline.LocalTime(5));
        Assert.True(timeline.Finished(2.5));
        timeline.Looping = true;
        Assert.True(Math.Abs(timeline.LocalTime(5) - 1) < 1e-6);
        Assert.False(timeline.Finished(100));
    }

    [Fact]
    public void PlacingAKeyKeepsTimeOrderAndReplacesOneAtTheSameMoment()
    {
        var t = TrackOf(Target.OfField("transform", "scale"));
        foreach (var time in new[] { 2f, 0.5f, 1f })
        {
            t.Place(new Key(time, JsonNumber.Of((double)time), Ease.Linear));
        }
        t.Place(new Key(1.0002f, Json("9"), Ease.Step));
        Assert.Equal([0.5f, 1.0002f, 2f], t.Keys.Select(k => k.Time));
        Assert.Equal(Json("9"), t.Keys[1].Value);
    }

    [Fact]
    public void BrokenTimelinesAreRefused()
    {
        static Timeline Good()
        {
            var timeline = new Timeline("door.bprefab", 2);
            timeline.Parts.Add(new Part(StableId.FromUInt128(1), [TrackOf(Target.OfField("transform", "position"), (0, "[0, 0, 0]", Ease.Smooth))]));
            return timeline;
        }
        Assert.Equal(Good().ToJson(), Timeline.FromJson(Good().ToJson()).Value.ToJson());
        var late = Good();
        late.Parts[0].Tracks[0].Keys[0] = late.Parts[0].Tracks[0].Keys[0] with { Time = 3 };
        var unordered = Good();
        unordered.Parts[0].Tracks[0].Keys.Add(new Key(0, Json("[1, 1, 1]"), Ease.Step));
        var obj = Good();
        obj.Parts[0].Tracks[0].Keys[0] = obj.Parts[0].Tracks[0].Keys[0] with { Value = Json("{\"x\": 1}") };
        var twice = Good();
        twice.Parts.Add(twice.Parts[0]);
        var name = Good();
        name.Parts[0].Tracks[0] = new Track(Target.OfField("name", "x"), name.Parts[0].Tracks[0].Keys);
        var tooLong = Good();
        tooLong.Length = float.PositiveInfinity;
        foreach (var broken in new[] { late, unordered, obj, twice, name, tooLong })
        {
            Assert.True(broken.Validate().IsError, broken.ToJson());
        }
        Assert.True(Timeline.FromJson(Good().ToJson().Replace(Timeline.FormatName, "bayleaf.scene", StringComparison.Ordinal)).IsError);
    }

    [Fact]
    public void ApplyingSamplesWritesEachPartOnItsBoundObject()
    {
        var scene = new Scene();
        var root = scene.CreateObject(null, "Lamp").Value;
        var bulb = scene.CreateObject(root, "Bulb").Value;
        scene.AddComponent(bulb, "pointLight");
        scene.Set(bulb, new Scripts([new ScriptRef("flicker.wasm")]));
        // The prefab file's ids differ from the copy's: the copy is found by hierarchy order.
        var (fileRoot, fileBulb) = (StableId.FromUInt128(10), StableId.FromUInt128(11));
        var timeline = new Timeline("lamp.bprefab", 1);
        timeline.Parts.Add(new Part(fileRoot, [TrackOf(Target.OfField("transform", "position"), (0, "[0, 5, 0]", Ease.Step))]));
        timeline.Parts.Add(new Part(fileBulb,
        [
            TrackOf(Target.OfField("pointLight", "color"), (0, "[1, 0, 0]", Ease.Step)),
            TrackOf(Target.OfProperty("flicker.wasm", "rate"), (0, "3.0", Ease.Step)),
            TrackOf(Target.OfField("mesh", "color"), (0, "[1, 0, 0]", Ease.Step)),
        ]));
        var binding = Binding.ByOrder(scene, root, [fileRoot, fileBulb], timeline);
        var problems = Timeline.Apply(scene, binding, timeline.SampleAt(0.5f));
        Assert.Equal(new Double3(0, 5, 0), scene.Get<Transform>(root).Value.Position);
        Assert.Equal(Json("[1.0, 0.0, 0.0]"), Timeline.ReadValue(scene, bulb, Target.OfField("pointLight", "color")));
        Assert.Equal(Json("3.0"), Timeline.ReadValue(scene, bulb, Target.OfProperty("flicker.wasm", "rate")));
        Assert.Single(problems);
    }
}

using System.Numerics;

using Bayleaf.Assets;
using Bayleaf.Core.Json;
using Bayleaf.Graph;

namespace Bayleaf.Animation.Tests;

/// <summary>The Rust engine's animation tests (<c>rust/crates/animation/src/tests.rs</c>), as behaviour.</summary>
public class AnimationTests
{
    private static Motion.Clip Once(string clip) => new(clip, 1f, false);

    /// <summary>Where the player puts the Arm joint now (its place against Root, which stays at rest).</summary>
    private static Vector3 ArmAt(Player player, Skeleton skeleton, Clips clips) => player.Pose(skeleton, clips).Locals[1].Translation;

    [Fact]
    public void AFadeCarriesOneClipIntoTheNextOverItsTime()
    {
        var (skeleton, clips) = Arm.Load();
        var player = new Player();
        Assert.True(player.Play("", Once("Stretch"), 0f).IsOk);
        player.Advance(skeleton, clips, 1f);
        var stretched = ArmAt(player, skeleton, clips);
        Assert.True(stretched.X > 1.4f, $"Stretch at its end: {stretched}");
        Assert.True(player.Play("", Once("Raise"), 1f).IsOk);
        player.Advance(skeleton, clips, 0.5f);
        var halfway = ArmAt(player, skeleton, clips);
        Assert.True(halfway.X > 0.1f && halfway.X < stretched.X - 0.1f, $"half way through the fade, part stretched: {halfway}");
        Assert.True(halfway.Y > 0.05f && halfway.Y < 0.5f, $"and part raised: {halfway}");
        Assert.True(player.Layers[0].Fading);
        player.Advance(skeleton, clips, 0.6f);
        var raised = ArmAt(player, skeleton, clips);
        Assert.True(MathF.Abs(raised.X) < 1e-3f && raised.Y > 0.99f, $"after the fade, Raise alone: {raised}");
        Assert.False(player.Layers[0].Fading, "the finished fade lets the old clip go");
        player.Stop("", 0f);
        Assert.True(ArmAt(player, skeleton, clips).Length() < 1e-4f && !player.IsPlaying, "stopped, at rest");
    }

    [Fact]
    public void AMixWeighsItsClipsByWhereAParameterLies()
    {
        var (skeleton, clips) = Arm.Load();
        var player = new Player();
        var mix = new Motion.Mix([new(Once("Stretch"), 0f), new(Once("Raise"), 1f)], Number.Named("aim"));
        Assert.True(player.Play("", mix, 0f).IsOk);
        player.Advance(skeleton, clips, 1f);
        Vector3 At(float aim)
        {
            Assert.True(player.SetParameter("aim", Parameter.Number(aim)).IsOk);
            return ArmAt(player, skeleton, clips);
        }
        var (low, high, past) = (At(0.25f), At(0.75f), At(5f));
        Assert.True(low.X > high.X && low.Y < high.Y, $"more Raise as aim rises: {low} then {high}");
        Assert.True(low.X > 0f && low.Y > 0f, $"both clips show between their points: {low}");
        Assert.True(MathF.Abs(past.X) < 1e-4f && past.Y > 0.99f, $"past the last point, that clip alone: {past}");
    }

    [Fact]
    public void ALayerPlaysOnlyOnItsJointAndTheNodesUnderIt()
    {
        var (skeleton, clips) = Arm.Load();
        Assert.Equal([0f, 1f, 1f], skeleton.Mask("Arm") ?? []);
        Assert.Null(skeleton.Mask("Missing"));
        var player = new Player();
        Assert.True(player.Play("", Once("Stretch"), 0f).IsOk);
        // Raise moves Arm, which a layer on Hand does not cover.
        Assert.True(player.Play("Hand", Once("Raise"), 0f).IsOk);
        player.Advance(skeleton, clips, 1f);
        var beside = ArmAt(player, skeleton, clips);
        Assert.True(beside.X > 1.4f && MathF.Abs(beside.Y) < 1e-4f, $"a layer on Hand leaves Arm to the base: {beside}");
        Assert.True(player.Play("Arm", Once("Raise"), 0f).IsOk);
        player.Advance(skeleton, clips, 1f);
        var over = ArmAt(player, skeleton, clips);
        Assert.True(over.Y > 0.99f, $"a layer on Arm moves it: {over}");
        player.Stop("Arm", 1f);
        player.Advance(skeleton, clips, 0.5f);
        var stopping = ArmAt(player, skeleton, clips);
        Assert.True(stopping.Y > 0.1f && stopping.Y < 0.9f, $"a stopping layer fades out: {stopping}");
        player.Advance(skeleton, clips, 0.6f);
        Assert.True(player.LayerOn("Arm") is null && MathF.Abs(ArmAt(player, skeleton, clips).Y) < 1e-4f, "and goes");
    }

    [Fact]
    public void ClipEventsArriveOnceEachTimeTheirMomentPasses()
    {
        var (skeleton, clips) = Arm.Load(("Stretch", new ClipEvent(0.25f, "step")), ("Stretch", new ClipEvent(0.75f, "land")));
        var player = new Player();
        Assert.True(player.Play("", Motion.OfClip("Stretch"), 0f).IsOk);
        var heard = new List<string>();
        for (var i = 0; i < 20; i++)
        {
            heard.AddRange(player.Advance(skeleton, clips, 0.1f).Events);
        }
        Assert.Equal(["step", "land", "step", "land"], heard);
        // One long step passes both.
        var longer = new Player();
        Assert.True(longer.Play("", Once("Stretch"), 0f).IsOk);
        Assert.Equal(["step", "land"], longer.Advance(skeleton, clips, 5f).Events);
        Assert.Empty(longer.Advance(skeleton, clips, 5f).Events);
    }

    [Fact]
    public void RootMotionMovesTheObjectAndKeepsTheJointInPlace()
    {
        var (skeleton, clips) = Arm.Load();
        var player = new Player { Root = 1 };
        Assert.True(player.Play("", Motion.OfClip("Stretch"), 0f).IsOk);
        var first = player.Advance(skeleton, clips, 0.5f).Root;
        Assert.True(MathF.Abs(first.X - 0.75f) < 1e-3f && first.Y == 0f, $"half the clip carries it half way: {first}");
        Assert.True(MathF.Abs(ArmAt(player, skeleton, clips).X) < 1e-3f, "the joint is drawn where it stood");
        // Across the clip's start over, the runs add up rather than jump back.
        var across = player.Advance(skeleton, clips, 1f).Root;
        Assert.True(MathF.Abs(across.X - 1.5f) < 1e-3f, $"a whole clip's length further: {across}");
        // Raise moves up only: nothing along the ground.
        Assert.True(player.Play("", Motion.OfClip("Raise"), 0f).IsOk);
        Assert.True(player.Advance(skeleton, clips, 0.5f).Root.Length() < 1e-4f, "a move up is not root motion");
        Assert.True(ArmAt(player, skeleton, clips).Y > 0.4f, "and stays in the pose");
    }

    [Fact]
    public void AMotionNamesWhatTheModelLacks()
    {
        var (skeleton, clips) = Arm.Load();
        var layer = new Motion.Layer(Motion.OfClip("Walk"), Motion.OfClip("Wave"), "Elbow", Number.Of(1f));
        var problems = layer.Problems(skeleton, clips);
        Assert.Contains(problems, p => p.Contains("\"Walk\"", StringComparison.Ordinal) && p.Contains("Stretch", StringComparison.Ordinal));
        Assert.Contains(problems, p => p.Contains("\"Elbow\"", StringComparison.Ordinal));
        Assert.Empty(Motion.OfClip("Wave").Problems(skeleton, clips));
    }

    /// <summary>Builds a graph of animation states node by node, as the editor does.</summary>
    private sealed class Build
    {
        public Graph.Graph Graph { get; } = Bayleaf.Graph.Graph.Empty(AnimationStates.Catalog);

        public uint Node(string kind, params (string Name, JsonValue Value)[] values)
        {
            var id = Graph.AddNode(AnimationStates.Catalog, kind, Vector2.Zero).Value;
            foreach (var (name, value) in values)
            {
                Assert.True(Graph.SetValue(AnimationStates.Catalog, id, name, value).IsOk, $"{kind} takes {name}");
            }
            return id;
        }

        public void Link(uint from, string output, uint to, string input) => Assert.True(Graph.Connect(AnimationStates.Catalog, new Link(from, output, to, input)).IsOk);
    }

    private static (string, JsonValue) S(string name, string value) => (name, new JsonString(value));

    private static (string, JsonValue) F(string name, double value) => (name, JsonNumber.Of(value));

    /// <summary>
    /// Idle mixes Stretch and Raise by "aim"; a "wave" trigger (unless "still") goes to Wave, a layer
    /// of Wave on Hand over Stretch; after one run of Wave, or with aim past 5, back to Idle. It uses
    /// every node kind.
    /// </summary>
    private static Graph.Graph EveryNode()
    {
        var b = new Build();
        b.Node("entry", S("state", "Idle"));
        var idle = b.Node("state", S("name", "Idle"));
        var (stretch, raise) = (b.Node("clip", S("clip", "Stretch"), S("play", "Once")), b.Node("clip", S("clip", "Raise"), S("play", "Once")));
        var mix = b.Node("mix", F("atA", 0), F("atB", 1));
        var aim = b.Node("number", S("name", "aim"));
        b.Link(stretch, "motion", mix, "a");
        b.Link(raise, "motion", mix, "b");
        b.Link(aim, "value", mix, "by");
        b.Link(mix, "motion", idle, "motion");
        var waveState = b.Node("state", S("name", "Wave"));
        var (lower, wave) = (b.Node("clip", S("clip", "Stretch")), b.Node("clip", S("clip", "Wave")));
        var layer = b.Node("layer", S("joint", "Hand"));
        b.Link(lower, "motion", layer, "base");
        b.Link(wave, "motion", layer, "top");
        b.Link(layer, "motion", waveState, "motion");
        var go = b.Node("transition", S("from", "Idle"), S("to", "Wave"), F("fade", 0));
        var (trigger, still) = (b.Node("trigger", S("name", "wave")), b.Node("flag", S("name", "still")));
        var (and, not) = (b.Node("and"), b.Node("not"));
        b.Link(trigger, "value", and, "a");
        b.Link(still, "value", not, "in");
        b.Link(not, "result", and, "b");
        b.Link(and, "result", go, "when");
        var back = b.Node("transition", S("from", "Wave"), S("to", "Idle"), F("fade", 0));
        var time = b.Node("stateTime");
        var (ran, aimed) = (b.Node("compare", S("test", "At Least"), F("b", 1)), b.Node("compare", S("test", "Greater"), F("b", 5)));
        b.Link(time, "runs", ran, "a");
        var aimAgain = b.Node("number", S("name", "aim"));
        b.Link(aimAgain, "value", aimed, "a");
        var or = b.Node("or");
        b.Link(ran, "result", or, "a");
        b.Link(aimed, "result", or, "b");
        b.Link(or, "result", back, "when");
        return b.Graph;
    }

    [Fact]
    public void AnimationStatesWithEveryNodeKindCompileAndRun()
    {
        var graph = EveryNode();
        foreach (var def in AnimationStates.Catalog.Nodes)
        {
            Assert.True(graph.Nodes.Any(n => n.Kind == def.Kind), $"the test graph lacks {def.Kind}");
        }
        var machine = AnimationStates.Compile(graph).Value;
        Assert.Equal(2, machine.States.Count);
        Assert.Equal(3, machine.Parameters.Count);
        var (skeleton, clips) = Arm.Load();
        var player = new Player();
        player.RunStates(machine);
        player.Advance(skeleton, clips, 0.1f);
        Assert.Equal("Idle", player.State);
        Assert.True(player.SetParameter("wave", Parameter.Trigger(true)).IsOk);
        player.Advance(skeleton, clips, 0.1f);
        Assert.Equal("Wave", player.State);
        Assert.Equal(Parameter.Trigger(false), player.Parameters["wave"]);
        player.Advance(skeleton, clips, 1.2f);
        player.Advance(skeleton, clips, 0.01f);
        Assert.Equal("Idle", player.State);
        Assert.True(player.SetParameter("still", Parameter.Flag(true)).IsOk);
        Assert.True(player.SetParameter("wave", Parameter.Trigger(true)).IsOk);
        player.Advance(skeleton, clips, 0.1f);
        Assert.Equal("Idle", player.State);
        Assert.Equal(Parameter.Trigger(true), player.Parameters["wave"]);
        Assert.True(player.GoTo("Wave", 0f).IsOk);
        Assert.Equal("Wave", player.State);
        Assert.Contains("Idle, Wave", player.GoTo("Run", 0f).Error, StringComparison.Ordinal);
    }

    [Fact]
    public void BrokenAnimationStatesNameTheirProblemsOnTheirNodes()
    {
        var b = new Build();
        var first = b.Node("state", S("name", "Idle"));
        var twin = b.Node("state", S("name", "Idle"));
        var nowhere = b.Node("transition", S("from", "Idle"), S("to", "Run"));
        var mix = b.Node("mix");
        b.Link(mix, "motion", first, "motion");
        var speed = b.Node("number", S("name", "speed"));
        var flag = b.Node("flag", S("name", "speed"));
        var problems = AnimationStates.Compile(b.Graph).Error;
        string[] On(uint node) => [.. problems.Where(p => p.Node == node).Select(p => p.Message)];
        Assert.True(On(twin).Length > 0 && On(nowhere).Length > 0 && On(mix).Length > 0, Problem.Summary(problems));
        Assert.True(On(speed).Length + On(flag).Length >= 1, $"one name, two kinds: {Problem.Summary(problems)}");
        // A motion goes only into a motion input.
        var compare = b.Node("compare");
        Assert.True(b.Graph.Connect(AnimationStates.Catalog, new Link(mix, "motion", compare, "a")).IsError);
        Assert.True(b.Graph.Connect(AnimationStates.Catalog, new Link(speed, "value", twin, "motion")).IsError);
    }
}

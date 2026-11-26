using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using SceneModel = Bayleaf.Scene.Scene;
using Transform = Bayleaf.Scene.Components.Transform;

namespace Bayleaf.Script.Tests;

/// <summary>
/// The C# script host, as the Rust host's tests check the WebAssembly one (rust/crates/script/src/tests.rs):
/// C# scripts written into a project folder stand in for the Rust test modules.
/// </summary>
public sealed class HostTests : IDisposable
{
    private readonly string folder = Path.Join(Path.GetTempPath(), $"bayleaf-host-{Guid.NewGuid():N}");

    public HostTests() => Directory.CreateDirectory(folder);

    public void Dispose() => Directory.Delete(folder, true);

    private void Write(string relative, string source)
    {
        var path = Path.Join(folder, relative);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, source);
    }

    private const string Spinner = """
        public class Spinner : GameScript
        {
            [ScriptProperty] public float Speed = 90f;

            protected override void Advance(float seconds)
            {
                var pose = Pose;
                pose.Rotation.Y = (pose.Rotation.Y + Speed * seconds) % 360f;
                Pose = pose;
            }
        }
        """;

    private (ScriptHost Host, LoadedScripts Loaded) Host()
    {
        var project = ScriptProject.Compile(folder);
        Assert.True(project.Compiled, project.Summary());
        var loaded = project.Load().Value;
        return (new ScriptHost(loaded), loaded);
    }

    private static StableId Add(SceneModel scene, string name, params ScriptRef[] scripts)
    {
        var id = scene.CreateObject(null, name).Value;
        if (scripts.Length > 0)
        {
            Assert.True(scene.Set(id, new Scripts(scripts)).IsOk);
        }
        return id;
    }

    private static ScriptRef Script(string module, params (string Name, PropertyValue Value)[] properties)
    {
        var script = new ScriptRef(module);
        foreach (var (name, value) in properties)
        {
            script.Properties[name] = value;
        }
        return script;
    }

    private static Transform TransformOf(SceneModel scene, StableId id) => scene.TryGet<Transform>(id, out var t) ? t : Transform.Initial;

    [Fact]
    public void AStepMovesTheObjectByItsPropertyAndTheSceneOverridesTheDefault()
    {
        Write("scripts/spinner.cs", Spinner);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        var plain = Add(scene, "Plain", Script("scripts/spinner.cs"));
        var fast = Add(scene, "Fast", Script("scripts/spinner.cs", ("speed", new PropertyValue.Number(180))));
        Assert.True(host.LoadScene(scene, 0));
        Assert.Equal(2, host.Running);
        Assert.True(host.Update(scene, 0.5f, 0.5));
        Assert.True(TransformOf(scene, plain).Rotation.Y > 0f, "the default speed turns it");
        Assert.True(TransformOf(scene, fast).Rotation.Y > TransformOf(scene, plain).Rotation.Y, "the scene's speed is faster than the default");
    }

    [Fact]
    public void ScriptsPrepareThenBeginOnceAndTheirLogsReachTheHost()
    {
        Write("a.cs", """
            public class A : GameScript
            {
                protected override void Prepare() => Log("a prepares");
                protected override void Begin() => Log($"a begins seeing {Game.Find("B").Name}");
            }
            """);
        Write("b.cs", """
            public class B : GameScript
            {
                protected override void Prepare() => Log("b prepares");
                protected override void Begin() => Log("b begins");
            }
            """);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        Add(scene, "A", Script("a.cs"));
        Add(scene, "B", Script("b.cs"));
        host.LoadScene(scene, 0);
        host.Update(scene, 0.1f, 0.1);
        host.Update(scene, 0.1f, 0.2);
        var lines = host.TakeLogs().Select(l => l.Text).ToList();
        Assert.Equal(["a prepares", "b prepares", "a begins seeing B", "b begins"], lines);
    }

    [Fact]
    public void AStepThatRunsTooLongIsStoppedByTheWatchdog()
    {
        Write("stuck.cs", """
            public class Stuck : GameScript
            {
                public static volatile bool Forever = true;
                protected override void Advance(float seconds)
                {
                    while (Forever) { }
                }
            }
            """);
        var (host, loaded) = Host();
        var scene = new SceneModel();
        Add(scene, "Stuck", Script("stuck.cs"));
        host.LoadScene(scene, 0);
        host.TimeLimit = TimeSpan.FromMilliseconds(200);
        Assert.False(host.Update(scene, 0.1f, 0.1), "the watchdog stops the step");
        Assert.True(host.Stalled);
        Assert.Contains("Stuck", host.StalledIn, StringComparison.Ordinal);
        Assert.False(host.Update(scene, 0.1f, 0.2), "a stalled host runs nothing more");
        // Let the stuck thread finish, so the test leaves nothing running.
        loaded.Types.Values.Single().Type.GetField("Forever")!.SetValue(null, false);
        host.Dispose();
    }

    [Fact]
    public void AnErrorStopsOnlyThatScriptWithItsReason()
    {
        Write("broken.cs", """
            public class Broken : GameScript
            {
                protected override void Advance(float seconds) => throw new InvalidOperationException("no way");
            }
            """);
        Write("scripts/spinner.cs", Spinner);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        var both = Add(scene, "Both", Script("broken.cs"), Script("scripts/spinner.cs"));
        host.LoadScene(scene, 0);
        host.Update(scene, 0.5f, 0.5);
        var fault = Assert.Single(host.TakeFaults());
        Assert.Equal("Broken", fault.Script);
        Assert.Contains("no way", fault.Error, StringComparison.Ordinal);
        Assert.Equal(1, host.Running);
        host.Update(scene, 0.5f, 1.0);
        Assert.True(TransformOf(scene, both).Rotation.Y > 50f, "the other script on the object keeps turning it");
    }

    [Fact]
    public void ARefusedCallStopsTheScriptEvenWhenItCatchesTheError()
    {
        Write("sly.cs", """
            public class Sly : GameScript
            {
                protected override void Advance(float seconds)
                {
                    try { Self.Send(""); } catch (InvalidOperationException) { }
                }
            }
            """);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        Add(scene, "Sly", Script("sly.cs"));
        host.LoadScene(scene, 0);
        host.Update(scene, 0.1f, 0.1);
        Assert.Contains("name", Assert.Single(host.TakeFaults()).Error, StringComparison.Ordinal);
        Assert.Equal(0, host.Running);
    }

    [Fact]
    public void ScriptsMoveOtherObjectsAndAskForSpawnsAndDestroysAfterTheStep()
    {
        Write("boss.cs", """
            public class Boss : GameScript
            {
                protected override void Advance(float seconds)
                {
                    var other = Game.Find("Minion");
                    var pose = other.Pose!.Value;
                    pose.Position.X += 2;
                    other.SetPose(pose);
                    Game.Spawn("prefabs/coin.bprefab", new Vector3d(1, 2, 3));
                    Game.Find("Doomed").Destroy();
                }
            }
            """);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        Add(scene, "Boss", Script("boss.cs"));
        var minion = Add(scene, "Minion");
        var doomed = Add(scene, "Doomed");
        host.LoadScene(scene, 0);
        host.Update(scene, 0.1f, 0.1);
        Assert.Equal(2.0, TransformOf(scene, minion).Position.X);
        var requests = host.TakeRequests();
        Assert.Contains(requests, r => r is Request.Spawn { Prefab: "prefabs/coin.bprefab" } s && s.Position == new Double3(1, 2, 3));
        Assert.Contains(requests, r => r is Request.Destroy d && d.Target == doomed);
        Assert.True(scene.Contains(doomed), "the engine destroys it, after the step");
    }

    [Fact]
    public void AMessageReachesOnlyTheScriptsOfItsObjectAfterTheStep()
    {
        Write("caller.cs", """
            public class Caller : GameScript
            {
                protected override void Advance(float seconds) => Game.Find("Hearer").Send("ping", 7);
            }
            """);
        Write("ear.cs", """
            public class Ear : GameScript
            {
                protected override void Receive(in Message message) => Log($"{Self.Name} heard {message.Name} {message.Value}");
            }
            """);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        Add(scene, "Caller", Script("caller.cs"));
        Add(scene, "Hearer", Script("ear.cs"));
        Add(scene, "Other", Script("ear.cs"));
        host.LoadScene(scene, 0);
        host.Update(scene, 0.1f, 0.1);
        Assert.Equal(["Hearer heard ping 7"], host.TakeLogs().Select(l => l.Text));
        host.Send(scene, null, "all", 1, 0.1);
        Assert.Equal(2, host.TakeLogs().Count);
    }

    [Fact]
    public void AScriptThatStepsManyMovesAllItsObjectsInOneCall()
    {
        Write("turner.cs", """
            public class Turner : GameScript, IAdvanceMany<Turner>
            {
                public static int Calls;
                [ScriptProperty] public float Speed = 10f;

                public static void AdvanceMany(Many<Turner> objects, float seconds)
                {
                    Calls++;
                    for (var i = 0; i < objects.Count; i++)
                    {
                        objects.Poses[i].Rotation.Y += objects.Scripts[i].Speed * seconds;
                    }
                }
            }
            """);
        var (host, loaded) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        var ids = Enumerable.Range(0, 5).Select(i => Add(scene, $"T{i}", Script("turner.cs", ("speed", new PropertyValue.Number(i * 10))))).ToList();
        host.LoadScene(scene, 0);
        host.Update(scene, 1f, 1);
        host.Update(scene, 1f, 2);
        var calls = (int)loaded.Types.Values.Single().Type.GetField("Calls")!.GetValue(null)!;
        Assert.Equal(2, calls);
        Assert.Equal([0f, 20f, 40f, 60f, 80f], ids.Select(id => TransformOf(scene, id).Rotation.Y));
    }

    [Fact]
    public void ACompileErrorSaysWhereAndAScriptFileWithoutAScriptSaysSo()
    {
        Write("bad.cs", "public class Bad : GameScript { void Nope() { undefined(); } }");
        var project = ScriptProject.Compile(folder);
        Assert.False(project.Compiled);
        var problem = Assert.Single(project.Problems);
        Assert.Equal("bad.cs", problem.File);
        Assert.Equal(1, problem.Line);
        Write("bad.cs", "public static class Helper { public static int Two => 2; }");
        var fixedProject = ScriptProject.Compile(folder);
        Assert.True(fixedProject.Compiled);
        Assert.Contains("declares no script", fixedProject.Describe("bad.cs").Error, StringComparison.Ordinal);
    }

    [Fact]
    public void DescribingAScriptReadsItsPropertiesWithoutRunningIt()
    {
        Write("props.cs", """
            [ScriptMenu("Light/Flicker")]
            [ScriptEvent("Damaged", "amount:number", "by:object")]
            public class Props : GameScript
            {
                public Props() => throw new InvalidOperationException("never run while describing");
                [ScriptProperty(Low = 0, High = 20, Slider = true, Hint = "Units per second")] public float Speed = 3.5f;
                [ScriptProperty] public int Lives = 3;
                [ScriptProperty] public bool Loud = true;
                [ScriptProperty(Color = true)] public Vector3 Tint = new(1, 0.5f, 0);
                [ScriptProperty("label")] public string Words = "Hi";
                [ScriptProperty(File = "prefab")] public string Coin = "";
                [ScriptProperty] public SceneObject Target;
                private float notShown;
            }
            """);
        var project = ScriptProject.Compile(folder);
        var described = project.Describe("props.cs").Value;
        Assert.Empty(described.Problems);
        Assert.Equal("Light/Flicker", described.Menu);
        Assert.Equal(["speed", "lives", "loud", "tint", "label", "coin", "target"], described.Properties.Select(p => p.Name));
        Assert.Equal([PropertyType.Number, PropertyType.WholeNumber, PropertyType.TrueFalse, PropertyType.Color, PropertyType.Text, PropertyType.Prefab, PropertyType.SceneObject], described.Properties.Select(p => p.Type));
        Assert.Equal(new PropertyValue.Number(3.5), described.Properties[0].Default);
        Assert.Equal(new PropertyValue.Triple(new Vector3(1, 0.5f, 0)), described.Properties[3].Default);
        Assert.Single(described.Inspector);
        Assert.Equal("Damaged", Assert.Single(described.Events).Name);
    }

    [Fact]
    public void APropertyOfATypeScriptsCannotHaveIsAProblemOfThatScript()
    {
        Write("odd.cs", """
            public class Odd : GameScript
            {
                [ScriptProperty] public List<int> Many = new();
                [ScriptProperty] public float Moving = Environment.TickCount;
            }
            """);
        var described = ScriptProject.Compile(folder).Describe("odd.cs").Value;
        Assert.Equal(2, described.Problems.Count);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        Add(scene, "Odd", Script("odd.cs"));
        host.LoadScene(scene, 0);
        Assert.Equal(0, host.Count);
        Assert.Contains("List", Assert.Single(host.TakeFaults()).Error, StringComparison.Ordinal);
    }

    [Fact]
    public void AChangedFileIsSeenAndTheReloadedScriptsBeginAgain()
    {
        Write("scripts/spinner.cs", Spinner);
        var (host, loaded) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        var id = Add(scene, "Spinning", Script("scripts/spinner.cs"));
        host.LoadScene(scene, 0);
        host.Update(scene, 1f, 1);
        Assert.Equal(90f, TransformOf(scene, id).Rotation.Y);
        Assert.False(loaded.Project.Changed());
        Write("scripts/spinner.cs", Spinner.Replace("% 360f", "% 360f + 1000f", StringComparison.Ordinal).Replace("90f", "1f", StringComparison.Ordinal));
        File.SetLastWriteTimeUtc(Path.Join(folder, "scripts/spinner.cs"), DateTime.UtcNow.AddSeconds(5));
        Assert.True(loaded.Project.Changed());
        var next = ScriptProject.Compile(folder).Load().Value;
        Assert.True(host.Reload(next, scene, 1));
        host.Update(scene, 1f, 2);
        Assert.True(TransformOf(scene, id).Rotation.Y > 1000f, "the new code runs, keeping where play had turned it");
    }

    [Fact]
    public void ScriptsReadTheGameStateTheInputAndTheDataFiles()
    {
        Write("reader.cs", """
            public class Reader : GameScript
            {
                protected override void Advance(float seconds)
                {
                    Game.SetValue("score", (Game.Value("score") ?? 0) + 1);
                    var goblin = Game.Data("data/goblin.bdata");
                    Log($"{Controls.KeyDown("W")} {goblin.Number("health")} {goblin.Count("loot")} {goblin.TextAt("loot", 1)} {goblin.Number("nothing") is null}");
                }
            }
            """);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        Add(scene, "Reader", Script("reader.cs"));
        host.Data = new Dictionary<string, SortedDictionary<string, ResolvedField>>
        {
            ["data/goblin.bdata"] = new(StringComparer.Ordinal)
            {
                ["health"] = new ResolvedField(PropertyType.Number, PropertyShape.One, new PropertyValue.Number(12)),
                ["loot"] = new ResolvedField(PropertyType.Text, PropertyShape.List, new PropertyValue.List([new PropertyValue.Text("gold"), new PropertyValue.Text("gem")])),
            },
        };
        host.Held = PlayInput.None with { Keys = new HashSet<string>(["W"], StringComparer.Ordinal) };
        host.LoadScene(scene, 0);
        host.Update(scene, 0.1f, 0.1);
        host.Update(scene, 0.1f, 0.2);
        Assert.Equal(2.0, host.State.Game["score"]);
        Assert.Equal("True 12 2 gem True", host.TakeLogs()[0].Text);
    }

    /// <summary>The repository's own example scripts (<c>scripts/</c>, the ports of <c>rust/scripts</c>), copied into the test's project.</summary>
    private void CopyExamples()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        foreach (var file in Directory.EnumerateFiles(Path.Join(dir!.FullName, "scripts"), "*.cs"))
        {
            File.Copy(file, Path.Join(folder, Path.GetFileName(file)));
        }
    }

    [Fact]
    public void TheExampleSpinnerTurnsEachObjectByItsSpeedAndTheMoverWalksAndHops()
    {
        CopyExamples();
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        var slow = Add(scene, "Slow", Script("spinner.cs", ("speed", new PropertyValue.Number(60))));
        var fast = Add(scene, "Fast", Script("spinner.cs", ("speed", new PropertyValue.Number(300))));
        var walker = Add(scene, "Walker", Script("mover.cs"));
        host.LoadScene(scene, 0);
        Assert.Contains(host.TakeLogs(), l => l.Text == "spinner: started");
        host.Held = PlayInput.None with { Keys = new HashSet<string>(["D", "Space"], StringComparer.Ordinal) };
        var time = 0.0;
        var highest = 0.0;
        for (var i = 0; i < 20; i++)
        {
            time += 0.1;
            host.Update(scene, 0.1f, time);
            highest = Math.Max(highest, TransformOf(scene, walker).Position.Y);
            host.Held = PlayInput.None with { Keys = new HashSet<string>(["D"], StringComparer.Ordinal) };
        }
        Assert.True(Math.Abs(TransformOf(scene, slow).Rotation.Y - 120f) < 1e-2f, $"two seconds at 60°/s: {TransformOf(scene, slow).Rotation.Y}");
        Assert.True(Math.Abs(TransformOf(scene, fast).Rotation.Y - 240f) < 1e-2f, $"600° turns back past 360: {TransformOf(scene, fast).Rotation.Y}");
        Assert.True(Math.Abs(TransformOf(scene, walker).Position.X - 6) < 1e-4, $"D held, 3 units per second: {TransformOf(scene, walker).Position.X}");
        Assert.True(highest > 0.5 && TransformOf(scene, walker).Position.Y == 0, $"Space hopped up to {highest}, and it landed again");
        Assert.Empty(host.TakeFaults());
    }

    [Fact]
    public void AStepAllocatesNothingOnceWarmedUp()
    {
        Write("scripts/spinner.cs", Spinner);
        CopyExamples();
        Write("follow.cs", """
            public class Follow : GameScript
            {
                private SceneObject target;
                protected override void Begin() => target = Game.Find("Target");
                protected override void Advance(float seconds)
                {
                    if (target.Pose is not { } goal) return;
                    var pose = Pose;
                    pose.Position.X += (goal.Position.X - pose.Position.X) * 0.5 * seconds;
                    Pose = pose;
                }
            }
            """);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        Add(scene, "Target");
        for (var i = 0; i < 200; i++)
        {
            Add(scene, $"O{i}", Script("scripts/spinner.cs"), Script("spinner.cs"), Script("follow.cs"));
        }
        host.LoadScene(scene, 0);
        var time = 0.0;
        for (var i = 0; i < 50; i++)
        {
            time += 1 / 60.0;
            host.Update(scene, 1 / 60f, time);
        }
        var before = GC.GetTotalAllocatedBytes(precise: true);
        for (var i = 0; i < 100; i++)
        {
            time += 1 / 60.0;
            host.Update(scene, 1 / 60f, time);
        }
        var allocated = GC.GetTotalAllocatedBytes(precise: true) - before;
        Assert.Equal(600, host.Running);
        Assert.True(allocated < 1024, $"100 steps of 600 scripts allocated {allocated} bytes");
    }

    [Fact]
    public void APoseChangedInPlaceReachesTheSceneAndOneThatIsNotFiniteIsRefused()
    {
        Write("wild.cs", """
            public class Wild : GameScript
            {
                [ScriptProperty] private bool broken;
                protected override void Advance(float seconds)
                {
                    Pose.Position.X += 1;
                    if (broken) Pose.Rotation.Y = float.NaN;
                }
            }
            """);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        var calm = Add(scene, "Calm", Script("wild.cs"));
        var broken = Add(scene, "Broken", Script("wild.cs", ("broken", new PropertyValue.Flag(true))));
        host.LoadScene(scene, 0);
        host.Update(scene, 0.1f, 0.1);
        host.Update(scene, 0.1f, 0.2);
        Assert.Equal(2.0, TransformOf(scene, calm).Position.X);
        Assert.Equal(0.0, TransformOf(scene, broken).Position.X);
        Assert.Contains(host.TakeLogs(), l => l.Error && l.Owner == broken && l.Text.Contains("not finite", StringComparison.Ordinal));
    }

    [Fact]
    public void WhatIsNotInTheCSharpEngineYetSaysSoOnceAndDoesNothing()
    {
        Write("noisy.cs", """
            public class Noisy : GameScript
            {
                protected override void Advance(float seconds)
                {
                    if (Self.PlaySound() || Self.Push(Vector3.UnitY) || Game.Raycast(default, Vector3.UnitX) is not null)
                    {
                        Log("it did something");
                    }
                }
            }
            """);
        var (host, _) = Host();
        using var _1 = host;
        var scene = new SceneModel();
        Add(scene, "Noisy", Script("noisy.cs"));
        host.LoadScene(scene, 0);
        host.Update(scene, 0.1f, 0.1);
        host.Update(scene, 0.1f, 0.2);
        var lines = host.TakeLogs();
        Assert.All(lines, l => Assert.True(l.Error));
        Assert.Equal(3, lines.Count);
        Assert.Contains(lines, l => l.Text.StartsWith("Sound is not in the C# engine yet", StringComparison.Ordinal));
        Assert.Equal(1, host.Running);
    }
}

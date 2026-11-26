using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Gameplay;
using Bayleaf.Scene.Components;
using Bayleaf.Streaming;

using Transform = Bayleaf.Scene.Components.Transform;

namespace Bayleaf.Engine.Tests;

/// <summary>
/// Play mode, as the Rust engine's tests check it (rust/crates/engine/src/tests.rs, the play tests):
/// C# scripts stand in for the WebAssembly modules those tests write.
/// </summary>
public sealed class PlayTests : IDisposable
{
    private readonly string dir = Path.Join(Path.GetTempPath(), $"bayleaf-play-cs-{Guid.NewGuid():N}");
    private readonly CommandRegistry registry = CommandRegistry.WithBuiltins();
    private readonly Session session = new();

    public PlayTests() => Directory.CreateDirectory(dir);

    public void Dispose()
    {
        if (session.IsPlaying)
        {
            registry.Execute(session, "play.stop", JsonValue.Null);
        }
        Directory.Delete(dir, true);
    }

    private static string Quote(string text) => new JsonString(text).ToCompact();

    private Result<JsonValue, CommandError> Try(string command, string parameters = "{}") => registry.Execute(session, command, JsonReader.Parse(parameters).Value);

    private JsonValue Run(string command, string parameters = "{}")
    {
        var result = Try(command, parameters);
        Assert.True(result.IsOk, result.IsOk ? "" : $"{command}: {result.Error}");
        return result.Value;
    }

    private string Create(string name, string components = "{}") =>
        Run("scene.object.create", $$"""{"name": {{Quote(name)}}, "components": {{components}}}""").Get("id")!.AsString()!;

    private void Write(string relative, string text)
    {
        var path = Path.Join(dir, relative);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, text);
    }

    private void SaveAs(string relative) => Run("scene.saveAs", $$"""{"path": {{Quote(Path.Join(dir, relative))}}}""");

    private void NewProject() => Run("project.new", $$"""{"path": {{Quote(Path.Join(dir, "game"))}}}""");

    private string ProjectDir => session.ProjectDir!;

    private void WriteInProject(string relative, string text)
    {
        var path = Path.Join(ProjectDir, relative);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, text);
    }

    private static StableId Id(string hex) => StableId.ParseHex(hex).Value;

    private float RotationY(string id) => session.Scene.TryGet<Transform>(Id(id), out var t) ? t.Rotation.Y : float.NaN;

    private int Named(string name) => Script.ScriptHost.Order(session.Scene).Count(id => session.Scene.NameOf(id) == name);

    private bool Said(string text) => session.Console.Any(l => l.Text.Contains(text, StringComparison.Ordinal));

    private const string Spinner = """
        public class Spin : GameScript
        {
            [ScriptProperty] public float Speed = 1f;

            protected override void Begin() => Log("spinning");

            protected override void Advance(float seconds)
            {
                var pose = Pose;
                pose.Rotation.Y += Speed * seconds;
                Pose = pose;
            }
        }
        """;

    /// <summary>A saved scene in the test's folder with one object running <paramref name="source"/> (as <c>spin.cs</c>), its scripts allowed.</summary>
    private string Scripted(string source)
    {
        Write("spin.cs", source);
        var id = Create("Spinner", """{"scripts": [{"module": "spin.cs", "properties": {"speed": 90}}]}""");
        SaveAs("play.bscene");
        session.AllowScripts(true);
        return id;
    }

    [Fact]
    public void PlayRunsScriptsAndStopRestoresTheEditedScene()
    {
        var id = Scripted(Spinner);
        var edited = session.Scene.ToDoc();
        var started = Run("play.start");
        Assert.Equal(1, started.Get("running")!.AsDouble());
        Assert.True(session.IsPlaying);
        Run("play.step", """{"seconds": 0.1, "steps": 10}""");
        Assert.True(Math.Abs(RotationY(id) - 90f) < 1e-3f, $"one second at 90°/s: {RotationY(id)}");
        Assert.True(Said("spinning"), "the script's log reached the console");
        Run("play.stop");
        Assert.False(session.IsPlaying);
        Assert.Equal(edited, session.Scene.ToDoc());
        Assert.False(session.Dirty, "play never marks the file changed");
    }

    [Fact]
    public void PauseStopsTicksButStepStillAdvances()
    {
        var id = Scripted(Spinner);
        Run("play.start");
        Run("play.pause");
        session.Tick(1f);
        Assert.Equal(0f, RotationY(id));
        Run("play.step", """{"seconds": 0.5}""");
        Assert.True(RotationY(id) > 0f);
        Run("play.pause");
        session.Tick(0.5f);
        Assert.True(RotationY(id) > 50f, "resumed, the editor's frames move it");
    }

    [Fact]
    public void FilesAndHistoryAreLockedDuringPlayAndEditsAreNotRecorded()
    {
        var id = Scripted(Spinner);
        var steps = session.History.Count;
        Run("play.start");
        foreach (var command in new[] { "scene.save", "scene.new", "edit.undo" })
        {
            Assert.Equal(CommandErrorKind.Disabled, Try(command).Error?.Kind);
        }
        Assert.True(Try("play.start").IsError, "play cannot start twice");
        Run("scene.component.add", $$"""{"id": "{{id}}", "component": "tags"}""");
        Assert.Equal(steps, session.History.Count);
        Run("play.stop");
        Assert.False(session.Scene.TryGet<Tags>(Id(id), out _), "play-mode edits are thrown away");
    }

    [Fact]
    public void AMissingOrBrokenScriptIsReportedAndPlayStillRuns()
    {
        Scripted("public class Spin : GameScript { void Broken() { nope(); } }");
        var started = Run("play.start");
        Assert.Equal(0, started.Get("running")!.AsDouble());
        Assert.Contains(session.Console, l => l.Level == Level.Error && l.Text.Contains("spin.cs", StringComparison.Ordinal) && l.Text.Contains("do not compile", StringComparison.Ordinal));
        Run("play.step");
        Assert.True(Try("play.step", """{"steps": 0}""").IsError);
        Assert.True(Try("play.step", """{"seconds": -1}""").IsError);
        Run("play.stop");

        File.Delete(Path.Join(dir, "spin.cs"));
        Create("Other", """{"scripts": [{"module": "missing.cs"}, {"module": "old.wasm"}]}""");
        Run("play.start");
        Assert.Contains(session.Console, l => l.Level == Level.Error && l.Text.Contains("missing.cs is not a C# script of the project", StringComparison.Ordinal));
        Assert.Contains(session.Console, l => l.Level == Level.Error && l.Text.Contains("old.wasm is not a C# script", StringComparison.Ordinal));
    }

    [Fact]
    public void ScriptsWaitForThePersonsConsentAndAgentsCannotGiveIt()
    {
        var id = Scripted(Spinner);
        session.AllowScripts(false);
        var started = Run("play.start");
        Assert.Equal(0, started.Get("running")!.AsDouble());
        Assert.Contains("allows this project's scripts", started.Get("notRun")!.AsString(), StringComparison.Ordinal);
        Run("play.step", """{"seconds": 0.5}""");
        Assert.Equal(0f, RotationY(id));
        Run("play.stop");

        session.Confinement = Confinement.Create([dir], [dir]).Value;
        var refused = Try("project.scripts.allow");
        Assert.Equal(CommandErrorKind.Disabled, refused.Error?.Kind);
        Assert.False(session.ScriptsAllowed);
        session.Confinement = null;
        Assert.True(Run("project.scripts.allow").Get("allowed") is JsonBool { Value: true });
        Assert.Equal(1, Run("play.start").Get("running")!.AsDouble());
    }

    [Fact]
    public void AStepThatRunsTooLongStopsPlayModeAndPutsTheSceneBack()
    {
        Scripted("""
            public class Spin : GameScript
            {
                public static volatile bool Stuck = true;
                protected override void Advance(float seconds)
                {
                    var pose = Pose;
                    pose.Position.X = 9;
                    Pose = pose;
                    while (Stuck) { }
                }
            }
            """);
        var edited = session.Scene.ToDoc();
        session.ScriptTimeLimit = TimeSpan.FromMilliseconds(300);
        Run("play.start");
        var type = session.Play!.Host.Scripts!.Project.Describe("spin.cs").Value.TypeName;
        Run("play.step");
        Assert.False(session.IsPlaying, "the watchdog stopped play mode");
        Assert.Equal(edited, session.Scene.ToDoc());
        Assert.Contains(session.Console, l => l.Level == Level.Error && l.Text.Contains("ran longer than 0.3 s", StringComparison.Ordinal) && l.Text.Contains("Spin on Spinner", StringComparison.Ordinal));
        // Let the stuck thread end, so the test leaves nothing running.
        AppDomain.CurrentDomain.GetAssemblies().Select(a => a.GetType(type)).OfType<Type>().ToList().ForEach(t => t.GetField("Stuck")?.SetValue(null, false));
    }

    [Fact]
    public void ScriptsSpawnPrefabsAndDestroyObjectsInPlayModeAndStopUndoesIt()
    {
        Scripted("""
            public class Spin : GameScript
            {
                protected override void Advance(float seconds)
                {
                    if (Controls.MouseDown(0))
                    {
                        Game.Spawn("ball.bprefab", new Vector3d(0, 5, 0));
                        Game.Find("Junk").Destroy();
                    }
                }
            }
            """);
        Create("Junk");
        var ball = Create("Ball");
        Run("scene.prefab.create", $$"""{"id": "{{ball}}", "path": "ball.bprefab"}""");
        Run("scene.object.delete", $$"""{"id": "{{ball}}"}""");
        Run("scene.save");
        var edited = session.Scene.ToDoc();
        Run("play.start");
        Run("play.step");
        Assert.Equal(0, Named("Ball"));
        Run("play.input", """{"keys": [], "pointer": [0.1, 0.2], "buttons": 1}""");
        Assert.True(Try("play.input", """{"keys": [], "pad": {"buttons": ["Jump"]}}""").IsError, "unknown gamepad buttons are refused");
        Run("play.input", """{"keys": [], "pointer": [0.1, 0.2], "buttons": 1, "pad": {"axes": [2, 0, 0, 0, 0, 0], "buttons": ["South"]}}""");
        Run("play.step");
        Assert.Equal(1, Named("Ball"));
        Assert.Equal(0, Named("Junk"));
        var spawned = Script.ScriptHost.Order(session.Scene).First(id => session.Scene.NameOf(id) == "Ball");
        Assert.Equal(5.0, session.Scene.TryGet<Transform>(spawned, out var t) ? t.Position.Y : 0);
        Assert.DoesNotContain(session.Console, l => l.Level == Level.Error);
        Run("play.stop");
        Assert.Equal(edited, session.Scene.ToDoc());
    }

    [Fact]
    public void GameRulesPlaceTheRestartAndEndTheGameAndTheStateStartsEmpty()
    {
        session.Use(new GameplayModule());
        NewProject();
        WriteInProject("referee.cs", """
            public class Referee : GameScript
            {
                protected override void Receive(in Message message)
                {
                    if (message.Is("gameWon")) Game.SetValue("cheered", 1);
                }
            }
            """);
        session.AllowScripts(true);
        Create("Referee", """{"scripts": [{"module": "referee.cs"}]}""");
        var hero = Create("Hero", """{"playerControl": {"speed": 4, "jumpSpeed": 6, "turnToMove": true, "camera": "none", "cameraDistance": 6, "cameraHeight": 3}}""");
        Run("scene.prefab.create", $$"""{"id": "{{hero}}", "path": "hero.bprefab"}""");
        Run("scene.object.delete", $$"""{"id": "{{hero}}"}""");
        Create("Start", """{"playerStart": {}, "transform": {"position": [40, 2, 0], "rotation": [0, 90, 0], "scale": [1, 1, 1]}}""");
        Create("Rules", """{"gameRules": {"playerPrefab": "hero.bprefab", "respawn": true, "respawnHeight": -5, "timeLimit": 3, "timeUp": "win"}}""");
        var edited = session.Scene.ToDoc();

        Run("play.start");
        var players = Rules.Players(session.Scene);
        var player = Assert.Single(players);
        Assert.True(Math.Abs(session.Scene.WorldMatrix(player)!.Value.WAxis.X - 40) < 1e-6, "at the Player Start");
        var state = Run("game.state");
        Assert.True(state.Get("game") is JsonObject { Count: 0 } && state.Get("players") is JsonArray { Count: 0 } && state.Get("outcome") is JsonNull);
        Assert.True(Try("game.state.set", $$"""{"name": "lives", "value": 3, "player": "{{StableId.Random()}}"}""").IsError, "only players have a player state");
        state = Run("game.state.set", $$"""{"name": "lives", "value": 3, "player": "{{player}}"}""");
        Assert.Equal(3.0, state.Get("players")!.AsArray()![0].Get("values")!.Get("lives")!.AsDouble());

        // Without physics nothing falls: a player put below the line starts over.
        Run("scene.component.set", $$$"""{"id": "{{{player}}}", "component": "transform", "value": {"position": [0, -20, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]}}""");
        Run("play.step", """{"seconds": 0.05, "steps": 2}""");
        Assert.True(Said("starts over"));
        Assert.True(session.Scene.WorldMatrix(player)!.Value.WAxis.Y > -5);
        Assert.True(Run("game.state").Get("outcome") is JsonNull, "the game still runs");

        Run("play.step", """{"seconds": 0.05, "steps": 60}""");
        state = Run("game.state");
        Assert.Equal("won", state.Get("outcome")!.AsString());
        Assert.Equal(1.0, state.Get("game")!.Get("cheered")!.AsDouble());
        Assert.Equal(1, session.Console.Count(l => l.Text == "Game won."));
        // The new project's starting scene turns its cube with a visual script, which the C# engine does not compile yet.
        Assert.DoesNotContain(session.Console, l => l.Level == Level.Error && !l.Text.Contains("spin.bscript is not a C# script", StringComparison.Ordinal));

        Run("play.stop");
        Assert.Equal(edited, session.Scene.ToDoc());
        Assert.True(Run("game.state").Get("game") is JsonObject { Count: 0 });
    }

    [Fact]
    public void SavedValuesLastBetweenPlaysAndStayInsideTheConfinement()
    {
        NewProject();
        WriteInProject("counter.cs", """
            public class Counter : GameScript
            {
                protected override void Begin() => Game.SetSavedValue("plays", (Game.SavedValue("plays") ?? 0) + 1);
            }
            """);
        session.AllowScripts(true);
        Create("Counter", """{"scripts": [{"module": "counter.cs"}]}""");
        JsonValue? Plays() => Run("game.state").Get("saved")!.Get("plays");
        Assert.Null(Plays());
        for (var i = 0; i < 2; i++)
        {
            Run("play.start");
            Run("play.stop");
        }
        Assert.Equal(2.0, Plays()!.AsDouble());

        var file = Path.Join(ProjectDir, "saves", "progress.json");
        File.WriteAllText(file, "not json");
        Run("play.start");
        Run("play.stop");
        Assert.Contains(session.Console, l => l.Level == Level.Error && l.Text.Contains("could not be read", StringComparison.Ordinal));
        Assert.Equal("not json", File.ReadAllText(file));
        Assert.True(Run("game.saved.clear").Get("removed") is JsonBool { Value: true });
        Assert.False(File.Exists(file));

        var scenes = Path.Join(ProjectDir, "scenes");
        session.Confinement = Confinement.Create([ProjectDir], [scenes]).Value;
        Run("play.start");
        Run("play.stop");
        Assert.False(File.Exists(file), "nothing written outside the write folders");
        Assert.True(Said("could not be written"));
        Assert.True(Try("game.saved.clear").IsError);
        session.Confinement = null;
    }

    [Fact]
    public void InputMapsAnswerPerObjectCountTriggersOnceAndSwitch()
    {
        NewProject();
        Run("input.asset.set", """{"path": "input/car.binput", "asset": {"start": "", "maps": [{"name": "Car", "actions": [{"name": "Horn", "kind": "trigger", "bindings": ["key:H"]}]}]}}""");
        WriteInProject("reader.cs", """
            public class Reader : GameScript
            {
                [ScriptProperty] public string Prefix = "";
                protected override void Advance(float seconds)
                {
                    Game.SetValue(Prefix + "Jump", Controls.Action("Jump"));
                    Game.SetValue(Prefix + "Horn", Controls.Action("Horn"));
                }
            }
            """);
        session.AllowScripts(true);
        Create("Walker", """{"scripts": [{"module": "reader.cs", "properties": {"prefix": "walker"}}]}""");
        var driver = Create("Driver", """{"scripts": [{"module": "reader.cs", "properties": {"prefix": "driver"}}], "inputMap": {"asset": "input/car.binput", "start": ""}}""");
        double? Game(string key) => Run("game.state").Get("game")!.Get(key)?.AsDouble();

        Run("play.start");
        Run("play.input", """{"keys": ["Space", "H"]}""");
        Run("play.step");
        Assert.Equal(1.0, Game("walkerJump"));
        Assert.Equal(1.0, Game("driverHorn"));
        Assert.Equal(0.0, Game("driverJump"));
        Run("play.step");
        Assert.Equal(0.0, Game("walkerJump"));

        var state = Run("input.map.set", """{"map": "Menu"}""");
        Assert.Equal("Menu", state.Get("project")!.Get("active")!.AsString());
        Assert.Equal(driver, state.Get("objects")!.AsArray()![0].Get("object")!.AsString());
        Assert.True(Try("input.map.set", """{"map": "Nowhere"}""").IsError);
        Run("play.input", """{"keys": []}""");
        Run("play.step");
        Run("play.input", """{"keys": ["Space"]}""");
        Run("play.step");
        Assert.Equal(0.0, Game("walkerJump"));
    }

    [Fact]
    public void SetFieldChangesAComponentInPlayAndStopRestoresIt()
    {
        var id = Scripted("""
            public class Spin : GameScript
            {
                protected override void Advance(float seconds)
                {
                    Self.SetField("Point Light", "intensity", 7f);
                    if (Self.SetField("rigidBody", "mass", 3f)) Log("physics fields are set");
                }
            }
            """);
        Run("scene.component.add", $$"""{"id": "{{id}}", "component": "pointLight"}""");
        float Intensity() => session.Scene.TryGet<PointLight>(Id(id), out var l) ? l.Intensity : float.NaN;
        var before = Intensity();
        Run("play.start");
        Run("play.step", """{"seconds": 0.1, "steps": 2}""");
        Assert.Equal(7f, Intensity());
        Assert.False(Said("physics fields are set"));
        Run("play.stop");
        Assert.Equal(before, Intensity());
    }

    [Fact]
    public void ScenesLoadInPlayAddedOrInPlaceAndAddedOnesUnload()
    {
        Write("room.cs", """public class Room : GameScript { protected override void Begin() => Log("room started"); }""");
        Write("hub.cs", """public class Hub : GameScript { protected override void Begin() => Game.LoadScene("room.bscene", add: true); }""");
        session.AllowScripts(true);
        Create("Room", """{"scripts": [{"module": "room.cs"}]}""");
        SaveAs("room.bscene");
        Run("scene.new");
        Create("Hub", """{"scripts": [{"module": "hub.cs"}]}""");
        SaveAs("hub.bscene");
        session.AllowScripts(true);
        var edited = session.Scene.ToDoc();
        List<string> Names() => [.. Script.ScriptHost.Order(session.Scene).Select(i => session.Scene.NameOf(i)!)];
        int RoomStarts() => session.Console.Count(l => l.Text.Contains("room started", StringComparison.Ordinal));

        Run("play.start");
        Run("play.step", """{"seconds": 0.1}""");
        Assert.Equal(["Hub", "Room"], Names());
        Assert.Equal(1, RoomStarts());
        Assert.Equal("room.bscene", Run("play.scene.list").Get("added")!.AsArray()![0].Get("path")!.AsString());
        Assert.Equal(1.0, Run("play.scene.unload", """{"path": "room.bscene"}""").Get("removed")!.AsDouble());
        Assert.Equal(["Hub"], Names());
        Assert.True(Try("play.scene.unload", """{"path": "room.bscene"}""").IsError);

        Run("game.state.set", """{"name": "coins", "value": 3}""");
        Run("play.scene.load", """{"path": "room.bscene"}""");
        Assert.Equal(["Room"], Names());
        Assert.Equal(2, RoomStarts());
        Assert.Equal(3.0, Run("game.state").Get("game")!.Get("coins")!.AsDouble());
        Assert.True(Try("play.scene.load", """{"path": "room.bmaterial"}""").IsError);

        var outside = Path.Join(Path.GetTempPath(), $"bayleaf-scene-load-outside-{Guid.NewGuid():N}");
        Directory.CreateDirectory(outside);
        File.Copy(Path.Join(dir, "room.bscene"), Path.Join(outside, "room.bscene"));
        session.Confinement = Confinement.Create([dir], [dir]).Value;
        Assert.True(Try("play.scene.load", $$"""{"path": {{Quote($"../{Path.GetFileName(outside)}/room.bscene")}}, "additive": true}""").IsError);
        session.Confinement = null;
        Directory.Delete(outside, true);

        Run("play.stop");
        Assert.Equal(edited, session.Scene.ToDoc());
    }

    [Fact]
    public void ASplitWorldStreamsItsCellsInAndOutAroundTheCamera()
    {
        session.Use(new StreamingModule());
        string Place(double x, double z) => $$"""{"position": [{{x}}, 0, {{z}}], "rotation": [0, 0, 0], "scale": [1, 1, 1]}""";
        var camera = Create("Camera", $$$"""{"transform": {{{Place(0, 0)}}}, "camera": {"projection": "perspective", "fov": 60, "size": 10, "near": 0.1, "far": 1000, "postProcessing": ""}}""");
        Create("Tree", $$"""{"transform": {{Place(30, 0)}}}""");
        Create("Rock", $$"""{"transform": {{Place(210, 5)}}}""");
        Create("Lost", """{"streamedArea": {"scene": "cells/missing.bscene", "loadDistance": 10, "keepMargin": 1}}""");
        SaveAs("world.bscene");
        Assert.Equal(2.0, Run("stream.split", """{"cellSize": 100, "folder": "cells"}""").Get("moved")!.AsDouble());

        Run("play.start");
        Run("stream.wait", "{}");
        string State(JsonValue areas, string scene) => areas.Get("areas")!.AsArray()!.First(a => a.Get("scene")!.AsString() == scene).Get("state")!.AsString()!;
        var areas = Run("stream.areas");
        Assert.Equal("here", State(areas, "cells/cell_0_0.bscene"));
        Assert.Equal("away", State(areas, "cells/cell_2_0.bscene"));
        Assert.Equal("failed", State(areas, "cells/missing.bscene"));
        Assert.True(Said("missing.bscene could not be added"));
        var tree = Script.ScriptHost.Order(session.Scene).First(i => session.Scene.NameOf(i) == "Tree");
        var at = session.Scene.WorldMatrix(tree)!.Value.WAxis.Xyz;
        Assert.True(Math.Abs(at.X - 30) < 1e-9 && Math.Abs(at.Z) < 1e-9, "where it was before the split");

        session.Scene.Set(Id(camera), Transform.Initial with { Position = new(250, 0, 50) });
        Run("play.step", """{"seconds": 0.02}""");
        Run("stream.wait", "{}");
        areas = Run("stream.areas");
        Assert.Equal("away", State(areas, "cells/cell_0_0.bscene"));
        Assert.Equal("here", State(areas, "cells/cell_2_0.bscene"));
        Assert.Equal((0, 1), (Named("Tree"), Named("Rock")));

        Run("play.stop");
        Assert.Equal(0, Named("Rock"));
    }

    [Fact]
    public void ABuildWithoutStreamingSaysSo()
    {
        Create("Area", """{"streamedArea": {"scene": "a.bscene", "loadDistance": 10, "keepMargin": 1}}""");
        Run("play.start");
        Assert.True(Try("stream.areas").IsError);
        Assert.True(Said("no world streaming"));
    }

    [Fact]
    public void PlayMessageSendsWhilePlayingAndRefusesWhatDoesNotFit()
    {
        Write("ear.cs", """public class Ear : GameScript { protected override void Receive(in Message m) => Log($"heard {m.Name} {m.Value}"); }""");
        session.AllowScripts(true);
        var id = Create("Target", """{"scripts": [{"module": "ear.cs"}]}""");
        SaveAs("m.bscene");
        session.AllowScripts(true);
        Assert.True(Try("play.message", $$"""{"name": "jump", "id": "{{id}}"}""").IsError, "only while the game plays");
        Run("play.start");
        Run("play.message", $$"""{"name": "jump", "id": "{{id}}"}""");
        Run("play.message", """{"name": "everyone", "value": 2.5}""");
        Assert.True(Said("heard jump 0") && Said("heard everyone 2.5"));
        foreach (var bad in new[] { """{"name": ""}""", $$"""{"name": "{{new string('x', 65)}}"}""", $$"""{"name": "jump", "id": "{{StableId.Random()}}"}""" })
        {
            Assert.Equal(CommandErrorKind.BadParams, Try("play.message", bad).Error?.Kind);
        }
    }

    [Fact]
    public void AnInterfaceTellsItsScriptsWhatThePlayerDidAndShowsWhatTheySet()
    {
        NewProject();
        Run("asset.create", """{"kind": "interface", "folder": "", "name": "menu"}""");
        var layout = (JsonObject)Run("ui.layout.get", """{"path": "menu.bui"}""").Get("layout")!;
        ((JsonArray)layout.Get("root")!.Get("children")!).Add(JsonReader.Parse("""{"kind": "text", "name": "Score", "bind": "game:score"}""").Value);
        Run("ui.layout.set", $$"""{"path": "menu.bui", "layout": {{layout.ToCompact()}}}""");
        WriteInProject("menu.cs", """
            public class Menu : GameScript
            {
                protected override void Receive(in Message message)
                {
                    if (message.Is("pressed:Start")) Self.SetInterfaceText("Title", "Started");
                }
            }
            """);
        session.AllowScripts(true);
        var id = Create("Menu", """{"interface": {"layout": "menu.bui", "visible": true, "order": 0}, "scripts": [{"module": "menu.cs"}]}""");
        string TextOf(string name) => Run("play.ui.state", $$"""{"id": "{{id}}"}""").Get("elements")!.AsArray()!.First(e => e.Get("name")!.AsString() == name).Get("text")!.AsString()!;

        Assert.True(Try("play.ui.state", $$"""{"id": "{{id}}"}""").IsError, "only while playing");
        Run("play.start");
        Assert.Equal("My Game", TextOf("Title"));
        Run("game.state.set", """{"name": "score", "value": 7}""");
        Assert.Equal("7", TextOf("Score"));
        Run("play.ui.press", $$"""{"id": "{{id}}", "element": "Start"}""");
        Assert.Equal("Started", TextOf("Title"));
        Assert.True(Try("play.ui.press", $$"""{"id": "{{id}}", "element": "Title"}""").IsError, "text is not pressed");
        Assert.True(Try("play.ui.press", $$"""{"id": "{{id}}", "element": "Nothing"}""").IsError);
        Run("play.stop");
        Run("play.start");
        Assert.Equal("My Game", TextOf("Title"));
    }

    [Fact]
    public void PlayReadsTheDataFilesAsSavedAndNeverWritesThem()
    {
        NewProject();
        Directory.CreateDirectory(Path.Join(ProjectDir, "data"));
        Run("data.shape.set", """{"path": "data/enemy.bshape", "shape": {"fields": [{"name": "health", "type": "number", "default": 10}, {"name": "loot", "type": "list:text", "default": {"list": []}}]}}""");
        Run("data.set", """{"path": "data/goblin.bdata", "data": {"shape": "data/enemy.bshape", "values": {"health": 12, "loot": {"list": ["gold", "gem"]}}}}""");
        var written = File.ReadAllText(Path.Join(ProjectDir, "data/goblin.bdata"));
        WriteInProject("reader.cs", """
            public class Reader : GameScript
            {
                protected override void Begin()
                {
                    var goblin = Game.Data("data/goblin.bdata");
                    Log($"health {goblin.Number("health")}, {goblin.Count("loot")} loot, then {goblin.TextAt("loot", 1)}; nothing: {Game.Data("data/none.bdata").Number("health") is null}");
                }
            }
            """);
        session.AllowScripts(true);
        Create("Reader", """{"scripts": [{"module": "reader.cs"}]}""");
        Run("play.start");
        Assert.True(Said("health 12, 2 loot, then gem; nothing: True"), string.Join("\n", session.Console.Select(l => l.Text)));
        Run("play.stop");
        Assert.Equal(written, File.ReadAllText(Path.Join(ProjectDir, "data/goblin.bdata")));
    }

    [Fact]
    public void ANewScriptFromTheTemplateCompilesRunsAndIsDescribed()
    {
        NewProject();
        var made = Run("script.new", """{"folder": "scripts", "name": "Turn Table"}""");
        Assert.EndsWith("turn_table.cs", made.Get("path")!.AsString(), StringComparison.Ordinal);
        Assert.True(Try("script.new", """{"folder": "scripts", "name": "turn table"}""").IsError, "the name is taken");
        Assert.Contains("1 script files compiled", Run("script.build", """{"path": "scripts/turn_table.cs"}""").Get("log")!.AsString(), StringComparison.Ordinal);
        var info = Run("asset.info", """{"path": "scripts/turn_table.cs"}""");
        Assert.Equal("Script", info.Get("kind")!.AsString());
        Assert.Contains("speed (number) = 1", info.ToCompact(), StringComparison.Ordinal);
        var asset = Run("asset.create", """{"kind": "script", "folder": "scripts", "name": "Door"}""");
        Assert.EndsWith("door.cs", asset.Get("module")!.AsString(), StringComparison.Ordinal);

        session.AllowScripts(true);
        var id = Create("Table", """{"scripts": [{"module": "scripts/turn_table.cs"}]}""");
        Run("play.start");
        Run("play.step", """{"seconds": 0.125}""");
        Assert.True(Math.Abs(RotationY(id) - 45f) < 1e-3f, $"the template turns one turn per second: {RotationY(id)}");
        Run("play.stop");

        File.WriteAllText(Path.Join(ProjectDir, "scripts/door.cs"), "public class Door : GameScript { void X() { nope(); } }");
        var broken = Try("script.build", """{"path": "scripts/door.cs"}""");
        Assert.Contains("door.cs(1,", broken.Error?.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void AChangedScriptReloadsWhilePlayingAndKeepsWhatPlayChanged()
    {
        var id = Scripted(Spinner);
        Run("play.start");
        Run("play.step", """{"seconds": 0.5}""");
        var turned = RotationY(id);
        Write("spin.cs", Spinner.Replace("Speed * seconds", "Speed * seconds * 100", StringComparison.Ordinal));
        File.SetLastWriteTimeUtc(Path.Join(dir, "spin.cs"), DateTime.UtcNow.AddSeconds(10));
        Run("play.step", """{"seconds": 0.5}""");
        Assert.True(Said("Reloaded the project's scripts"));
        Assert.True(session.Console.Count(l => l.Text.Contains("spinning", StringComparison.Ordinal)) >= 2, "its Begin ran again");
        Run("play.step", """{"seconds": 0.1}""");
        Assert.True(RotationY(id) > turned + 500f, $"the new code runs from where play had turned it: {RotationY(id)}");

        Write("spin.cs", "public class Spin : GameScript { void X() { nope(); } }");
        File.SetLastWriteTimeUtc(Path.Join(dir, "spin.cs"), DateTime.UtcNow.AddSeconds(20));
        Run("play.step", """{"seconds": 0.5}""");
        Assert.Equal(1, session.Console.Count(l => l.Text.Contains("do not compile, so the running ones keep running", StringComparison.Ordinal)));
        Run("play.step", """{"seconds": 0.5}""");
        Assert.Equal(1, session.Console.Count(l => l.Text.Contains("do not compile, so the running ones keep running", StringComparison.Ordinal)));
    }

    [Fact]
    public void AKeyframeAnimationPlaysWhenAskedAndStopsWhereItIs()
    {
        var id = Create("Lift");
        Write("lift.btimeline", $$"""
            {"format": "bayleaf.timeline", "version": 1, "prefab": "lift.bprefab", "length": 1, "parts": [{"id": "{{id}}", "tracks": [
              {"target": {"component": "transform", "field": "position"}, "keys": [{"time": 0, "value": [0, 0, 0], "ease": "linear"}, {"time": 1, "value": [0, 4, 0]}]}]}]}
            """);
        Run("scene.component.add", $$"""{"id": "{{id}}", "component": "keyframeAnimation"}""");
        Run("scene.component.set", $$$"""{"id": "{{{id}}}", "component": "keyframeAnimation", "value": {"animation": "lift.btimeline", "playOnStart": false, "speed": 1}}""");
        SaveAs("lift.bscene");
        double Height() => session.Scene.TryGet<Transform>(Id(id), out var t) ? t.Position.Y : double.NaN;
        Assert.True(Try("timeline.play", $$"""{"id": "{{id}}"}""").IsError, "only in play mode");
        Run("play.start");
        Run("play.step", """{"seconds": 0.25}""");
        Assert.Equal(0.0, Height());
        Run("timeline.play", $$"""{"id": "{{id}}"}""");
        Run("play.step", """{"seconds": 0.5}""");
        Assert.True(Math.Abs(Height() - 2) < 1e-3, $"half way up: {Height()}");
        Run("timeline.stop", $$"""{"id": "{{id}}"}""");
        Run("play.step", """{"seconds": 0.25}""");
        Assert.True(Math.Abs(Height() - 2) < 1e-3, "it stays where it stopped");
        Run("play.stop");
        Assert.Equal(0.0, Height());
    }
}

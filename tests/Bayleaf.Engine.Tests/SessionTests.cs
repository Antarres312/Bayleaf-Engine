using Bayleaf.Core.Json;

namespace Bayleaf.Engine.Tests;

public class SessionTests
{
    private static JsonValue Json(string text) => JsonReader.Parse(text).Value;

    private static (CommandRegistry Registry, Session Session) Fresh() => (CommandRegistry.WithBuiltins(), new Session());

    private static string Create(CommandRegistry registry, Session session, string name) =>
        registry.Execute(session, "scene.object.create", Json($"{{\"name\": \"{name}\"}}")).Value.Get("id")!.AsString()!;

    [Fact]
    public void EachChangeIsOneUndoStepAndUndoAndRedoWalkThem()
    {
        var (registry, session) = Fresh();
        var a = Create(registry, session, "A");
        Create(registry, session, "B");
        Assert.Equal(["scene.object.create", "scene.object.create"], session.History);
        Assert.True(session.Dirty);
        registry.Execute(session, "edit.undo", JsonValue.Null);
        Assert.Equal(1, session.Scene.ObjectCount);
        registry.Execute(session, "edit.undo", JsonValue.Null);
        Assert.Equal(0, session.Scene.ObjectCount);
        Assert.True(registry.Execute(session, "edit.undo", JsonValue.Null).IsError, "nothing left to undo");
        registry.Execute(session, "edit.redo", JsonValue.Null);
        Assert.Equal("A", session.Scene.NameOf(Core.StableId.ParseHex(a).Value));
        Create(registry, session, "C");
        Assert.False(session.CanRedo, "a new change drops what could be redone");
    }

    [Fact]
    public void AFailedCommandLeavesTheSceneAsItWasAndRecordsNoStep()
    {
        var (registry, session) = Fresh();
        var a = Create(registry, session, "A");
        var before = session.Scene.ToDoc();
        var steps = session.History.Count;
        var failed = registry.Execute(session, "scene.object.create", Json($"{{\"parent\": \"{a}\", \"components\": {{\"camera\": {{\"projection\": \"perspective\", \"fov\": 0, \"size\": 5, \"near\": 0.1, \"far\": 9}}}}}}"));
        Assert.True(failed.IsError);
        Assert.Equal(before, session.Scene.ToDoc());
        Assert.Equal(steps, session.History.Count);
    }

    [Fact]
    public void RunsWithOneMergeKeyShareAnUndoStep()
    {
        var (registry, session) = Fresh();
        var a = Create(registry, session, "A");
        foreach (var x in new[] { 1, 2, 3 })
        {
            registry.ExecuteMerging(session, "scene.component.set", Json($"{{\"id\": \"{a}\", \"component\": \"transform\", \"value\": {{\"position\": [{x}, 0, 0], \"rotation\": [0, 0, 0], \"scale\": [1, 1, 1]}}}}"), "drag");
        }
        Assert.Equal(2, session.History.Count);
        registry.Execute(session, "edit.undo", JsonValue.Null);
        Assert.Equal(0.0, session.Scene.Get<Scene.Components.Transform>(Core.StableId.ParseHex(a).Value).Value.Position.X);
    }

    [Fact]
    public void ParamsThatAreNotAnObjectAndUnknownCommandsAreRefused()
    {
        var (registry, session) = Fresh();
        Assert.Equal(CommandErrorKind.ParamsNotObject, registry.Execute(session, "scene.objects", Json("[1]")).Error.Kind);
        Assert.Equal(CommandErrorKind.Unknown, registry.Execute(session, "scene.nothing", JsonValue.Null).Error.Kind);
        Assert.Equal(CommandErrorKind.Disabled, registry.Execute(session, "scene.save", JsonValue.Null).Error.Kind);
        Assert.Equal(CommandErrorKind.BadParams, registry.Execute(session, "scene.object.create", Json("{\"nmae\": \"A\"}")).Error.Kind);
    }

    [Fact]
    public void CommandsNotPortedYetAreListedButDisabledWithAReason()
    {
        var (registry, session) = Fresh();
        var render = registry.List(session).First(r => r.Get("id")?.AsString() == "scene.render");
        Assert.Equal(JsonValue.From(false), render.Get("enabled"));
        Assert.Contains("not in the C# engine yet", render.Get("disabledReason")!.AsString(), StringComparison.Ordinal);
    }
}

public class ConfinementTests
{
    private static (string Base, string Read, string Write) Roots(string name)
    {
        var baseDir = Path.Join(Path.GetTempPath(), $"bayleaf-confine-cs-{name}-{Environment.ProcessId}");
        var read = Path.Join(baseDir, "read");
        var write = Path.Join(baseDir, "write");
        Directory.CreateDirectory(read);
        Directory.CreateDirectory(Path.Join(write, "sub"));
        return (baseDir, read, write);
    }

    [Fact]
    public void AllowsInsideAndRefusesOutsideWithAReason()
    {
        var (baseDir, read, write) = Roots("inside");
        try
        {
            var c = Confinement.Create([read], [write]).Value;
            Assert.True(c.Check(Path.Join(read, "a.bscene"), Access.Read).IsOk);
            Assert.True(c.Check(Path.Join(write, "sub", "new.png"), Access.Write).IsOk);
            Assert.True(c.Check(Path.Join(write, "x.bscene"), Access.Read).IsOk, "a write root can be read");
            var refused = c.Check(Path.Join(read, "a.bscene"), Access.Write);
            Assert.Contains("may not write", refused.Error, StringComparison.Ordinal);
            Assert.Contains("outside", refused.Error, StringComparison.Ordinal);
            Assert.True(c.Check(Path.Join(baseDir, "elsewhere.txt"), Access.Read).IsError);
            Assert.True(c.Check(Path.Join(read + "-twin", "a.bscene"), Access.Read).IsError, "a folder that only starts like a root is outside it");
        }
        finally
        {
            Directory.Delete(baseDir, true);
        }
    }

    [Fact]
    public void DotDotCannotClimbOutAndRelativePathsUseTheWriteRoot()
    {
        var (baseDir, read, write) = Roots("dotdot");
        try
        {
            var c = Confinement.Create([read], [write]).Value;
            Assert.True(c.Check("../../escape.txt", Access.Write).IsError);
            Assert.True(c.Check(Path.Join(write, "sub", "..", "..", "escape.txt"), Access.Write).IsError);
            var resolved = c.Check("sub/out.png", Access.Write).Value;
            Assert.EndsWith(Path.Join("sub", "out.png"), resolved, StringComparison.Ordinal);
        }
        finally
        {
            Directory.Delete(baseDir, true);
        }
    }

    [Fact]
    public void BothListsAreRequiredAndMustBeFolders()
    {
        var (baseDir, read, write) = Roots("required");
        try
        {
            Assert.True(Confinement.Create([], [write]).IsError);
            Assert.True(Confinement.Create([read], []).IsError);
            Assert.True(Confinement.Create([read], [Path.Join(baseDir, "missing")]).IsError);
        }
        finally
        {
            Directory.Delete(baseDir, true);
        }
    }

    [Fact]
    public void AConfinedSessionRefusesToSaveOutsideItsFolders()
    {
        var (baseDir, read, write) = Roots("session");
        try
        {
            var registry = CommandRegistry.WithBuiltins();
            var session = new Session { Confinement = Confinement.Create([read], [write]).Value };
            Assert.True(registry.Execute(session, "scene.saveAs", JsonReader.Parse("{\"path\": \"../outside/y.bscene\"}").Value).IsError, ".. cannot climb out");
            Assert.True(registry.Execute(session, "scene.saveAs", JsonReader.Parse("{\"path\": \"inside.bscene\"}").Value).IsOk);
            Assert.True(File.Exists(Path.Join(write, "inside.bscene")));
        }
        finally
        {
            Directory.Delete(baseDir, true);
        }
    }
}

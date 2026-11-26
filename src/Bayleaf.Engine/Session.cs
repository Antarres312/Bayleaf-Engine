using Bayleaf.Core.Json;
using Bayleaf.Scene;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Engine;

/// <summary>How important a console line is.</summary>
public enum Level
{
    Info,
    Error,
}

/// <summary>One line in the editor console: script output and play-mode messages.</summary>
public sealed record ConsoleLine(Level Level, string Text);

/// <summary>
/// State that commands read and change: the open scene, its file, the undo history, the console and
/// (when automation drives it) the folders it may read and write.
/// </summary>
public sealed partial class Session
{
    /// <summary>Undo and redo together keep at most this many bytes of snapshots; the oldest steps go first.</summary>
    public const long UndoBudgetBytes = 256L << 20;

    /// <summary>Most undo steps kept.</summary>
    public const int MaxUndoSteps = 500;

    /// <summary>Console lines kept; older ones are dropped.</summary>
    private const int ConsoleLimit = 2000;

    /// <summary>What an undo step restores: a document as compact JSON.</summary>
    internal enum SnapshotKind
    {
        Scene,
        Timeline,

        /// <summary>A graph, by its id among the open graphs (<see cref="Snapshot.Graph"/>).</summary>
        Graph,
    }

    /// <param name="Graph">For a graph's step: the open graph's id.</param>
    internal sealed record Snapshot(SnapshotKind Kind, string Text, ulong Graph = 0);

    internal sealed record HistoryStep(string Command, Snapshot Before, string? MergeKey);

    private readonly List<ConsoleLine> console = [];

    internal List<HistoryStep> Undo { get; set; } = [];

    internal List<HistoryStep> Redo { get; set; } = [];

    /// <summary>Prefabs open on their own, the innermost last, each with the scene it stands in front of.</summary>
    internal List<PrefabEdit> PrefabEdits { get; } = [];

    public SceneModel Scene { get; internal set; } = new();

    /// <summary>File the scene was opened from or last saved to.</summary>
    public string? ScenePath { get; private set; }

    /// <summary>The project folder the open scene belongs to: the nearest folder above it with a <c>.bproject</c> file.</summary>
    public string? ProjectDir { get; private set; }

    /// <summary>True when the scene has changes that are not saved.</summary>
    public bool Dirty { get; internal set; }

    /// <summary>Set while an automation request runs: the folders it may read and write.</summary>
    public Confinement? Confinement { get; set; }

    /// <summary>True while play mode runs (also when paused).</summary>
    public bool IsPlaying => Play is not null;

    /// <summary>Play mode's state while it runs (<c>Play.cs</c>).</summary>
    internal PlayState? Play { get; set; }

    public IReadOnlyList<ConsoleLine> Console => console;

    public void ClearConsole() => console.Clear();

    /// <summary>Adds a line to the console.</summary>
    public void Log(Level level, string text)
    {
        console.Add(new ConsoleLine(level, text));
        if (console.Count > ConsoleLimit)
        {
            console.RemoveRange(0, console.Count - ConsoleLimit);
        }
    }

    /// <summary>Ids of the commands that can be undone, oldest first.</summary>
    public IReadOnlyList<string> History => Undo.Select(s => s.Command).ToList();

    public bool CanUndo => Undo.Count > 0;

    public bool CanRedo => Redo.Count > 0;

    /// <summary>Bytes the undo and redo steps keep.</summary>
    public long HistoryBytes => Undo.Concat(Redo).Sum(s => (long)s.Before.Text.Length);

    /// <summary>The keyframe animation the <c>timeline.*</c> commands work on.</summary>
    internal TimelineDoc? Timeline { get; set; }

    /// <summary>The prefabs open on their own, outermost first, by their paths in the project.</summary>
    public IReadOnlyList<string> OpenPrefabs => PrefabEdits.Select(e => e.Source).ToList();

    /// <summary>For commands that need the scene's own state: null while it is free.</summary>
    public static string? Always(Session session) => null;

    /// <summary>For commands that would lose or save play-mode state: scene files, undo and redo.</summary>
    public static string? NotPlaying(Session session) => session.IsPlaying ? "stop play mode first" : null;

    /// <summary>The path a command may use for <paramref name="access"/>: unchanged without confinement, resolved and checked with it.</summary>
    internal string Allowed(string path, Access access)
    {
        if (Confinement is null)
        {
            return path;
        }
        var checkedPath = Confinement.Check(path, access);
        return checkedPath.IsOk ? checkedPath.Value : throw Stop.Failed(checkedPath.Error);
    }

    /// <summary>The folder paths in the scene start from: the project folder, or the scene's own folder outside a project.</summary>
    public string ScriptDir
    {
        get
        {
            if (ProjectDir is not null)
            {
                return ProjectDir;
            }
            return ScenePath is not null && Core.Files.Parent(ScenePath) is { } dir ? dir : ".";
        }
    }

    internal void SetScenePath(string? path)
    {
        ProjectDir = path is null ? null : ProjectCommands.ProjectDirOf(path);
        ScenePath = path;
    }

    /// <summary>Refuses a command that needs the scene itself while a prefab is open.</summary>
    internal void NoPrefabOpen(string what)
    {
        if (PrefabEdits.Count > 0)
        {
            throw Stop.Failed($"{what}: the prefab {PrefabEdits[^1].Source} is open; save it and go back first (prefab.close)");
        }
    }

    /// <summary>
    /// Opens <paramref name="scene"/> in place of the current one. Prefab copies update from their
    /// files; the problems with files that cannot be read are returned.
    /// </summary>
    internal List<string> ReplaceScene(SceneModel scene, string? path)
    {
        Scene = scene;
        SetScenePath(path);
        var problems = PrefabCommands.RefreshAll(this);
        Dirty = false;
        // An older scene's Environment moves into a lighting asset; the scene is then changed.
        if (LightingCommands.MoveEnvironment(this) is var (level, message, changed))
        {
            Dirty = changed;
            Log(level, message);
            problems.Add(message);
        }
        Forget(SnapshotKind.Scene);
        return problems;
    }

    /// <summary>The scene as an undo step keeps it: its document as compact JSON.</summary>
    internal Snapshot SceneSnapshot() => new(SnapshotKind.Scene, Scene.ToDoc().ToJson().ToCompact());

    /// <summary>The open timeline as an undo step keeps it; null without one.</summary>
    internal Snapshot? TimelineSnapshot() => Timeline is null ? null : new(SnapshotKind.Timeline, Timeline.Timeline.ToJson());

    /// <summary>Drops the undo and redo steps of one kind.</summary>
    internal void Forget(SnapshotKind kind)
    {
        Undo.RemoveAll(s => s.Before.Kind == kind);
        Redo.RemoveAll(s => s.Before.Kind == kind);
    }

    /// <summary>Puts a snapshot's document back as it was, with nothing else (a failed command's undoing).</summary>
    internal void PutBack(Snapshot snapshot)
    {
        if (snapshot.Kind == SnapshotKind.Graph)
        {
            if (Graph is { } open && GraphCommands.FromSnapshot(snapshot, open.Catalog) is { IsOk: true } graph)
            {
                open.Graph = graph.Value;
            }
            return;
        }
        if (snapshot.Kind == SnapshotKind.Timeline)
        {
            if (Timeline is not null && Bayleaf.Scene.Timeline.FromJson(snapshot.Text) is { IsOk: true } restored)
            {
                Timeline.Timeline = restored.Value;
            }
            return;
        }
        if (SceneDoc.FromText(snapshot.Text) is { IsOk: true } doc && SceneModel.FromDoc(doc.Value) is { IsOk: true } scene)
        {
            Scene = scene.Value;
        }
    }

    /// <summary>Puts a snapshot back and gives the state it replaced, for the opposite stack.</summary>
    internal Snapshot Restore(Snapshot snapshot)
    {
        if (snapshot.Kind == SnapshotKind.Graph)
        {
            return RestoreGraph(snapshot);
        }
        if (snapshot.Kind == SnapshotKind.Timeline)
        {
            var doc = Timeline ?? throw Stop.Failed("the timeline of this step is closed");
            var current = TimelineSnapshot()!;
            var restored = Bayleaf.Scene.Timeline.FromJson(snapshot.Text);
            if (restored.IsError)
            {
                throw Stop.Failed($"the undo step cannot be read: {restored.Error}");
            }
            doc.Timeline = restored.Value;
            doc.Playhead = Math.Min(doc.Playhead, doc.Timeline.Length);
            doc.Dirty = true;
            TimelineCommands.AfterChange(this);
            return current;
        }
        var scene = SceneSnapshot();
        var parsed = SceneDoc.FromText(snapshot.Text);
        if (parsed.IsError)
        {
            throw Stop.Failed($"the undo step cannot be read: {parsed.Error}");
        }
        Scene = SceneModel.FromDoc(parsed.Value).OrFail();
        return scene;
    }
    /// <summary>Records an undo step for <paramref name="command"/>; with a merge key equal to the last step's, the last step stays.</summary>
    internal void Record(string command, Snapshot before, string? mergeKey)
    {
        var merges = mergeKey is not null && Undo.Count > 0 && Undo[^1].MergeKey == mergeKey;
        Redo.Clear();
        if (!merges)
        {
            Undo.Add(new HistoryStep(command, before, mergeKey));
            TrimHistory();
        }
    }

    /// <summary>Drops the oldest undo steps beyond the step and byte limits. The newest step always stays.</summary>
    private void TrimHistory()
    {
        var total = HistoryBytes;
        var drop = 0;
        while (Undo.Count - drop > 1 && (Undo.Count - drop > MaxUndoSteps || total > UndoBudgetBytes))
        {
            total -= Undo[drop].Before.Text.Length;
            drop++;
        }
        Undo.RemoveRange(0, drop);
    }
}

/// <summary>Reads command params: an object (null counts as one with no fields) with only the fields a command knows.</summary>
internal static class Params
{
    public static JsonFields Of(JsonValue parameters, params string[] known) =>
        JsonFields.Of(parameters is JsonNull ? new JsonObject() : parameters, "struct params", true, known);

    /// <summary>Params that must be empty.</summary>
    public static void None(JsonValue parameters) => Of(parameters);

    public static int Index(JsonValue value)
    {
        if (value is JsonNumber n && n.AsUInt64() is { } whole)
        {
            return whole <= int.MaxValue ? (int)whole : int.MaxValue;
        }
        throw new JsonShapeException($"invalid type: {JsonRead.Describe(value)}, expected usize");
    }

    public static Core.StableId? OptionalId(JsonFields f, string name) => f.NullableValue(name, JsonRead.Id);
}

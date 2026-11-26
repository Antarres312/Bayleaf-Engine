using System.Globalization;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;
using Bayleaf.Script;

using SceneModel = Bayleaf.Scene.Scene;
using Transform = Bayleaf.Scene.Components.Transform;

namespace Bayleaf.Engine;

/// <summary>Play mode while it runs: the scene as it was before (put back on stop), the scripts and what play keeps apart from the files.</summary>
internal sealed class PlayState(SceneDoc snapshot, ScriptHost host)
{
    /// <summary>The scene as it was when play mode started; restored on stop.</summary>
    public SceneDoc Snapshot { get; } = snapshot;

    public ScriptHost Host { get; set; } = host;

    public double Time { get; set; }

    public bool Paused { get; set; }

    public float SinceReloadCheck { get; set; }

    /// <summary>Whether the end of the game was already announced.</summary>
    public bool ToldOutcome { get; set; }

    /// <summary>The saved values as the project holds them; null when its file could not be read, so it is not written over.</summary>
    public SortedDictionary<string, double>? SavedOnDisk { get; set; }

    /// <summary>Scenes added while playing, with their top objects, in load order.</summary>
    public List<(string Path, IReadOnlyList<StableId> Roots)> Loaded { get; } = [];

    /// <summary>Input Map assets read this play, by path: their maps, or why they could not be read.</summary>
    public Dictionary<string, Result<ActionMaps, string>> InputAssets { get; set; } = new(StringComparer.Ordinal);

    /// <summary>What scripts and the player changed of each object's Interface.</summary>
    public Dictionary<StableId, InterfaceState> Interfaces { get; } = [];

    /// <summary>Each Interface's layout as read this play (its parts in place), by object, with the file it came from; null when it did not read.</summary>
    public Dictionary<StableId, (string Layout, GameInterface.Element? Root)> InterfaceRoots { get; } = [];

    /// <summary>The Keyframe Animations playing.</summary>
    public TimelinePlayers Timelines { get; } = new();

    /// <summary>The scripts as last compiled for this play (also when that compile failed), to see when they change again.</summary>
    public ScriptProject? SeenScripts { get; set; }

    /// <summary>Things play mode says once per play.</summary>
    public HashSet<string> Told { get; } = new(StringComparer.Ordinal);
}

// Play mode in the session (docs/automation.md, the play.* commands): what scripts read of the player's
// input, the opt-in modules, the consent to run a project's scripts, and the play loop.
public sealed partial class Session
{
    private readonly List<IPlayModule> modules = [];
    private readonly HashSet<string> allowedScripts = new(StringComparer.OrdinalIgnoreCase);
    private ScriptProject? compiledScripts;

    /// <summary>What scripts see as held: keys, the pointer, mouse buttons and the gamepad, until the next <c>play.input</c> or editor frame.</summary>
    public PlayInput PlayInput { get; set; } = PlayInput.None;

    /// <summary>The longest one play step's scripts may take before the watchdog stops play mode.</summary>
    public TimeSpan ScriptTimeLimit { get; set; } = ScriptHost.DefaultTimeLimit;

    public bool IsPaused => Play?.Paused ?? false;

    /// <summary>Seconds since play mode started; 0 outside it.</summary>
    public double PlayTime => Play?.Time ?? 0;

    /// <summary>The opt-in modules play mode runs, in the order given.</summary>
    public IReadOnlyList<IPlayModule> Modules => modules;

    /// <summary>Turns an opt-in module on for this session's play mode (gameplay roles, world streaming).</summary>
    public void Use(IPlayModule module) => modules.Add(module);

    /// <summary>The editor's frame: play mode moves on by <paramref name="dt"/> seconds unless it is paused.</summary>
    public void Tick(float dt)
    {
        if (Play is { Paused: false })
        {
            PlayLoop.Step(this, dt);
        }
    }

    /// <summary>The folder whose scripts a consent is for: the project folder, or the scene's folder outside a project.</summary>
    private string ScriptFolder => Path.GetFullPath(ScriptDir).TrimEnd(Path.DirectorySeparatorChar);

    /// <summary>
    /// Whether this project's C# scripts may run in play mode. A C# script runs in the editor's process
    /// with nothing around it, so opening a project for the first time asks the person first (as a code
    /// editor asks before trusting a folder); until they say yes, play mode runs without the scripts and
    /// says so. The editor asks and remembers the answer; the CLI takes <c>--allow-scripts</c>; agents
    /// cannot give it (<c>project.scripts.allow</c> refuses them).
    /// </summary>
    public bool ScriptsAllowed => allowedScripts.Contains(ScriptFolder);

    /// <summary>Gives (or takes back) the consent to run the open project's scripts.</summary>
    public void AllowScripts(bool allow)
    {
        if (allow)
        {
            allowedScripts.Add(ScriptFolder);
        }
        else
        {
            allowedScripts.Remove(ScriptFolder);
        }
    }

    /// <summary>The project's scripts compiled, again when their files changed since. Runs no script code.</summary>
    internal ScriptProject CompiledScripts()
    {
        var folder = ScriptFolder;
        if (compiledScripts is null || !string.Equals(compiledScripts.Folder, folder, StringComparison.OrdinalIgnoreCase) || compiledScripts.Changed())
        {
            compiledScripts = ScriptProject.Compile(folder);
        }
        return compiledScripts;
    }
}

/// <summary>The play loop: starting, stepping, stopping, and doing what scripts asked for after each step.</summary>
internal static partial class PlayLoop
{
    /// <summary>How often play mode looks for changed scripts, in simulated seconds.</summary>
    private const float ReloadCheckInterval = 0.5f;

    /// <summary>Longest simulation step; longer frames are split.</summary>
    public const float MaxStep = 0.1f;

    /// <summary>Most objects a scene may hold while scripts spawn more.</summary>
    public const int MaxObjects = 100_000;

    /// <summary>Under automation every script module of <paramref name="scene"/> must be inside the readable folders.</summary>
    public static void ReadableModules(Session session, SceneModel scene)
    {
        foreach (var id in ScriptHost.Order(scene))
        {
            if (scene.TryGet<Scripts>(id, out var list))
            {
                foreach (var script in list.Items)
                {
                    session.Allowed(Path.Combine(session.ScriptDir, script.Module), Access.Read);
                }
            }
        }
    }

    /// <summary>How many of the scene's scripts are C# scripts.</summary>
    private static int CSharpScripts(SceneModel scene) =>
        ScriptHost.Order(scene).Sum(id => scene.TryGet<Scripts>(id, out var list) ? list.Items.Count(s => ScriptProject.IsScript(s.Module)) : 0);

    /// <summary>
    /// A script host for the playing scene, with the project's Input Map and objects' own and its data
    /// files, before any script starts, and what could not be read. The project's scripts are compiled
    /// (again when they changed) and loaded when the scene has C# scripts and the person allowed them.
    /// </summary>
    public static (ScriptHost Host, ScriptProject? Seen, List<string> Problems) FreshHost(Session session, PlayState? play)
    {
        var problems = new List<string>();
        LoadedScripts? loaded = null;
        ScriptProject? seen = null;
        var wanted = CSharpScripts(session.Scene);
        if (wanted > 0 && !session.ScriptsAllowed)
        {
            problems.Add(string.Create(CultureInfo.InvariantCulture,
                $"Scripts did not run: {wanted} C# scripts wait until the person at the editor allows this project's scripts to run (the editor asks when a project is first opened; Project > Allow Scripts, or bayleaf-cli --allow-scripts)."));
        }
        else if (wanted > 0)
        {
            seen = session.CompiledScripts();
            if (!seen.Compiled)
            {
                problems.Add($"Scripts did not run: the project's scripts do not compile: {seen.Summary()}");
            }
            else
            {
                var load = seen.Load();
                if (load.IsOk)
                {
                    loaded = load.Value;
                }
                else
                {
                    problems.Add($"Scripts did not run: {load.Error}");
                }
            }
        }
        var host = new ScriptHost(loaded) { TimeLimit = session.ScriptTimeLimit, Held = session.PlayInput };
        var project = ProjectInput(session);
        if (project.IsOk)
        {
            host.Input.Project = project.Value is { } maps ? new ActionMaps.State(maps) : null;
        }
        else
        {
            problems.Add($"The project's Input Map: {project.Error}");
        }
        var assets = play?.InputAssets ?? new Dictionary<string, Result<ActionMaps, string>>(StringComparer.Ordinal);
        problems.AddRange(SyncObjectInput(session, host, assets));
        var (data, dataProblems) = LoadData(session);
        host.Data = data;
        problems.AddRange(dataProblems);
        return (host, seen, problems);
    }

    /// <summary>The project's Input Map: the asset Project Settings names, actions still in an old project file, or for a scene outside any project the usual controls. Null when the project has none.</summary>
    private static Result<ActionMaps?, string> ProjectInput(Session session)
    {
        if (session.ProjectDir is null)
        {
            return Maps(InputAsset.Starter());
        }
        var settings = ProjectCommands.SettingsOf(session).Input;
        if (settings.Map.Length == 0)
        {
            return settings.LegacyAsset() is { } legacy ? Maps(legacy) : (ActionMaps?)null;
        }
        var read = InputCommands.ReadAsset(session, settings.Map);
        return read.IsOk ? Maps(read.Value) : Result.Fail(read.Error);
    }

    private static Result<ActionMaps?, string> Maps(InputAsset asset)
    {
        var names = asset.CheckedMaps();
        if (names.IsError)
        {
            return Result.Fail(names.Error);
        }
        var specs = asset.Maps.SelectMany(m => m.Actions.Select(a => new ActionSpec(m.Name, a.Name, (ActionMode)(int)a.Kind, a.Bindings)));
        return ActionMaps.Of(names.Value, asset.Start, specs);
    }

    /// <summary>Gives each object with an Input Map component its own map state and drops those of objects that went; what went wrong reading assets or starting maps.</summary>
    public static List<string> SyncObjectInput(Session session, ScriptHost host, Dictionary<string, Result<ActionMaps, string>> assets)
    {
        var problems = new List<string>();
        var scene = session.Scene;
        var wanted = new List<(StableId Id, InputMap Component)>();
        foreach (var id in ScriptHost.Order(scene))
        {
            if (scene.TryGet<InputMap>(id, out var map) && map.Asset.Length > 0)
            {
                wanted.Add((id, map));
            }
        }
        foreach (var gone in host.Input.Objects.Keys.Where(id => !wanted.Any(w => w.Id == id)).ToList())
        {
            host.Input.Objects.Remove(gone);
        }
        foreach (var (id, component) in wanted)
        {
            if (host.Input.Objects.ContainsKey(id))
            {
                continue;
            }
            if (!assets.TryGetValue(component.Asset, out var maps))
            {
                var read = InputCommands.ReadAsset(session, component.Asset);
                maps = read.IsOk && Maps(read.Value) is { IsOk: true } made ? made.Value! : Result.Fail(read.IsOk ? Maps(read.Value).Error : read.Error);
                assets[component.Asset] = maps;
                if (maps.IsError)
                {
                    problems.Add($"Input Map {component.Asset}: {maps.Error}");
                }
            }
            if (maps.IsError)
            {
                continue;
            }
            var state = new ActionMaps.State(maps.Value);
            if (component.Start.Length > 0 && !state.SetActive(component.Start))
            {
                problems.Add($"[{scene.NameOf(id) ?? "Object"}] Input Map {component.Asset} has no map {component.Start}; it starts with {state.Active}");
            }
            host.Input.Objects[id] = state;
        }
        return problems;
    }

    /// <summary>Most data files play reads.</summary>
    private const int MaxDataFiles = 4096;

    /// <summary>Every data file of the project as it was saved, by its path relative to the project folder; play never writes them.</summary>
    public static (Dictionary<string, SortedDictionary<string, ResolvedField>> Data, List<string> Problems) LoadData(Session session)
    {
        var data = new Dictionary<string, SortedDictionary<string, ResolvedField>>(StringComparer.Ordinal);
        var problems = new List<string>();
        var root = session.ScriptDir;
        var pending = new Stack<string>([root]);
        while (pending.TryPop(out var dir))
        {
            try
            {
                session.Allowed(dir, Access.Read);
            }
            catch (CommandException)
            {
                continue;
            }
            IEnumerable<string> entries;
            try
            {
                entries = Directory.EnumerateFileSystemEntries(dir).ToList();
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                continue;
            }
            foreach (var entry in entries)
            {
                var name = Path.GetFileName(entry);
                if (name.StartsWith('.') || name == "target")
                {
                    continue;
                }
                if (Directory.Exists(entry))
                {
                    pending.Push(entry);
                    continue;
                }
                if (!Text.AsciiLower(name).EndsWith("." + DataFile.Extension, StringComparison.Ordinal))
                {
                    continue;
                }
                if (data.Count >= MaxDataFiles)
                {
                    problems.Add(string.Create(CultureInfo.InvariantCulture, $"Only the first {MaxDataFiles} data files are read."));
                    return (data, problems);
                }
                var relative = Path.GetRelativePath(root, entry).Replace('\\', '/');
                var read = DataFile.Load(entry);
                if (read.IsError)
                {
                    problems.Add($"The data file {relative} is not read: {read.Error.Message}.");
                    continue;
                }
                var shape = DataCommands.ShapeOf(session, read.Value);
                if (shape.IsError)
                {
                    problems.Add($"The data file {relative} is not read: {shape.Error}.");
                    continue;
                }
                data[relative] = read.Value.Resolved(shape.Value);
            }
        }
        return (data, problems);
    }

    /// <summary>Moves what scripts wrote and their errors into the console, labelled with the object's name.</summary>
    public static void Drain(Session session)
    {
        if (session.Play is not { } play)
        {
            return;
        }
        foreach (var line in play.Host.TakeLogs())
        {
            session.Log(line.Error ? Level.Error : Level.Info, $"[{session.Scene.NameOf(line.Owner) ?? "Object"} · {line.Script}] {line.Text}");
        }
        foreach (var fault in play.Host.TakeFaults())
        {
            session.Log(Level.Error, $"[{session.Scene.NameOf(fault.Owner) ?? "Object"} · {fault.Script}] script stopped: {fault.Error}");
        }
    }

    /// <summary>After scripts ran outside a step (a message, a press): what they asked for, what they logged, and whether the watchdog stopped play.</summary>
    public static void AfterScripts(Session session)
    {
        if (StoppedByWatchdog(session))
        {
            return;
        }
        ApplyRequests(session);
        Drain(session);
    }

    /// <summary>When the watchdog stopped the scripts' last run, play mode stops and says why. True when it did.</summary>
    public static bool StoppedByWatchdog(Session session)
    {
        if (session.Play is not { Host.Stalled: true } play)
        {
            return false;
        }
        Drain(session);
        var limit = session.ScriptTimeLimit.TotalSeconds.ToString("0.###", CultureInfo.InvariantCulture);
        session.Log(Level.Error, $"Play mode stopped: the scripts ran longer than {limit} s in one go ({play.Host.StalledIn ?? "a script"} was running). A script stuck in a loop keeps running in the background until the editor closes.");
        StopPlay(session);
        return true;
    }

    /// <summary>Stops play mode: the saved values are kept, the scene comes back as it was, the modules forget it. The time played.</summary>
    public static double StopPlay(Session session)
    {
        KeepSaved(session);
        var play = session.Play ?? throw Stop.Failed("play mode is not running");
        session.Play = null;
        play.Host.Dispose();
        session.Scene = SceneModel.FromDoc(play.Snapshot).OrFail();
        session.Animation.Clear();
        foreach (var module in session.Modules)
        {
            module.Stopped();
        }
        session.Log(Level.Info, string.Create(CultureInfo.InvariantCulture, $"Play mode stopped after {play.Time:0.00} s; the scene is back as it was."));
        return play.Time;
    }

    /// <summary>Advances play mode by <paramref name="dt"/> seconds, split into steps of at most <see cref="MaxStep"/>.</summary>
    public static void Step(Session session, float dt)
    {
        var remaining = Math.Max(dt, 0f);
        while (remaining > 0f)
        {
            var seconds = Math.Min(remaining, MaxStep);
            remaining -= seconds;
            if (session.Play is not { } play)
            {
                return;
            }
            foreach (var problem in SyncObjectInput(session, play.Host, play.InputAssets))
            {
                session.Log(Level.Error, problem);
            }
            play.Time += seconds;
            play.Host.Held = session.PlayInput;
            play.Host.Input.Step(session.PlayInput);
            // Scripts read the interfaces as they show at the step's start.
            InterfacePlay.RefreshValues(session);
            if (!play.Host.Update(session.Scene, seconds, play.Time))
            {
                StoppedByWatchdog(session);
                return;
            }
            ApplyRequests(session);
            // Keyframe Animations pose their parts after the scripts asked to play or stop them.
            TimelinePlay.Step(session, seconds);
            AnimationCommands.Step(session, seconds);
            if (session.Play is null)
            {
                return;
            }
            RunModules(session, (module, world) => module.Stepped(world, seconds));
            ReportOutcome(session);
            if (session.Play is not { } still)
            {
                return;
            }
            still.SinceReloadCheck += seconds;
            if (still.SinceReloadCheck >= ReloadCheckInterval)
            {
                still.SinceReloadCheck = 0;
                ReloadChanged(session);
            }
            Drain(session);
        }
    }

    /// <summary>Runs each opt-in module, in the order given; without world streaming a scene with Streamed Areas says once that they stay empty.</summary>
    public static void RunModules(Session session, Action<IPlayModule, IPlayWorld> run)
    {
        if (session.Play is not { } play)
        {
            return;
        }
        var world = new PlayWorld(session);
        foreach (var module in session.Modules)
        {
            run(module, world);
            if (session.Play is null)
            {
                return;
            }
        }
        if (!session.Modules.OfType<IStreamingModule>().Any() && ScriptHost.Order(session.Scene).Any(id => session.Scene.TryGet<StreamedArea>(id, out _)) && play.Told.Add("streaming"))
        {
            session.Log(Level.Info, "This build has no world streaming, so Streamed Areas stay empty.");
        }
        AfterScripts(session);
    }

    /// <summary>Recompiles the scripts when their files changed and runs them anew; a change that does not compile is reported once and the running scripts keep running.</summary>
    private static void ReloadChanged(Session session)
    {
        if (session.Play is not { } play || play.SeenScripts is not { } seen || !seen.Changed())
        {
            return;
        }
        var next = session.CompiledScripts();
        play.SeenScripts = next;
        if (!next.Compiled)
        {
            session.Log(Level.Error, $"The scripts changed but do not compile, so the running ones keep running: {next.Summary()}");
            return;
        }
        if (next.Load().TryGet(out var loaded, out var why) is false)
        {
            session.Log(Level.Error, $"The scripts changed but do not load: {why}");
            return;
        }
        if (!play.Host.Reload(loaded, session.Scene, play.Time))
        {
            StoppedByWatchdog(session);
            return;
        }
        session.Log(Level.Info, "Reloaded the project's scripts; they started again.");
        AfterScripts(session);
    }

    /// <summary>Announces the end of the game once: in the console and to every script (gameWon or gameLost).</summary>
    private static void ReportOutcome(Session session)
    {
        if (session.Play is not { } play || play.Host.State.Outcome is not { } won || play.ToldOutcome)
        {
            return;
        }
        play.ToldOutcome = true;
        play.Host.Send(session.Scene, null, won ? "gameWon" : "gameLost", 0f, play.Time);
        if (StoppedByWatchdog(session))
        {
            return;
        }
        session.Log(Level.Info, won ? "Game won." : "Game lost.");
        ApplyRequests(session);
        KeepSaved(session);
    }

    /// <summary>Writes the saved values to the project when scripts changed them. Outside a project they last for this play only, which the console says once.</summary>
    public static void KeepSaved(Session session)
    {
        if (session.Play is not { SavedOnDisk: { } onDisk } play)
        {
            return;
        }
        var saved = play.Host.State.Saved;
        if (onDisk.Count == saved.Count && onDisk.All(e => saved.TryGetValue(e.Key, out var v) && v.Equals(e.Value)))
        {
            return;
        }
        play.SavedOnDisk = new SortedDictionary<string, double>(saved, Utf8Order.Instance);
        if (session.ProjectDir is null)
        {
            session.Log(Level.Info, "Saved values are kept only in a project; open or make one to keep them between plays.");
            return;
        }
        var written = SavedValues.Write(session, saved);
        if (written.IsError)
        {
            session.Log(Level.Error, $"Saved values could not be written: {written.Error}");
        }
    }

    /// <summary>Does what scripts asked for since the last time, in order.</summary>
    public static void ApplyRequests(Session session)
    {
        if (session.Play is not { } play)
        {
            return;
        }
        foreach (var request in play.Host.TakeRequests())
        {
            if (session.Play is null)
            {
                return;
            }
            var who = session.Scene.NameOf(request.By) ?? "Object";
            switch (request)
            {
                case Request.Destroy d:
                    if (session.Scene.Contains(d.Target))
                    {
                        session.Scene.DeleteObject(d.Target);
                    }
                    break;
                case Request.Animation a:
                    AnimationCommands.ApplyScript(session, a.By, a.Target, a.Action);
                    break;
                case Request.InterfaceEdit i:
                    InterfacePlay.Apply(session, i.By, i.Target, i.Element, i.Change);
                    break;
                case Request.Keyframes k:
                    if (TimelinePlay.SetPlaying(session, k.Target, k.Play) is { } problem)
                    {
                        session.Log(Level.Error, $"[{who}] cannot {(k.Play ? "play" : "stop")} the Keyframe Animation of {session.Scene.NameOf(k.Target) ?? "Object"}: {problem}");
                    }
                    break;
                case Request.LoadScene l:
                    if (LoadScene(session, l.Path, l.Additive).TryGet(out _, out var notLoaded) is false)
                    {
                        session.Log(Level.Error, $"[{who}] could not load the scene {l.Path}: {notLoaded}");
                    }
                    break;
                case Request.UnloadScene u:
                    if (UnloadScene(session, u.Path).TryGet(out _, out var notUnloaded) is false)
                    {
                        session.Log(Level.Error, $"[{who}] could not unload the scene {u.Path}: {notUnloaded}");
                    }
                    break;
                case Request.Spawn s:
                    if (Spawn(session, s.Prefab, t => t with { Position = s.Position }).TryGet(out _, out var notSpawned) is false)
                    {
                        session.Log(Level.Error, $"[{who}] could not spawn {s.Prefab}: {notSpawned}");
                    }
                    break;
                case Request.SetField f:
                    SetField(session, who, f);
                    break;
            }
        }
    }

    /// <summary>Sets a component's field as a script asked: the field keeps its shape (a colour keeps its alpha), and the component checks the value.</summary>
    private static void SetField(Session session, string who, Request.SetField f)
    {
        string? error = null;
        var current = session.Scene.Component(f.Target, f.Component);
        if (current.IsError || current.Value is not Core.Json.JsonObject fields || fields.Get(f.Field) is not { } value || ScriptHost.WithField(value, f.Value) is not { } changed)
        {
            error = $"it has no {f.Component} field {f.Field}";
        }
        else
        {
            fields.Set(f.Field, changed);
            var set = session.Scene.SetComponent(f.Target, f.Component, fields);
            error = set.IsError ? set.Error.Message : null;
        }
        if (error is not null)
        {
            session.Log(Level.Error, $"[{who}] could not set {f.Field} of {session.Scene.NameOf(f.Target) ?? "Object"}'s {f.Component}: {error}");
        }
    }

    /// <summary>Places a linked copy of a prefab at the top of the scene, <paramref name="place"/> setting its transform, and starts its scripts.</summary>
    public static Result<StableId, string> Spawn(Session session, string prefab, Func<Transform, Transform> place)
    {
        if (session.Scene.ObjectCount >= MaxObjects)
        {
            return Result.Fail(string.Create(CultureInfo.InvariantCulture, $"the scene already has {MaxObjects} objects"));
        }
        try
        {
            var objects = PrefabCommands.ReadPrefab(session, prefab);
            var roots = session.Scene.Import(objects, null, null).OrFail();
            if (roots.Count == 0)
            {
                return Result.Fail("the prefab is empty");
            }
            var id = roots[0];
            var transform = session.Scene.TryGet<Transform>(id, out var t) ? t : Transform.Initial;
            session.Scene.Set(id, place(transform)).OrFail();
            session.Scene.Set(id, new Prefab(prefab, [])).OrFail();
            StartUnder(session, [id]);
            return id;
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error.Message);
        }
    }

    /// <summary>Starts the scripts and keyframe animations of <paramref name="roots"/> and everything under them.</summary>
    public static void StartUnder(Session session, IReadOnlyList<StableId> roots)
    {
        if (session.Play is not { } play)
        {
            return;
        }
        ReadableModules(session, session.Scene);
        if (!play.Host.LoadUnder(session.Scene, roots, play.Time))
        {
            StoppedByWatchdog(session);
            return;
        }
        foreach (var problem in TimelinePlay.StartUnder(session, roots))
        {
            session.Log(Level.Error, problem);
        }
    }

    /// <summary>Adds a scene read while playing to the playing scene, under <paramref name="parent"/> (or at the top): its objects join, their scripts start. The new top objects.</summary>
    public static Result<IReadOnlyList<StableId>, string> AddScene(Session session, SceneModel loaded, StableId? parent)
    {
        try
        {
            ReadableModules(session, loaded);
            if (session.Scene.ObjectCount + loaded.ObjectCount > MaxObjects)
            {
                return Result.Fail(string.Create(CultureInfo.InvariantCulture, $"the scene would have more than {MaxObjects} objects"));
            }
            var objects = loaded.Export(loaded.Children(null)).OrFail();
            var roots = session.Scene.Import(objects, parent, null).OrFail();
            StartUnder(session, roots);
            Drain(session);
            return roots.ToList();
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error.Message);
        }
    }

    /// <summary>Reads a scene file of the project for play: a <c>.bscene</c>, relative to the project folder.</summary>
    private static Result<SceneModel, string> ReadScene(Session session, string relative)
    {
        if (!relative.EndsWith(".bscene", StringComparison.Ordinal))
        {
            return Result.Fail("give a .bscene file");
        }
        try
        {
            var path = session.Allowed(Path.Combine(session.ScriptDir, relative), Access.Read);
            var scene = SceneModel.Load(path);
            if (scene.IsError)
            {
                return Result.Fail(scene.Error.Message);
            }
            ReadableModules(session, scene.Value);
            return scene.Value;
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error.Message);
        }
    }

    /// <summary>
    /// Loads a scene while playing. Added, its objects join the playing scene, their scripts start, and
    /// unloading removes them again. Otherwise it takes the playing scene's place: every script starts
    /// again with the game's state kept. Stop still brings back the scene as it was before play.
    /// </summary>
    public static Result<IReadOnlyList<StableId>, string> LoadScene(Session session, string relative, bool additive)
    {
        if (session.Play is null)
        {
            return Result.Fail("play mode is not running");
        }
        var loaded = ReadScene(session, relative);
        if (loaded.IsError)
        {
            return loaded.AsFailure();
        }
        if (additive)
        {
            var roots = AddScene(session, loaded.Value, null);
            if (roots.IsOk && session.Play is { } added)
            {
                added.Loaded.Add((relative, roots.Value));
                session.Log(Level.Info, $"Added the scene {relative}.");
            }
            return roots;
        }
        // What the scripts said so far reaches the console before their host goes.
        Drain(session);
        var play = session.Play!;
        var state = play.Host.State.Copy();
        session.Scene = loaded.Value;
        play.Host.Dispose();
        var (host, seen, problems) = FreshHost(session, play);
        host.State = state;
        play.Host = host;
        play.SeenScripts = seen;
        play.Loaded.Clear();
        play.Interfaces.Clear();
        play.InterfaceRoots.Clear();
        play.Timelines.Clear();
        session.Animation.Clear();
        foreach (var problem in problems)
        {
            session.Log(Level.Error, problem);
        }
        if (!host.LoadScene(session.Scene, play.Time))
        {
            StoppedByWatchdog(session);
            return Result.Fail("the scripts ran too long");
        }
        session.Log(Level.Info, $"Loaded the scene {relative}.");
        foreach (var problem in TimelinePlay.StartUnder(session, null))
        {
            session.Log(Level.Error, problem);
        }
        AnimationCommands.StartPlay(session);
        RunModules(session, (module, world) => module.Started(world));
        Drain(session);
        return session.Scene.Children(null).ToList();
    }

    /// <summary>Removes every scene added from <paramref name="relative"/> while playing; how many objects went.</summary>
    public static Result<int, string> UnloadScene(Session session, string relative)
    {
        if (session.Play is not { } play)
        {
            return Result.Fail("play mode is not running");
        }
        var gone = play.Loaded.Where(l => l.Path == relative).ToList();
        if (gone.Count == 0)
        {
            return Result.Fail($"{relative} was not added while playing");
        }
        play.Loaded.RemoveAll(l => l.Path == relative);
        var removed = 0;
        foreach (var root in gone.SelectMany(g => g.Roots))
        {
            if (session.Scene.Contains(root) && session.Scene.DeleteObject(root).TryGet(out var count, out _))
            {
                removed += count;
            }
        }
        session.Log(Level.Info, $"Removed the scene {relative}.");
        return removed;
    }

    /// <summary>What play mode lets the opt-in modules do.</summary>
    private sealed class PlayWorld(Session session) : IPlayWorld
    {
        public SceneModel Scene => session.Scene;

        public double Time => session.PlayTime;

        public void Log(bool problem, string text) => session.Log(problem ? Level.Error : Level.Info, text);

        public Result<StableId, string> Place(string prefab, Func<Transform, Transform> place) => Spawn(session, prefab, place);

        public Result<IReadOnlyList<StableId>, string> AddScene(SceneModel scene, StableId? parent) => PlayLoop.AddScene(session, scene, parent);

        public Result<string, string> Readable(string relative)
        {
            try
            {
                return session.Allowed(Path.Combine(session.ScriptDir, relative), Access.Read);
            }
            catch (CommandException e)
            {
                return Result.Fail(e.Error.Message);
            }
        }

        public bool EndGame(bool won) => session.Play is { } play && play.Host.State.End(won);
    }
}


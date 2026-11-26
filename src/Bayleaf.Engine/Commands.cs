using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Engine;

/// <summary>Which document a command changes, and so what its undo step restores.</summary>
public enum Mutates
{
    /// <summary>Changes nothing that undo covers (reads, files, play mode).</summary>
    No,

    /// <summary>Changes the scene; one undo step, and the scene file is marked modified.</summary>
    Scene,

    /// <summary>Changes the open graph; one undo step, and the graph file is marked modified.</summary>
    Graph,

    /// <summary>Changes the open timeline; one undo step, and the timeline file is marked modified.</summary>
    Timeline,
}

/// <summary>One value a fields prompt asks for.</summary>
public sealed record PromptField(string Name, string Label, string Default, bool Number);

public enum PromptKind
{
    /// <summary>Params can be null.</summary>
    None,

    /// <summary><c>{"path"}</c> of an existing scene file.</summary>
    OpenScene,

    /// <summary><c>{"path"}</c> to write a scene file to.</summary>
    SaveScene,

    /// <summary><c>{"id"}</c> of the selected object.</summary>
    SelectedObject,

    /// <summary><c>{"path"}</c> of a new, empty project folder.</summary>
    NewProject,

    /// <summary><c>{"path"}</c> of a <c>.bproject</c> file.</summary>
    OpenProject,

    /// <summary><c>{"folder"}</c>: a folder to write into.</summary>
    Folder,

    /// <summary>The front end shows a window of its own for the command's choices.</summary>
    Dialog,

    /// <summary>A form of named values, sent as an object of them.</summary>
    Fields,
}

/// <summary>What a front end has to ask the user for before running a command.</summary>
public sealed record Prompt(PromptKind Kind, IReadOnlyList<PromptField> Fields)
{
    public static readonly Prompt None = new(PromptKind.None, []);
    public static readonly Prompt OpenScene = new(PromptKind.OpenScene, []);
    public static readonly Prompt SaveScene = new(PromptKind.SaveScene, []);
    public static readonly Prompt SelectedObject = new(PromptKind.SelectedObject, []);
    public static readonly Prompt NewProject = new(PromptKind.NewProject, []);
    public static readonly Prompt OpenProject = new(PromptKind.OpenProject, []);
    public static readonly Prompt Folder = new(PromptKind.Folder, []);
    public static readonly Prompt Dialog = new(PromptKind.Dialog, []);

    public static Prompt Form(params PromptField[] fields) => new(PromptKind.Fields, fields);

    public JsonValue ToJson() => Kind switch
    {
        PromptKind.Fields => new JsonObject
        {
            {
                "fields",
                new JsonArray(Fields.Select(f => (JsonValue)new JsonObject
                {
                    { "name", new JsonString(f.Name) },
                    { "label", new JsonString(f.Label) },
                    { "default", new JsonString(f.Default) },
                    { "number", JsonValue.From(f.Number) },
                }))
            },
        },
        _ => new JsonString(char.ToLowerInvariant(Kind.ToString()[0]) + Kind.ToString()[1..]),
    };
}

/// <summary>
/// One registered command. Its run throws <see cref="CommandException"/> (or, for params of the wrong
/// shape, <see cref="JsonShapeException"/>) to fail; the registry turns either into an error.
/// </summary>
public sealed record CommandSpec
{
    /// <summary>Dotted id with camelCase segments, such as <c>scene.object.create</c>.</summary>
    public required string Id { get; init; }

    /// <summary>English label. It is the translation key.</summary>
    public required string Label { get; init; }

    /// <summary>Menu path, empty when the command has no menu entry.</summary>
    public string Menu { get; init; } = "";

    /// <summary>Default shortcut such as <c>Ctrl+Shift+S</c>, empty for none.</summary>
    public string Shortcut { get; init; } = "";

    /// <summary>Human-readable description of the params object.</summary>
    public required string Params { get; init; }

    public Prompt Prompt { get; init; } = Prompt.None;

    /// <summary>What the command changes; changes record one undo step.</summary>
    public Mutates Mutates { get; init; } = Mutates.No;

    /// <summary>Null when the command can run now, else why not.</summary>
    public Func<Session, string?> Enabled { get; init; } = Session.Always;

    public required Func<Session, JsonValue, JsonValue> Run { get; init; }
}

/// <summary>Registry of every command the engine knows.</summary>
public sealed class CommandRegistry
{
    private readonly List<CommandSpec> commands;

    private CommandRegistry(List<CommandSpec> commands) => this.commands = commands;

    /// <summary>The engine's version, the same as the Rust engine's while both are released together.</summary>
    public const string EngineVersion = BuiltinCommands.Version;

    /// <summary>Why a command the C# engine does not run yet is disabled (the same reason for each).</summary>
    public static string PendingReason(string id) => PendingCommands.Reason(id)!;

    /// <summary>Registry with the built-in commands.</summary>
    public static CommandRegistry WithBuiltins()
    {
        var all = new List<CommandSpec>();
        all.AddRange(BuiltinCommands.All());
        all.AddRange(SceneCommands.All());
        all.AddRange(LightingCommands.All());
        all.AddRange(DataCommands.All());
        all.AddRange(ProjectCommands.All());
        all.AddRange(PrefabCommands.All());
        all.AddRange(InputCommands.All());
        all.AddRange(TimelineCommands.All());
        all.AddRange(ConsoleCommands.All());
        all.AddRange(StreamCommands.All());
        all.AddRange(AssetCommands.All());
        all.AddRange(ImportCommands.All());
        all.AddRange(InterfaceCommands.All());
        all.AddRange(PendingCommands.All(all.Select(c => c.Id).ToHashSet(StringComparer.Ordinal)));
        return new CommandRegistry(all);
    }

    public IReadOnlyList<CommandSpec> Commands => commands;

    public CommandSpec? Get(string id) => commands.Find(c => c.Id == id);

    /// <summary>Every command as <c>commands</c> lists them: id, label, menu, shortcut, params, prompt, enabled and why not.</summary>
    public JsonArray List(Session session) => new(commands.Select(c =>
    {
        var reason = c.Enabled(session);
        var row = new JsonObject
        {
            { "id", new JsonString(c.Id) },
            { "label", new JsonString(c.Label) },
            { "menu", new JsonString(c.Menu) },
            { "shortcut", new JsonString(c.Shortcut) },
            { "params", new JsonString(c.Params) },
            { "prompt", c.Prompt.ToJson() },
            { "enabled", JsonValue.From(reason is null) },
        };
        if (reason is not null)
        {
            row.Add("disabledReason", new JsonString(reason));
        }
        return (JsonValue)row;
    }));

    /// <summary>Runs <paramref name="id"/> with <paramref name="parameters"/>, which must be a JSON object or null.</summary>
    public Result<JsonValue, CommandError> Execute(Session session, string id, JsonValue parameters) => ExecuteMerging(session, id, parameters, null);

    /// <summary>
    /// Like <see cref="Execute"/>, but consecutive changing runs with the same <paramref name="mergeKey"/>
    /// share one undo step. The editor uses this while a value is being dragged or typed.
    /// </summary>
    public Result<JsonValue, CommandError> ExecuteMerging(Session session, string id, JsonValue parameters, string? mergeKey)
    {
        if (parameters is not (JsonObject or JsonNull))
        {
            return Result.Fail(CommandError.ParamsNotObject);
        }
        // While recording, edits of the posed parts become keys.
        if (TimelineCommands.Redirect(session, id, parameters))
        {
            id = "timeline.key.record";
        }
        if (Get(id) is not { } spec)
        {
            return Result.Fail(CommandError.Unknown(id));
        }
        // The timeline's pose is only shown: it goes before anything else sees or saves the scene.
        TimelineCommands.BeforeCommand(session, id);
        if (spec.Enabled(session) is { } reason)
        {
            return Result.Fail(CommandError.Disabled(id, reason));
        }
        if (spec.Mutates == Mutates.No)
        {
            return Run(spec, session, parameters);
        }
        Session.Snapshot before;
        switch (spec.Mutates)
        {
            case Mutates.Scene:
                before = session.SceneSnapshot();
                break;
            case Mutates.Timeline:
                before = session.TimelineSnapshot() ?? new Session.Snapshot(Session.SnapshotKind.Timeline, "");
                if (session.Timeline is null)
                {
                    return Result.Fail(CommandError.Failed(TimelineCommands.NoTimeline));
                }
                break;
            default:
                return Result.Fail(CommandError.Failed($"{id} changes a document the C# engine does not hold yet"));
        }
        var result = Run(spec, session, parameters);
        if (result.IsError)
        {
            // A failed command leaves its document as it was.
            session.PutBack(before);
            return result;
        }
        if (spec.Mutates == Mutates.Timeline)
        {
            // A timeline edit that changed nothing is no undo step.
            if (session.TimelineSnapshot() is { } now && now.Text == before.Text)
            {
                TimelineCommands.AfterChange(session);
                return result;
            }
        }
        else if (session.IsPlaying)
        {
            // Play-mode changes to the scene are thrown away on stop: they are no undo steps.
            return result;
        }
        session.Record(id, before, mergeKey);
        if (spec.Mutates == Mutates.Timeline)
        {
            session.Timeline!.Dirty = true;
            TimelineCommands.AfterChange(session);
        }
        else
        {
            session.Dirty = true;
        }
        return result;
    }

    private static Result<JsonValue, CommandError> Run(CommandSpec spec, Session session, JsonValue parameters)
    {
        try
        {
            return spec.Run(session, parameters).Canonical();
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error);
        }
        catch (JsonShapeException e)
        {
            return Result.Fail(CommandError.BadParams(e.Message));
        }
    }
}

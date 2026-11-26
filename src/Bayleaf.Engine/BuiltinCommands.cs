using Bayleaf.Core.Json;

namespace Bayleaf.Engine;

/// <summary>Built-in commands that do not belong to a feature area: about, undo and redo.</summary>
internal static class BuiltinCommands
{
    /// <summary>The engine's version, the same as the Rust engine's while both are released together.</summary>
    public const string Version = "0.3.0";

    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "app.version",
            Label = "About Bayleaf Engine",
            Menu = "Help/About Bayleaf Engine",
            Params = "none",
            Run = static (_, _) => J.Obj(("name", J.S("Bayleaf Engine")), ("version", J.S(Version))),
        },
        new()
        {
            Id = "edit.undo",
            Label = "Undo",
            Menu = "Edit/Undo",
            Shortcut = "Ctrl+Z",
            Params = "none",
            Enabled = static s => Session.NotPlaying(s) ?? (s.CanUndo ? null : "nothing to undo"),
            Run = static (s, _) => Step(s, s.Undo, s.Redo, "undone", "nothing to undo"),
        },
        new()
        {
            Id = "edit.redo",
            Label = "Redo",
            Menu = "Edit/Redo",
            Shortcut = "Ctrl+Y",
            Params = "none",
            Enabled = static s => Session.NotPlaying(s) ?? (s.CanRedo ? null : "nothing to redo"),
            Run = static (s, _) => Step(s, s.Redo, s.Undo, "redone", "nothing to redo"),
        },
    ];

    /// <summary>Takes the last step of <paramref name="from"/>, puts its scene back and keeps the scene it replaced on <paramref name="to"/>.</summary>
    private static JsonObject Step(Session session, List<Session.HistoryStep> from, List<Session.HistoryStep> to, string key, string nothing)
    {
        if (from.Count == 0)
        {
            throw Stop.Failed(nothing);
        }
        var step = from[^1];
        from.RemoveAt(from.Count - 1);
        var current = session.Restore(step.Before);
        if (step.Before.Kind == Session.SnapshotKind.Scene)
        {
            session.Dirty = true;
        }
        to.Add(new Session.HistoryStep(step.Command, current, null));
        return J.Obj((key, new JsonString(step.Command)));
    }
}

using Bayleaf.Core.Json;

namespace Bayleaf.Engine;

/// <summary>
/// The console (<c>console.read</c>, <c>console.clear</c>) and saved values: numbers a game keeps from
/// one play to the next, such as a best score, in the project's <c>saves/progress.json</c>. Play mode
/// reads and writes them; clearing them needs no play mode.
/// </summary>
internal static class ConsoleCommands
{
    /// <summary>Where saved values live, relative to the project folder.</summary>
    public const string SavedFile = "saves/progress.json";

    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "console.read", Label = "Read Console",
            Params = "{last?: n (default 100), errors?: true for errors only}; returns {lines: [{level, text}], total}: the newest lines, oldest first",
            Run = Read,
        },
        new()
        {
            Id = "console.clear", Label = "Clear Console", Params = "none",
            Run = static (s, p) =>
            {
                Params.None(p);
                s.ClearConsole();
                return new JsonObject();
            },
        },
        new()
        {
            Id = "game.saved.clear", Label = "Clear Saved Values",
            Params = "none; forgets the values the game saved in the project, and in play mode the ones it holds",
            Run = SavedClear,
        },
    ];

    private static JsonObject Read(Session session, JsonValue p)
    {
        var f = Params.Of(p, "last", "errors");
        var last = f.Optional("last", Params.Index, 100);
        var errors = f.Optional("errors", JsonRead.Bool, false);
        var lines = session.Console.Where(l => !errors || l.Level == Level.Error).ToList();
        var shown = lines.Skip(Math.Max(0, lines.Count - last)).Select(l =>
            (JsonValue)J.Obj(("level", J.S(l.Level == Level.Error ? "error" : "info")), ("text", J.S(l.Text))));
        return J.Obj(("lines", new JsonArray(shown)), ("total", J.N(lines.Count)));
    }

    private static JsonObject SavedClear(Session session, JsonValue p)
    {
        Params.None(p);
        var removed = false;
        if (session.ProjectDir is { } dir)
        {
            var path = session.Allowed(Path.Combine(dir, SavedFile), Access.Write);
            if (File.Exists(path))
            {
                Core.Files.RemoveFile(path).MapError(e => $"{path}: {e}").OrFail();
                removed = true;
            }
        }
        if (session.Play is { } play)
        {
            play.Host.State.Saved.Clear();
            play.SavedOnDisk = new SortedDictionary<string, double>(Core.Utf8Order.Instance);
        }
        session.Log(Level.Info, "Saved values cleared.");
        return J.Obj(("removed", J.B(removed)));
    }
}

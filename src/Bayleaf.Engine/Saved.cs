using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Script;

namespace Bayleaf.Engine;

/// <summary>
/// Saved values: numbers a game keeps from one play to the next, such as a best score. They live in the
/// project, in <c>saves/progress.json</c>; outside a project they last for one play.
/// </summary>
internal static class SavedValues
{
    private const string Format = "bayleaf.saved";
    private const uint Version = 1;

    /// <summary>The largest saved-values file read.</summary>
    private const long MaxFileBytes = 1 << 20;

    /// <summary>The saved-values file of the session's project, if it has one.</summary>
    public static string? PathOf(Session session) => session.ProjectDir is { } dir ? System.IO.Path.Combine(dir, ConsoleCommands.SavedFile) : null;

    /// <summary>The values saved in the project: empty when there is no project or nothing saved yet.</summary>
    public static Result<SortedDictionary<string, double>, string> Read(Session session)
    {
        var values = new SortedDictionary<string, double>(Utf8Order.Instance);
        if (PathOf(session) is not { } file)
        {
            return values;
        }
        string path;
        try
        {
            path = session.Allowed(file, Access.Read);
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error.Message);
        }
        if (!File.Exists(path))
        {
            return values;
        }
        var size = Files.Length(path);
        if (size.IsError)
        {
            return Result.Fail($"{path}: {size.Error}");
        }
        if (size.Value > MaxFileBytes)
        {
            return Result.Fail($"{path} is larger than {MaxFileBytes / 1024} KiB");
        }
        var text = Files.ReadText(path);
        if (text.IsError)
        {
            return Result.Fail($"{path}: {text.Error}");
        }
        var parsed = JsonReader.Parse(text.Value);
        var read = parsed.IsOk ? JsonRead.Try(() =>
        {
            var f = JsonFields.Of(parsed.Value, "struct SavedFile", true, "format", "version", "values");
            return (Format: f.Required("format", JsonRead.Str), Version: f.Required("version", JsonRead.U32), Values: f.Required("values", v => JsonRead.Map(v, JsonRead.F64)));
        }) : Result.Fail(parsed.Error);
        if (read.IsError)
        {
            return Result.Fail($"{path}: {read.Error}");
        }
        if (read.Value.Format != Format || read.Value.Version != Version)
        {
            return Result.Fail($"{path} is not a version {Version} saved-values file");
        }
        if (read.Value.Values.Count > GameState.MaxValues)
        {
            return Result.Fail($"{path} holds more than {GameState.MaxValues} values");
        }
        foreach (var (name, value) in read.Value.Values)
        {
            if (!GameState.Valid(name, value))
            {
                return Result.Fail($"{path}: {Text.Debug(name)} needs a name of 1 to 64 bytes and a finite number");
            }
            values[name] = value;
        }
        return values;
    }

    /// <summary>Writes the values to the project; an error when there is no project or it may not be written.</summary>
    public static Result<string, string> Write(Session session, IReadOnlyDictionary<string, double> values)
    {
        if (PathOf(session) is not { } file)
        {
            return Result.Fail("saved values are kept only in a project");
        }
        string path;
        try
        {
            path = session.Allowed(file, Access.Write);
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error.Message);
        }
        if (System.IO.Path.GetDirectoryName(path) is { Length: > 0 } folder)
        {
            var made = Files.CreateDirectory(folder);
            if (made.IsError)
            {
                return Result.Fail($"{folder}: {made.Error}");
            }
        }
        var body = new JsonObject();
        foreach (var (name, value) in values.OrderBy(v => v.Key, Utf8Order.Instance))
        {
            body.Add(name, JsonNumber.Of(value));
        }
        var text = new JsonObject { { "format", new JsonString(Format) }, { "version", JsonNumber.Of((ulong)Version) }, { "values", body } }.ToPretty() + "\n";
        var written = Files.Write(path, System.Text.Encoding.UTF8.GetBytes(text));
        return written.IsOk ? path : Result.Fail($"{path}: {written.Error}");
    }
}

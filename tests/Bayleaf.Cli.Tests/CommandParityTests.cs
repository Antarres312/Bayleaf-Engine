using System.Text;

using Bayleaf.Core.Json;
using Bayleaf.Engine;

namespace Bayleaf.Cli.Tests;

/// <summary>
/// Stage 2 of docs/csharp-redesign.md: the commands give back what the Rust engine's give back. Each
/// batch in <c>tests/commands</c> runs as <c>bayleaf-cli batch</c> runs it, and what it prints (made the
/// same on every machine, as <c>rust/crates/engine/src/parity_tests.rs</c> makes it) must be what the
/// Rust engine printed, in <c>tests/commands/expected</c>.
/// </summary>
public class CommandParityTests
{
    private static string Root()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    private static string Plain(string path) => Path.GetFullPath(path).TrimEnd(Path.DirectorySeparatorChar).Replace('\\', '/');

    private static JsonValue Place(JsonValue value, string root, string output) => value switch
    {
        JsonString s => new JsonString(s.Value.Replace("$root", root, StringComparison.Ordinal).Replace("$out", output, StringComparison.Ordinal)),
        JsonArray a => new JsonArray(a.Select(v => Place(v, root, output))),
        JsonObject o => new JsonObject(o.Entries.Select(e => new KeyValuePair<string, JsonValue>(e.Key, Place(e.Value, root, output)))),
        _ => value,
    };

    private static string NormalText(string text, IReadOnlyList<(string Folder, string Name)> folders, List<string> ids)
    {
        var output = text;
        foreach (var (folder, name) in folders)
        {
            output = output.Replace(folder, name, StringComparison.Ordinal).Replace(folder.Replace('/', '\\'), name, StringComparison.Ordinal);
        }
        if (output.StartsWith("$root", StringComparison.Ordinal) || output.StartsWith("$out", StringComparison.Ordinal))
        {
            output = output.Replace('\\', '/');
        }
        var result = new StringBuilder();
        var i = 0;
        static bool LowerHex(char c) => char.IsAsciiDigit(c) || c is >= 'a' and <= 'f';
        while (i < output.Length)
        {
            var run = 0;
            while (i + run < output.Length && LowerHex(output[i + run]))
            {
                run++;
            }
            var boundary = i == 0 || !char.IsAsciiLetterOrDigit(output[i - 1]);
            var after = i + 32 >= output.Length || !char.IsAsciiLetterOrDigit(output[i + 32]);
            if (run >= 32 && boundary && after)
            {
                var id = output.Substring(i, 32);
                var n = ids.IndexOf(id);
                if (n < 0)
                {
                    ids.Add(id);
                    n = ids.Count - 1;
                }
                result.Append("<id ").Append(n).Append('>');
                i += 32;
            }
            else
            {
                result.Append(output[i]);
                i++;
            }
        }
        return result.ToString();
    }

    private static JsonValue Normal(JsonValue value, IReadOnlyList<(string, string)> folders, List<string> ids) => value switch
    {
        JsonString s => new JsonString(NormalText(s.Value, folders, ids)),
        JsonArray a => new JsonArray(a.Select(v => Normal(v, folders, ids)).ToList()),
        JsonObject o => NormalObject(o, folders, ids),
        _ => value,
    };

    private static JsonObject NormalObject(JsonObject o, IReadOnlyList<(string, string)> folders, List<string> ids)
    {
        var entries = new List<KeyValuePair<string, JsonValue>>();
        foreach (var (key, item) in o.Entries)
        {
            var k = NormalText(key, folders, ids);
            entries.Add(new(k, Normal(item, folders, ids)));
        }
        return (JsonObject)new JsonObject(entries).Canonical();
    }

    /// <summary>A params error from the JSON reader, down to the words both engines share.</summary>
    private static string NormalError(string error)
    {
        const string prefix = "invalid params: ";
        if (!error.StartsWith(prefix, StringComparison.Ordinal))
        {
            return error;
        }
        var rest = error[prefix.Length..];
        string[] readers = ["invalid type", "invalid value", "invalid length", "missing field", "unknown field", "unknown variant", "duplicate field"];
        return readers.Any(w => rest.StartsWith(w, StringComparison.Ordinal))
            ? prefix + string.Join(' ', rest.Split(' ', StringSplitOptions.RemoveEmptyEntries).Take(3))
            : error;
    }

    private static string RunBatch(string path)
    {
        var root = Plain(Root());
        var outDir = Path.Join(Path.GetTempPath(), $"bayleaf-parity-cs-{Path.GetFileNameWithoutExtension(path)}-{Environment.ProcessId}");
        if (Directory.Exists(outDir))
        {
            Directory.Delete(outDir, true);
        }
        Directory.CreateDirectory(outDir);
        var output = Plain(outDir);
        try
        {
            var steps = Cli.Steps(path).Value.Select(s => s with { Params = Place(s.Params, root, output) }).ToList();
            var registry = CommandRegistry.WithBuiltins();
            var session = new Session();
            var (ran, failure) = Cli.RunSteps(registry, session, steps);
            var folders = new List<(string, string)> { (output, "$out"), (root, "$root") };
            var ids = new List<string>();
            var results = Normal(new JsonArray(ran), folders, ids);
            var report = new JsonObject();
            if (failure is var (index, error))
            {
                report.Add("error", new JsonString(NormalText(NormalError(error), folders, ids)));
                report.Add("failedStep", JsonNumber.Of((long)index));
            }
            report.Add("results", results);
            return report.ToPretty() + "\n";
        }
        finally
        {
            Directory.Delete(outDir, true);
        }
    }

    public static TheoryData<string> Batches()
    {
        var data = new TheoryData<string>();
        foreach (var file in Directory.EnumerateFiles(Path.Join(Root(), "tests", "commands"), "*.json").Order(StringComparer.Ordinal))
        {
            data.Add(Path.GetFileName(file));
        }
        return data;
    }

    [Theory]
    [MemberData(nameof(Batches))]
    public void EachBatchGivesBackWhatTheRustEngineGaveBack(string name)
    {
        var root = Root();
        var expected = File.ReadAllText(Path.Join(root, "tests", "commands", "expected", name)).ReplaceLineEndings("\n");
        Assert.Equal(expected, RunBatch(Path.Join(root, "tests", "commands", name)));
    }

    [Fact]
    public void EveryRustCommandIsListedAndThePortedOnesListTheSame()
    {
        var rust = (JsonArray)JsonReader.Parse(File.ReadAllText(Path.Join(Root(), "tests", "commands", "expected", "commands.json"))).Value;
        var registry = CommandRegistry.WithBuiltins();
        var mine = registry.List(new Session());
        var byId = mine.ToDictionary(r => r.Get("id")!.AsString()!, r => r, StringComparer.Ordinal);
        Assert.Equal(rust.Select(r => r.Get("id")!.AsString()!).Order(StringComparer.Ordinal), byId.Keys.Order(StringComparer.Ordinal));
        foreach (var row in rust)
        {
            var id = row.Get("id")!.AsString()!;
            var ours = byId[id];
            if (ours.Get("enabled") is JsonBool { Value: false } && ours.Get("disabledReason")?.AsString() == CommandRegistry.PendingReason(id))
            {
                continue;
            }
            Assert.Equal(row.ToCompact(), ours.ToCompact());
        }
    }
}

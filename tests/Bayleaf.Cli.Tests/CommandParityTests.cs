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

    private static JsonObject RunBatch(string path)
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
            return report;
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

    /// <summary>
    /// The differences from the Rust engine this engine keeps on purpose, each with why
    /// (<c>tests/commands/differences/known.json</c>): a field of some steps' results that must be
    /// what <c>csharp</c> says here and what the Rust engine gave there.
    /// </summary>
    private static List<(string Batch, List<int> Steps, string Field, JsonValue CSharp)> KnownDifferences()
    {
        var path = Path.Join(Root(), "tests", "commands", "differences", "known.json");
        if (!File.Exists(path))
        {
            return [];
        }
        var listed = (JsonArray)JsonReader.Parse(File.ReadAllText(path)).Value;
        return [.. listed.Select(d => (
            d.Get("batch")!.AsString()!,
            ((JsonArray)d.Get("steps")!).Select(s => (int)((JsonNumber)s).AsUInt64()!.Value).ToList(),
            d.Get("field")!.AsString()!,
            d.Get("csharp")!))];
    }

    /// <summary>
    /// The report as printed, with each known difference checked and then put back as the Rust engine
    /// gave it. The report is changed where it is, not printed and read back, since reading decimals
    /// back (as the Rust engine's JSON library reads them) can move a number's last digit.
    /// </summary>
    private static string WithKnownDifferences(string name, JsonObject ours, string expected)
    {
        var differences = KnownDifferences().Where(d => d.Batch == name).ToList();
        if (differences.Count == 0)
        {
            return ours.ToPretty() + "\n";
        }
        var rust = (JsonObject)JsonReader.Parse(expected).Value;
        foreach (var (_, steps, field, csharp) in differences)
        {
            foreach (var step in steps)
            {
                var mine = (JsonObject)((JsonArray)ours.Get("results")!)[step];
                Assert.Equal(csharp.Canonical().ToCompact(), mine.Get(field)?.Canonical().ToCompact());
                mine.Set(field, ((JsonArray)rust.Get("results")!)[step].Get(field)!);
            }
        }
        return ours.ToPretty() + "\n";
    }

    [Theory]
    [MemberData(nameof(Batches))]
    public void EachBatchGivesBackWhatTheRustEngineGaveBack(string name)
    {
        var root = Root();
        var expected = File.ReadAllText(Path.Join(root, "tests", "commands", "expected", name)).ReplaceLineEndings("\n");
        Assert.Equal(expected, WithKnownDifferences(name, RunBatch(Path.Join(root, "tests", "commands", name)), expected));
    }

    /// <summary>
    /// How this engine's command list differs from the Rust engine's on purpose, each with why
    /// (<c>tests/commands/differences/listing.json</c>): a field of a command's row that must be what
    /// <c>csharp</c> says, or (field <c>only</c>) a command only this engine has.
    /// </summary>
    private static List<(string Id, string Field, JsonValue CSharp)> ListingDifferences()
    {
        var path = Path.Join(Root(), "tests", "commands", "differences", "listing.json");
        var listed = (JsonArray)JsonReader.Parse(File.ReadAllText(path)).Value;
        return [.. listed.Select(d => (d.Get("id")!.AsString()!, d.Get("field")!.AsString()!, d.Get("csharp")!))];
    }

    [Fact]
    public void EveryRustCommandIsListedAndThePortedOnesListTheSame()
    {
        var rust = (JsonArray)JsonReader.Parse(File.ReadAllText(Path.Join(Root(), "tests", "commands", "expected", "commands.json"))).Value;
        var registry = CommandRegistry.WithBuiltins();
        var mine = registry.List(new Session());
        var byId = mine.ToDictionary(r => r.Get("id")!.AsString()!, r => r, StringComparer.Ordinal);
        var differences = ListingDifferences();
        var only = differences.Where(d => d.Field == "only").Select(d => d.Id).ToHashSet(StringComparer.Ordinal);
        Assert.Equal(rust.Select(r => r.Get("id")!.AsString()!).Concat(only).Order(StringComparer.Ordinal), byId.Keys.Order(StringComparer.Ordinal));
        foreach (var row in rust)
        {
            var id = row.Get("id")!.AsString()!;
            var ours = (JsonObject)JsonReader.Parse(byId[id].ToCompact()).Value;
            if (ours.Get("enabled") is JsonBool { Value: false } && ours.Get("disabledReason")?.AsString() == CommandRegistry.PendingReason(id))
            {
                continue;
            }
            foreach (var (_, field, csharp) in differences.Where(d => d.Id == id))
            {
                Assert.Equal(csharp.ToCompact(), ours.Get(field)?.ToCompact());
                ours.Set(field, row.Get(field)!);
            }
            Assert.Equal(row.ToCompact(), ours.ToCompact());
        }
    }
}

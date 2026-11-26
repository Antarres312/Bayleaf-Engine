using System.Globalization;
using System.Text;

using Bayleaf.Assets;
using Bayleaf.Core;

namespace Bayleaf.Animation.Tests;

/// <summary>
/// Every animation states file (<c>.banim</c>) of the examples, the benchmark and <c>tests/files</c>
/// reads, writes back, compiles and runs as in the Rust engine (docs/csharp-redesign.md). The Rust
/// engine's test (<c>rust/crates/animation/src/shared_files.rs</c>) writes into
/// <c>tests/files/expected</c> what it writes back after reading each with the animation states'
/// catalog (compared here byte for byte) and a summary of what it compiles to and how that runs on
/// the arm fixture (<c>&lt;name&gt;.summary</c>, compared word by word, numbers within a small tolerance).
/// </summary>
public class SharedFilesTests
{
    private const string Refused = "refused\n";

    internal static string RepositoryRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("the repository root is above the test's folder");
    }

    private static string Num(float v) =>
        float.IsNaN(v) ? "NaN" : float.IsInfinity(v) ? (v > 0 ? "inf" : "-inf") : v.ToString("F4", CultureInfo.InvariantCulture);

    private static string Quoted(string text) => Text.Debug(text);

    private static string B(bool v) => v ? "true" : "false";

    private static string NumberText(Number n) => n.Parameter is { } name ? $"parameter {Quoted(name)}" : $"value {Num(n.Value)}";

    private static string MotionText(Motion m) => m switch
    {
        Motion.Clip c => $"( clip {Quoted(c.Name)} speed {Num(c.Speed)} {(c.Looping ? "loop" : "once")} )",
        Motion.Mix x => $"( mix by {NumberText(x.By)} {string.Join(' ', x.Items.Select(i => $"{MotionText(i.Motion)} at {Num(i.At)}"))} )",
        Motion.Layer l => $"( layer joint {Quoted(l.Joint)} weight {NumberText(l.Weight)} base {MotionText(l.Base)} top {MotionText(l.Top)} )",
        _ => "rest",
    };

    private static string AmountText(Amount a) => a switch
    {
        Amount.Value v => $"value {Num(v.Number)}",
        Amount.Named n => $"parameter {Quoted(n.Parameter)}",
        Amount.Seconds => "seconds",
        _ => "runs",
    };

    private static string TestText(Test test) => test switch
    {
        Test.Greater => "greater",
        Test.Less => "less",
        Test.AtLeast => "at-least",
        Test.AtMost => "at-most",
        Test.Equal => "equal",
        _ => "not-equal",
    };

    private static string ConditionText(Condition c) => c switch
    {
        Condition.Value v => B(v.Holds),
        Condition.Flag f => $"flag {Quoted(f.Name)}",
        Condition.Trigger t => $"trigger {Quoted(t.Name)}",
        Condition.Compare x => $"( compare {AmountText(x.A)} {TestText(x.Test)} {AmountText(x.B)} )",
        Condition.Both both => $"( and {ConditionText(both.A)} {ConditionText(both.B)} )",
        Condition.Either either => $"( or {ConditionText(either.A)} {ConditionText(either.B)} )",
        Condition.Negated n => $"( not {ConditionText(n.Of)} )",
        _ => "?",
    };

    private static string PoseLine(Pose pose)
    {
        var s = new StringBuilder();
        for (var i = 0; i < pose.Locals.Length; i++)
        {
            var (p, r) = (pose.Locals[i].Translation, pose.Locals[i].Rotation);
            s.Append(CultureInfo.InvariantCulture, $" n{i} {Num(p.X)} {Num(p.Y)} {Num(p.Z)} {Num(r.X)} {Num(r.Y)} {Num(r.Z)} {Num(r.W)}");
        }
        return s.ToString();
    }

    /// <summary>The parameters a run sets before step <paramref name="i"/>, as the Rust test's schedule does.</summary>
    private static void Schedule(int i, Machine machine, Player player)
    {
        foreach (var (name, kind) in machine.Parameters)
        {
            Parameter? value = (i, kind) switch
            {
                (3 or 18 or 30, ParameterKind.Trigger) => Parameter.Trigger(true),
                (8, ParameterKind.Number) => Parameter.Number(1f),
                (24, ParameterKind.Number) => Parameter.Number(6f),
                (14, ParameterKind.Flag) => Parameter.Flag(true),
                (30, ParameterKind.Flag) => Parameter.Flag(false),
                _ => null,
            };
            if (value is { } v)
            {
                Assert.True(player.SetParameter(name, v).IsOk);
            }
        }
    }

    private static string MachineSummary(Machine machine, Skeleton skeleton, Clips clips)
    {
        var s = new StringBuilder();
        void Line(string text) => s.Append(text).Append('\n');
        var start = machine.Start < machine.States.Count ? machine.States[machine.Start].Name : "-";
        Line(string.Create(CultureInfo.InvariantCulture, $"states {machine.States.Count} start {Quoted(start)}"));
        foreach (var state in machine.States)
        {
            Line($"state {Quoted(state.Name)} {MotionText(state.Motion)}");
        }
        foreach (var t in machine.Transitions)
        {
            var from = t.From is { } f && f < machine.States.Count ? Quoted(machine.States[f].Name) : "any";
            var to = t.To < machine.States.Count ? machine.States[t.To].Name : "-";
            Line($"transition from {from} to {Quoted(to)} fade {Num(t.Fade)} when {ConditionText(t.When)}");
        }
        foreach (var (name, kind) in machine.Parameters)
        {
            Line($"parameter {Quoted(name)} {kind.Label()}");
        }
        foreach (var state in machine.States)
        {
            foreach (var problem in state.Motion.Problems(skeleton, clips))
            {
                Line($"lacks {Quoted(state.Name)} {problem}");
            }
        }
        var player = new Player { Root = Motion.DefaultRoot(skeleton) };
        player.RunStates(machine);
        for (var i = 0; i < 40; i++)
        {
            Schedule(i, machine, player);
            var step = player.Advance(skeleton, clips, 0.1f);
            var events = step.Events.Count == 0 ? "-" : string.Join(',', step.Events);
            Line(string.Create(CultureInfo.InvariantCulture,
                $"step {i} state {Quoted(player.State ?? "-")} fading {B(player.Layers[0].Fading)} playing {B(player.IsPlaying)} events {events} root {Num(step.Root.X)} {Num(step.Root.Y)} {Num(step.Root.Z)}{PoseLine(player.Pose(skeleton, clips))}"));
        }
        return s.ToString();
    }

    /// <summary>What the C# engine writes back after reading the file, or <c>refused</c>; and its summary.</summary>
    private static (string Written, string Summary) Rewrite(string path, Skeleton skeleton, Clips clips)
    {
        if (!Files.ReadText(path).TryGet(out var text, out _) || !Graph.Graph.FromJson(text, AnimationStates.Catalog).TryGet(out var graph, out _))
        {
            return (Refused, Refused);
        }
        string summary;
        if (AnimationStates.Compile(graph).TryGet(out var machine, out var problems))
        {
            summary = MachineSummary(machine, skeleton, clips);
        }
        else
        {
            summary = string.Concat(problems.Select(p => string.Create(CultureInfo.InvariantCulture, $"problem {(p.Node is { } node ? $"node {node}" : "graph")} {p.Message}\n")));
        }
        return (graph.ToJson(), summary);
    }

    /// <summary>Whether two summaries say the same: word by word, numbers within a small tolerance.</summary>
    private static string? Differences(string written, string expected)
    {
        var (a, b) = (written.Split('\n'), expected.Split('\n'));
        if (a.Length != b.Length)
        {
            return $"{a.Length} lines, not {b.Length}";
        }
        static bool Close(string p, string q) =>
            p == q || ((p.Contains('.', StringComparison.Ordinal) || q.Contains('.', StringComparison.Ordinal))
                && double.TryParse(p, NumberStyles.Float, CultureInfo.InvariantCulture, out var u) && double.TryParse(q, NumberStyles.Float, CultureInfo.InvariantCulture, out var v)
                && Math.Abs(u - v) <= 2e-4 + 2e-5 * Math.Abs(u));
        for (var n = 0; n < a.Length; n++)
        {
            var (xs, ys) = (a[n].Split(' '), b[n].Split(' '));
            if (xs.Length != ys.Length || xs.Zip(ys).Any(t => !Close(t.First, t.Second)))
            {
                return $"line {n + 1}: `{a[n]}` is not `{b[n]}`";
            }
        }
        return null;
    }

    public static TheoryData<string> SharedFiles()
    {
        var root = RepositoryRoot();
        var files = new TheoryData<string>();
        foreach (var dir in new[] { Path.Join(root, "examples"), Path.Join(root, "rust", "bench", "graphs"), Path.Join(root, "tests", "files") })
        {
            foreach (var file in Directory.EnumerateFiles(dir, "*." + AnimationStates.Catalog.Extension, SearchOption.AllDirectories).Order(StringComparer.Ordinal))
            {
                var relative = Path.GetRelativePath(root, file);
                if (!relative.Split(Path.DirectorySeparatorChar).Contains("expected"))
                {
                    files.Add(relative.Replace(Path.DirectorySeparatorChar, '/'));
                }
            }
        }
        return files;
    }

    private static string Expected(string name) => File.ReadAllText(Path.Join(RepositoryRoot(), "tests", "files", "expected", name)).ReplaceLineEndings("\n");

    [Fact]
    public void TheSharedFilesAreThere() => Assert.True(SharedFiles().Count >= 10);

    [Theory]
    [MemberData(nameof(SharedFiles))]
    public void EachFileReadsCompilesAndRunsAsInTheRustEngine(string relative)
    {
        var root = RepositoryRoot();
        var name = string.Join("__", relative.Split('/').SkipWhile(p => p is "tests" or "files"));
        var (skeleton, clips) = Arm.Load(("Stretch", new ClipEvent(0.25f, "step")), ("Stretch", new ClipEvent(0.75f, "land")));
        var (written, summary) = Rewrite(Path.Join(root, relative), skeleton, clips);
        Assert.Equal(Expected(name), written);
        var differences = Differences(summary, Expected($"{name}.summary"));
        Assert.True(differences is null, $"{relative}: {differences}\n{summary}");
    }
}

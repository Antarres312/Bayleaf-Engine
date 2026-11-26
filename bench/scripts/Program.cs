// The script step of the C# engine (docs/csharp-redesign.md, stage 4's gate), on the scene of
// rust/bench: a target, then N objects on a grid, each with a mesh, a collider, a tag, a point light
// on every tenth and three scripts (spin, bob, follow the target), or three empty scripts. The
// scripts are C# (game/), compiled and run by the engine's own script host.
//
//   --objects N           objects with scripts (default 20000)
//   --scripts KIND        csharp (default): a step per object; many: each script steps all its
//                         objects in one call; empty: three scripts whose step does nothing
//   --part PART           scripts (default): ScriptHost.Update, the call play mode makes each step;
//                         step: the whole play.step command
//   --warmup N --min-frames N --max-frames N --seconds S   as rust/bench (30, 200, 20000, 3)
//
// Prints one JSON object: what ran, the load time, a checksum over the scene, and the timed steps'
// mean, median, 5th and 95th percentiles in milliseconds, with the garbage collections during them.

using System.Diagnostics;
using System.Globalization;
using System.Text.Json;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Engine;
using Bayleaf.Scene.Components;
using Bayleaf.Script;

using SceneModel = Bayleaf.Scene.Scene;
using Transform = Bayleaf.Scene.Components.Transform;

const float Dt = 1f / 60f;

var options = new Dictionary<string, string>(StringComparer.Ordinal);
for (var i = 0; i + 1 < args.Length; i += 2)
{
    options[args[i].TrimStart('-')] = args[i + 1];
}
int Option(string name, int fallback) => options.TryGetValue(name, out var v) ? int.Parse(v, CultureInfo.InvariantCulture) : fallback;
var objects = Option("objects", 20_000);
var kind = options.GetValueOrDefault("scripts", "csharp");
var part = options.GetValueOrDefault("part", "scripts");
var (warmup, minFrames, maxFrames) = (Option("warmup", 30), Option("min-frames", 200), Option("max-frames", 20_000));
var seconds = options.TryGetValue("seconds", out var s) ? double.Parse(s, CultureInfo.InvariantCulture) : 3.0;
if (kind is not ("csharp" or "many" or "empty" or "spin" or "bob" or "follow") || part is not ("scripts" or "step"))
{
    Console.Error.WriteLine("usage: bench-scripts [--objects N] [--scripts csharp|many|empty] [--part scripts|step] [--warmup N] [--min-frames N] [--max-frames N] [--seconds S]");
    return 2;
}

// The project: the benchmark's scripts in a work folder.
var work = Path.Join(Path.GetTempPath(), $"bayleaf-bench-scripts-{System.Environment.ProcessId}");
Directory.CreateDirectory(Path.Join(work, "scripts"));
foreach (var file in Directory.EnumerateFiles(Path.Join(AppContext.BaseDirectory, "game"), "*.cs"))
{
    File.Copy(file, Path.Join(work, "scripts", Path.GetFileName(file)), true);
}

string[] names = kind switch
{
    "empty" => ["empty", "empty", "empty"],
    "many" => ["spin_many", "bob_many", "follow_many"],
    "spin" or "bob" or "follow" => [kind, kind, kind],
    _ => ["spin", "bob", "follow"],
};

// The same per-object values as rust/bench's `properties`.
SortedDictionary<string, PropertyValue> Properties(string name, int i)
{
    var values = new SortedDictionary<string, PropertyValue>(Utf8Order.Instance);
    void Set(string key, double value) => values[key] = new PropertyValue.Number(value);
    switch (name.Replace("_many", "", StringComparison.Ordinal))
    {
        case "spin":
            Set("speed", 30.0 + (i % 7) * 10.0);
            break;
        case "bob":
            Set("speed", 1.0 + (i % 5) * 0.25);
            Set("height", 0.25 + (i % 3) * 0.1);
            Set("base", 0.5);
            break;
        case "follow":
            Set("rate", 0.05 + (i % 4) * 0.05);
            break;
    }
    return values;
}

SceneModel Build()
{
    var scene = new SceneModel();
    var target = scene.CreateObject(null, "Target").Value;
    scene.Set(target, Transform.Initial);
    var side = Math.Max(1, (int)Math.Ceiling(Math.Sqrt(objects)));
    for (var i = 0; i < objects; i++)
    {
        var id = scene.CreateObject(null, string.Create(CultureInfo.InvariantCulture, $"Object {i}")).Value;
        scene.Set(id, Transform.Initial with { Position = new Double3((i % side * 2.0) - side, 0, (i / side * 2.0) - side) });
        scene.Set(id, Mesh.Initial);
        scene.Set(id, Collider.Initial);
        scene.Set(id, new Tags(["bench"]));
        if (i % 10 == 0)
        {
            scene.Set(id, PointLight.Initial);
        }
        scene.Set(id, new Scripts([.. names.Select(n => new ScriptRef($"scripts/{n}.cs", Properties(n, i)))]));
    }
    return scene;
}

double Checksum(SceneModel scene) =>
    ScriptHost.Order(scene).Sum(id => scene.TryGet<Transform>(id, out var t) ? t.Position.X + t.Position.Y + t.Position.Z + t.Rotation.Y : 0);

long allocated = 0;
var gcBefore = (0, 0, 0);
var parts = (Scripts: 0.0, Write: 0.0);

List<double> Measure(Action step)
{
    for (var i = 0; i < warmup; i++)
    {
        step();
    }
    var times = new List<double>(maxFrames);
    var bytes = GC.GetTotalAllocatedBytes(precise: true);
    gcBefore = (GC.CollectionCount(0), GC.CollectionCount(1), GC.CollectionCount(2));
    var clock = Stopwatch.StartNew();
    while (times.Count < maxFrames && (times.Count < minFrames || clock.Elapsed.TotalSeconds < seconds))
    {
        var started = Stopwatch.GetTimestamp();
        step();
        times.Add(Stopwatch.GetElapsedTime(started).TotalMilliseconds);
    }
    allocated = (GC.GetTotalAllocatedBytes(precise: true) - bytes) / Math.Max(1, times.Count);
    return times;
}

var scene = Build();
var loading = Stopwatch.StartNew();
List<double> timed;
int instances, running;
var collections = (0, 0, 0);
double check;
if (part == "scripts")
{
    var project = ScriptProject.Compile(work);
    if (!project.Compiled)
    {
        Console.Error.WriteLine(project.Summary());
        return 1;
    }
    using var host = new ScriptHost(project.Load().Value);
    host.LoadScene(scene, 0);
    var loadSeconds = loading.Elapsed.TotalSeconds;
    if (host.TakeFaults() is [var fault, ..])
    {
        Console.Error.WriteLine($"a script did not load: {fault.Error} ({fault.Script})");
        return 1;
    }
    var time = 0.0;
    System.Runtime.GCSettings.LatencyMode = System.Runtime.GCLatencyMode.SustainedLowLatency;
    timed = Measure(() =>
    {
        time += Dt;
        host.Update(scene, Dt, time);
        host.TakeLogs();
        parts.Scripts += host.LastStep.Scripts.TotalMilliseconds;
        parts.Write += host.LastStep.Write.TotalMilliseconds;
    });
    collections = (GC.CollectionCount(0) - gcBefore.Item1, GC.CollectionCount(1) - gcBefore.Item2, GC.CollectionCount(2) - gcBefore.Item3);
    (instances, running) = (host.Count, host.Running);
    check = Checksum(scene);
    Report(loadSeconds);
}
else
{
    var path = Path.Join(work, "bench.bscene");
    scene.Save(path);
    var registry = CommandRegistry.WithBuiltins();
    var session = new Session { ScriptsAllowedEverywhere = true };
    Run("scene.open", $$"""{"path": {{new JsonString(path).ToCompact()}}}""");
    loading.Restart();
    var started = Run("play.start", "{}");
    var loadSeconds = loading.Elapsed.TotalSeconds;
    var step = JsonReader.Parse($$"""{"seconds": {{Dt.ToString("R", CultureInfo.InvariantCulture)}}}""").Value;
    timed = Measure(() => registry.Execute(session, "play.step", step));
    collections = (GC.CollectionCount(0) - gcBefore.Item1, GC.CollectionCount(1) - gcBefore.Item2, GC.CollectionCount(2) - gcBefore.Item3);
    (instances, running) = ((int)(started.Get("scripts")?.AsDouble() ?? 0), (int)(started.Get("running")?.AsDouble() ?? 0));
    check = Checksum(session.Scene);
    Report(loadSeconds);

    JsonValue Run(string command, string parameters)
    {
        var result = registry.Execute(session, command, JsonReader.Parse(parameters).Value);
        if (result.IsError)
        {
            throw new InvalidOperationException($"{command}: {result.Error.Message}");
        }
        return result.Value;
    }
}
Directory.Delete(work, true);
return 0;

void Report(double loadSeconds)
{
    var sorted = timed.Order().ToList();
    double At(double q) => sorted[(int)Math.Round((sorted.Count - 1) * q)];
    var mean = sorted.Average();
    var output = new Dictionary<string, object>
    {
        ["scripts"] = kind,
        ["part"] = part,
        ["objects"] = objects,
        ["loadSeconds"] = Math.Round(loadSeconds, 3),
        ["instances"] = instances,
        ["running"] = running,
        ["check"] = check,
        ["frames"] = sorted.Count,
        ["meanMs"] = Math.Round(mean, 4),
        ["medianMs"] = Math.Round(At(0.5), 4),
        ["p5Ms"] = Math.Round(At(0.05), 4),
        ["p95Ms"] = Math.Round(At(0.95), 4),
        ["gcDuringTimedSteps"] = new[] { collections.Item1, collections.Item2, collections.Item3 },
        ["bytesPerStep"] = allocated,
        ["partsMeanMs"] = new { scripts = Math.Round(parts.Scripts / (sorted.Count + warmup), 4), write = Math.Round(parts.Write / (sorted.Count + warmup), 4) },
    };
    Console.WriteLine(JsonSerializer.Serialize(output));
}

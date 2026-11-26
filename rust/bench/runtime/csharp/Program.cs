// Times the small runtime of Runtime.cs running the game of Game.cs (bench/README.md, "The same
// runtime in C++ and Rust"). Prints one JSON line per style (typed, virtual) and part (scripts:
// every script's update; step: that, then world matrices of what moved and reading the moved list),
// as bench/runtime/rust/src/main.rs does, with what the garbage collector did during the timed steps.
//
// Build and run: dotnet run -c Release -- <objects>    NativeAOT: dotnet publish -c Release -p:PublishAot=true

using System.Diagnostics;
using System.Globalization;

using Bayleaf.Bench.Runtime;

const float Dt = 1f / 60f;
const int Warmup = 30;
const int MinFrames = 100;
const double MinSeconds = 3.0;
const int MaxFrames = 20000;
// Steps of the fixed run whose checksum every version must agree on.
const int CheckSteps = 100;

var n = args.Length > 0 && int.TryParse(args[0], CultureInfo.InvariantCulture, out var given) ? given : 5000;
var how = System.Runtime.CompilerServices.RuntimeFeature.IsDynamicCodeSupported ? "jit" : "aot";
foreach (var typed in new[] { true, false })
{
    foreach (var wholeStep in new[] { false, true })
    {
        Run(n, typed, wholeStep, how);
    }
}

static double Check(int n, bool typed)
{
    var runtime = new Runtime();
    Game.Build(runtime, n, typed);
    runtime.Start();
    var time = 0.0;
    for (var i = 0; i < CheckSteps; i++)
    {
        time += Dt;
        runtime.Update(new Frame(Dt, time));
        runtime.World.FinishStep();
    }
    return runtime.World.Checksum();
}

static void Run(int n, bool typed, bool wholeStep, string how)
{
    var runtime = new Runtime();
    Game.Build(runtime, n, typed);
    runtime.Start();
    runtime.World.FinishStep();
    var time = 0.0;
    var taken = 0.0;
    void Step()
    {
        time += Dt;
        runtime.Update(new Frame(Dt, time));
        if (wholeStep)
        {
            taken += runtime.World.FinishStep();
        }
        else
        {
            runtime.World.ForgetMoved();
        }
    }

    for (var i = 0; i < Warmup; i++)
    {
        Step();
    }
    var ms = new List<double>(MaxFrames);
    var collections = GC.CollectionCount(0);
    var allocated = GC.GetAllocatedBytesForCurrentThread();
    var started = Stopwatch.GetTimestamp();
    while (ms.Count < MaxFrames && (ms.Count < MinFrames || Stopwatch.GetElapsedTime(started).TotalSeconds < MinSeconds))
    {
        var t = Stopwatch.GetTimestamp();
        Step();
        ms.Add(Stopwatch.GetElapsedTime(t).TotalMilliseconds);
    }
    // The timing list was made before, at its full size, so what is counted here is the steps'.
    allocated = GC.GetAllocatedBytesForCurrentThread() - allocated;
    collections = GC.CollectionCount(0) - collections;
    GC.KeepAlive(taken);

    ms.Sort();
    var mean = ms.Average();
    var stdev = Math.Sqrt(ms.Sum(v => (v - mean) * (v - mean)) / ms.Count);
    double At(double q) => ms[(int)Math.Round((ms.Count - 1.0) * q, MidpointRounding.AwayFromZero)];
    var inv = CultureInfo.InvariantCulture;
    var line = string.Create(
        inv,
        $"{{\"mode\":\"csharp-{how}-runtime-{(typed ? "typed" : "virtual")}\",\"scripts\":\"csharp\",\"part\":\"{(wholeStep ? "step" : "scripts")}\",\"objects\":{n},\"check\":{Check(n, typed):0.000000000e+00},\"world\":{runtime.World.Count},\"data\":{runtime.World.DataSum():0.000000000e+00},\"gcCollections\":{collections},\"allocatedBytes\":{allocated},\"stats\":{{\"frames\":{ms.Count},\"meanMs\":{mean:0.000000},\"medianMs\":{At(0.5):0.000000},\"p5Ms\":{At(0.05):0.000000},\"p95Ms\":{At(0.95):0.000000},\"stdevMs\":{stdev:0.000000}}}}}");
    Console.WriteLine(line);
}

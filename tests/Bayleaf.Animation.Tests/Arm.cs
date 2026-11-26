using Bayleaf.Assets;

namespace Bayleaf.Animation.Tests;

/// <summary>The generated arm model (<see cref="Fixtures.SkinnedArm"/>): Root, Arm under it, Hand under Arm.</summary>
internal static class Arm
{
    private static int next;

    public static (Skeleton Skeleton, Clips Clips) Load(params (string Clip, ClipEvent Event)[] events)
    {
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-animation-cs-{Environment.ProcessId}-{Interlocked.Increment(ref next)}");
        Directory.CreateDirectory(dir);
        var model = Loader.LoadModel(Fixtures.SkinnedArm(dir).Value).Value;
        Directory.Delete(dir, true);
        var skeleton = model.Skeleton!;
        return (skeleton, Clips.OfModel(skeleton, events));
    }
}

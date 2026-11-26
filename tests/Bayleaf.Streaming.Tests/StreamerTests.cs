using Bayleaf.Core;
using Bayleaf.Core.Geometry;

namespace Bayleaf.Streaming.Tests;

public class StreamerTests
{
    private static Area AreaAt(StableId id, double x, string scene) => new(id, new Double3(x, 0, 0), scene, 50, 10);

    [Fact]
    public void AnAreaComesWithinItsDistanceAndGoesOnlyPastTheMargin()
    {
        var streamer = new Streamer();
        var id = StableId.Random();
        Area[] areas = [AreaAt(id, 100, "cell.bscene")];
        Double3[] At(double x) => [new Double3(x, 0, 0)];
        Assert.Empty(streamer.PlanFor(areas, At(0)).Read);
        Assert.Single(streamer.PlanFor(areas, At(60)).Read);
        Assert.Empty(streamer.PlanFor(areas, At(60)).Read);
        streamer.Placed(id, [StableId.Random()]);
        Assert.Empty(streamer.PlanFor(areas, At(45)).Remove);
        var away = streamer.PlanFor(areas, At(30));
        Assert.Single(away.Remove);
        Assert.Single(away.Remove[0].Objects);
        Assert.Empty(streamer.PlanFor(areas, []).Read);
    }

    [Fact]
    public void AChangedOrRemovedAreaGoes()
    {
        var streamer = new Streamer();
        var id = StableId.Random();
        Double3[] camera = [Double3.Zero];
        streamer.PlanFor([AreaAt(id, 0, "a.bscene")], camera);
        var changed = streamer.PlanFor([AreaAt(id, 0, "b.bscene")], camera);
        Assert.Single(changed.Remove);
        Assert.Single(changed.Read);
        Assert.Single(streamer.PlanFor([], camera).Remove);
    }

    [Fact]
    public void ScenesAreReadInTheBackgroundAndFailuresAreKept()
    {
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-streaming-cs-{Environment.ProcessId}");
        Directory.CreateDirectory(dir);
        try
        {
            var cell = new Scene.Scene();
            cell.CreateObject(null, "Rock");
            Assert.True(cell.Save(Path.Join(dir, "cell.bscene")).IsOk);
            var streamer = new Streamer();
            var (good, bad) = (StableId.Random(), StableId.Random());
            Area[] areas = [AreaAt(good, 0, "cell.bscene"), AreaAt(bad, 0, "missing.bscene")];
            foreach (var (id, scene) in streamer.PlanFor(areas, [Double3.Zero]).Read)
            {
                streamer.Read(id, Path.Join(dir, scene));
            }
            var arrived = streamer.Wait(TimeSpan.FromSeconds(10));
            Assert.Equal(2, arrived.Count);
            var read = arrived.First(a => a.Area == good).Scene.Value;
            Assert.Equal(1, read.ObjectCount);
            streamer.Placed(good, read.Children(null));
            Assert.Equal(0, streamer.Busy);
            Assert.Equal(StateKind.Failed, streamer.States.First(s => s.Area == bad).State.Kind);
            Assert.Empty(streamer.PlanFor(areas, [Double3.Zero]).Read);
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }
}

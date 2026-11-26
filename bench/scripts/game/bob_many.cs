// Benchmark script: `bob` written for many objects: one call moves every object.
public class BobMany : GameScript, IAdvanceMany<BobMany>
{
    [ScriptProperty] private float speed = 2f;
    [ScriptProperty] private float height = 0.5f;
    [ScriptProperty("base")] private float middle;

    public static void AdvanceMany(Many<BobMany> objects, float seconds)
    {
        var now = (float)Game.Time;
        var scripts = objects.Scripts;
        var poses = objects.Poses;
        for (var i = 0; i < scripts.Length; i++)
        {
            var bob = scripts[i];
            poses[i].Position.Y = bob.middle + bob.height * MathF.Sin(now * bob.speed);
        }
    }
}

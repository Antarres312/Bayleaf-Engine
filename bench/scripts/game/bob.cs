// Benchmark script: moves the object up and down. Properties: `speed` (radians per second),
// `height` (how far from `base`) and `base` (the middle height).
public class Bob : GameScript
{
    [ScriptProperty] private float speed = 2f;
    [ScriptProperty] private float height = 0.5f;
    [ScriptProperty("base")] private float middle;

    protected override void Advance(float seconds)
    {
        var pose = Pose;
        pose.Position.Y = middle + height * MathF.Sin((float)Time * speed);
        Pose = pose;
    }
}

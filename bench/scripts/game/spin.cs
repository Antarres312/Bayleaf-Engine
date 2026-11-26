// Benchmark script: turns the object about its Y axis. Property `speed`: degrees per second.
public class Spin : GameScript
{
    [ScriptProperty] private float speed = 90f;

    protected override void Advance(float seconds)
    {
        var pose = Pose;
        pose.Rotation.Y = (pose.Rotation.Y + speed * seconds) % 360f;
        Pose = pose;
    }
}

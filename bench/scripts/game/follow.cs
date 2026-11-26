// Benchmark script: eases the object towards the object named `Target` on the ground plane.
// Property `rate`: the part of the way covered per second.
public class Follow : GameScript
{
    [ScriptProperty] private float rate = 0.5f;
    private SceneObject target;

    protected override void Begin() => target = Game.Find("Target");

    protected override void Advance(float seconds)
    {
        if (target.Pose is not { } goal)
        {
            return;
        }
        var pose = Pose;
        var f = Math.Min(rate * seconds, 1f);
        pose.Position.X += (goal.Position.X - pose.Position.X) * f;
        pose.Position.Z += (goal.Position.Z - pose.Position.Z) * f;
        Pose = pose;
    }
}

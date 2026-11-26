// Benchmark script: `follow` written for many objects: one call eases every object towards `Target`.
public class FollowMany : GameScript, IAdvanceMany<FollowMany>
{
    [ScriptProperty] private float rate = 0.5f;
    private SceneObject target;

    protected override void Begin() => target = Game.Find("Target");

    public static void AdvanceMany(Many<FollowMany> objects, float seconds)
    {
        var scripts = objects.Scripts;
        var poses = objects.Poses;
        for (var i = 0; i < scripts.Length; i++)
        {
            var follow = scripts[i];
            if (follow.target.Pose is not { } goal)
            {
                continue;
            }
            ref var position = ref poses[i].Position;
            var f = Math.Min(follow.rate * seconds, 1f);
            position.X += (goal.Position.X - position.X) * f;
            position.Z += (goal.Position.Z - position.Z) * f;
        }
    }
}

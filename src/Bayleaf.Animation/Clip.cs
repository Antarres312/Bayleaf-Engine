using System.Numerics;

using Bayleaf.Assets;
using Bayleaf.Core;

namespace Bayleaf.Animation;

/// <summary>A moment a clip marks (a footstep): when it plays past <paramref name="Time"/> seconds, the object gets the script event <paramref name="Name"/>.</summary>
public sealed record ClipEvent(float Time, string Name);

/// <summary>
/// Something that poses named parts over time. A model's own animations are clips
/// (<see cref="ModelClip"/>); so is anything else that can say where each part is at a moment, such
/// as a timeline's (<see cref="NamedClip"/> is a ready one, its parts found by name).
/// </summary>
/// <remarks>
/// To give an object a clip of another kind, implement this and add it to the object's
/// <see cref="Clips"/> under a name (<see cref="Clips.Insert"/>); motions, animation states, scripts
/// and the <c>animation.*</c> commands then play it by that name like any other.
/// </remarks>
public interface IClipSource
{
    /// <summary>Seconds from its start to its end.</summary>
    float Duration { get; }

    /// <summary>The moments it marks, by rising time.</summary>
    IReadOnlyList<ClipEvent> Events { get; }

    /// <summary>
    /// Writes where the clip puts the parts it moves at <paramref name="time"/> seconds (from 0 to its
    /// duration) over <paramref name="pose"/>, finding them in <paramref name="skeleton"/> (each part
    /// is a node of it, by index or by name); parts it does not move keep their place.
    /// </summary>
    void Sample(Skeleton skeleton, float time, Pose pose);
}

/// <summary>One of a model's own animations, by its index in the model.</summary>
public sealed record ModelClip(int Animation, float Duration, IReadOnlyList<ClipEvent> Events) : IClipSource
{
    public void Sample(Skeleton skeleton, float time, Pose pose) => skeleton.SampleInto(Animation, time, pose);
}

/// <summary>Keys of one property of a part named <paramref name="Part"/>.</summary>
/// <param name="Step">Holds each key until the next instead of moving between them.</param>
/// <param name="Times">Seconds, rising.</param>
/// <param name="Values">x, y, z (and w for a rotation, as a quaternion) of each key.</param>
public sealed record NamedTrack(string Part, ChannelProperty Property, bool Step, IReadOnlyList<float> Times, IReadOnlyList<Vector4> Values);

/// <summary>
/// A clip that poses parts by their names: what a clip made outside the model (on a timeline)
/// becomes. A part no node of the skeleton is called is left out.
/// </summary>
public sealed record NamedClip(float Duration, IReadOnlyList<NamedTrack> Tracks, IReadOnlyList<ClipEvent> Events) : IClipSource
{
    public void Sample(Skeleton skeleton, float time, Pose pose)
    {
        foreach (var track in Tracks)
        {
            if (skeleton.NodeNamed(track.Part) is not { } node || node >= pose.Locals.Length
                || Skeleton.SampleKeys(track.Property, track.Step, track.Times, track.Values, time) is not { } v)
            {
                continue;
            }
            ref var local = ref pose.Locals[node];
            local = track.Property switch
            {
                ChannelProperty.Translation => local with { Translation = new(v.X, v.Y, v.Z) },
                ChannelProperty.Rotation => local with { Rotation = Numbers.Normalize(new Quaternion(v.X, v.Y, v.Z, v.W)) },
                _ => local with { Scale = new(v.X, v.Y, v.Z) },
            };
        }
    }
}

/// <summary>An object's clips by name.</summary>
public sealed class Clips
{
    private readonly SortedDictionary<string, IClipSource> byName = new(Utf8Order.Instance);

    /// <summary>
    /// <paramref name="skeleton"/>'s own animations, under their names, with the moments
    /// <paramref name="events"/> marks in them (clip and event pairs).
    /// </summary>
    public static Clips OfModel(Skeleton skeleton, IEnumerable<(string Clip, ClipEvent Event)> events)
    {
        var marks = events.ToList();
        var clips = new Clips();
        for (var index = 0; index < skeleton.Animations.Count; index++)
        {
            var animation = skeleton.Animations[index];
            var own = marks.Where(m => m.Clip == animation.Name).Select(m => m.Event).OrderBy(e => e.Time, Numbers.TotalOrder).ToList();
            clips.Insert(animation.Name, new ModelClip(index, animation.Duration, own));
        }
        return clips;
    }

    /// <summary>Adds <paramref name="clip"/> as <paramref name="name"/>, in place of one of that name.</summary>
    public void Insert(string name, IClipSource clip) => byName[name] = clip;

    public IClipSource? Get(string name) => byName.GetValueOrDefault(name);

    /// <summary>The clips' names, in order.</summary>
    public IEnumerable<string> Names => byName.Keys;
}

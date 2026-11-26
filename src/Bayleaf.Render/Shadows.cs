using System.Numerics;

namespace Bayleaf.Render;

// The shadow maps' views and the per-frame values the shaders read (common.wgsl's Globals).
public sealed partial class Renderer
{
    /// <summary>
    /// The light that casts shadows, the matrices from the world to its shadow map's faces (one, six
    /// for a point light, or one per sun cascade), and how far to push tested points away from their
    /// surface so a lit surface does not shadow itself.
    /// </summary>
    /// <param name="Mode">0: one face; 1: six faces around a point light; 2: sun cascades, picked by view depth.</param>
    /// <param name="Splits">View depths where the second, third and fourth cascades start.</param>
    /// <param name="Bias">In shadow-map depth units.</param>
    /// <param name="NormalOffset">Along the normal, in world units.</param>
    internal sealed record ShadowView(int Index, IReadOnlyList<Matrix4x4> Faces, float Mode, Vector4 Splits, float Bias, float NormalOffset);

    /// <summary>Scenes with a bounding radius above this get sun shadows in cascades.</summary>
    private const float CascadeMinRadius = 40f;

    /// <summary>Tangent of half a point-light face's view angle.</summary>
    private const float PointFaceSpread = 1.1f;

    /// <summary>Closest a spot light's shadow map sees, in world units.</summary>
    private const float SpotShadowNear = 0.1f;

    /// <summary>Depth bias of spot light shadows, in perspective depth units.</summary>
    private const float SpotShadowBias = 0.0005f;

    /// <summary>The directions and up vectors of a point light's faces, in the order the shader picks them: +X, -X, +Y, -Y, +Z, -Z.</summary>
    private static readonly (Vector3 Dir, Vector3 Up)[] PointFaces =
    [
        (Vector3.UnitX, -Vector3.UnitY), (-Vector3.UnitX, -Vector3.UnitY), (Vector3.UnitY, Vector3.UnitZ),
        (-Vector3.UnitY, -Vector3.UnitZ), (Vector3.UnitZ, -Vector3.UnitY), (-Vector3.UnitZ, -Vector3.UnitY),
    ];

    /// <summary>An up vector for looking along <paramref name="dir"/>.</summary>
    private static Vector3 UpFor(Vector3 dir) => Vector3.Cross(dir, Vector3.UnitY).LengthSquared() < 1e-6f ? Vector3.UnitZ : Vector3.UnitY;

    /// <summary>Where 1, 2, 3 or 4 cascades split the view depth they cover, as fractions of it: nearer slices smaller.</summary>
    private static float[] CascadeSplits(uint count) => count switch
    {
        <= 1 => [0f, 1f],
        2 => [0f, 0.15f, 1f],
        3 => [0f, 0.06f, 0.25f, 1f],
        _ => [0f, 0.04f, 0.12f, 0.35f, 1f],
    };

    private static float Round(float v) => MathF.Round(v, MidpointRounding.AwayFromZero);

    /// <summary>
    /// The lights that cast shadows, as many as <paramref name="settings"/> allow (sun-like lights come
    /// first, then the nearest), with their faces: a directional light seen orthographically so the map
    /// covers every drawn shape (or in cascades by view depth), a spot light through its cone, a point
    /// light through six faces around it.
    /// </summary>
    internal static List<ShadowView> ShadowViews(Frame frame, ShadowSettings settings)
    {
        var size = (float)Math.Max(settings.Resolution, 1);
        var views = new List<ShadowView>();
        foreach (var (light, index) in frame.Lights.Select((l, i) => (l, i)).Where(x => x.l.Shadows).Take((int)settings.Lights))
        {
            switch (light.Kind)
            {
                case LightKind.Directional:
                {
                    if (Extraction.ItemBounds(frame.Items.Where(i => !i.Unlit)) is not { } bounds)
                    {
                        continue;
                    }
                    var (center, radius) = (bounds.Center, Math.Max(bounds.Radius, 0.5f));
                    var dir = FMat.NormalizeOr(light.Direction, -Vector3.UnitY);
                    // Cascades follow the camera in a large scene, or when the shadow reaches less far than the scene.
                    var large = settings.Cascades > 1 && radius > CascadeMinRadius;
                    var limited = settings.Reach > 0 && settings.Reach < Vector3.Distance(frame.Camera.Position, center) + radius;
                    if ((large || limited) && frame.Camera.Proj.M44 == 0)
                    {
                        views.Add(Cascades(frame, index, dir, center, radius, settings));
                    }
                    else
                    {
                        var eye = center - dir * radius * 2f;
                        var matrix = FMat.LookAtRh(eye, center, UpFor(dir)) * FMat.OrthographicRh(-radius, radius, -radius, radius, 0.01f, radius * 4f);
                        // One texel of world size: the box width 2r maps to 2 in x.
                        var texel = 2f / Math.Max(new Vector3(matrix.M11, matrix.M12, matrix.M13).Length(), 1e-6f) / size;
                        // Depth per world unit along the light: the length of the matrix's depth row.
                        var bias = texel * new Vector3(matrix.M13, matrix.M23, matrix.M33).Length() * 1.5f;
                        views.Add(new ShadowView(index, [matrix], 0, Vector4.Zero, bias, texel * 1.5f));
                    }
                    break;
                }
                case LightKind.Spot:
                {
                    var half = Math.Clamp(MathF.Acos(Math.Clamp(light.CosOuter, -1f, 1f)), 0.01f, 1.55f);
                    var dir = FMat.NormalizeOr(light.Direction, -Vector3.UnitY);
                    var (near, far) = (SpotShadowNear, Math.Max(light.Range, SpotShadowNear * 2f));
                    var matrix = FMat.LookAtRh(light.Position, light.Position + dir, UpFor(dir)) * FMat.PerspectiveRh(half * 2f, 1f, near, far);
                    // Texel size a third of the way out, where shadows usually fall.
                    var texel = 2f * MathF.Tan(half) * far / 3f / size;
                    views.Add(new ShadowView(index, [matrix], 0, Vector4.Zero, SpotShadowBias, texel * 1.5f));
                    break;
                }
                default:
                {
                    var (near, far) = (SpotShadowNear, Math.Max(light.Range, SpotShadowNear * 2f));
                    // A little wider than a quarter turn, so the filter's taps near a face's edge stay on it.
                    var proj = FMat.PerspectiveRh(2f * MathF.Atan(PointFaceSpread), 1f, near, far);
                    var faces = PointFaces.Select(f => FMat.LookAtRh(light.Position, light.Position + f.Dir, f.Up) * proj).ToList();
                    var texel = 2f * PointFaceSpread * far / 3f / size;
                    views.Add(new ShadowView(index, faces, 1, Vector4.Zero, SpotShadowBias, texel * 1.5f));
                    break;
                }
            }
        }
        return views;
    }

    /// <summary>
    /// Shadow maps for a sun, one per cascade: each covers a slice of what the camera sees, nearer slices
    /// smaller, so shadows close by stay sharp in a large world. They reach to the far side of the scene,
    /// or as far as the settings say.
    /// </summary>
    private static ShadowView Cascades(Frame frame, int index, Vector3 dir, Vector3 sceneCenter, float sceneRadius, ShadowSettings settings)
    {
        var camera = frame.Camera;
        var forward = FMat.NormalizeOr(FMat.TransformVector(FMat.Inverse(camera.View), -Vector3.UnitZ), -Vector3.UnitZ);
        var inverse = FMat.Inverse(camera.ViewProj);
        var rays = new[] { (-1f, -1f), (1f, -1f), (1f, 1f), (-1f, 1f) }.Select(c => FMat.ProjectPoint(inverse, new Vector3(c.Item1, c.Item2, 1)) - camera.Position).ToList();
        var far = Math.Max(Vector3.Distance(camera.Position, sceneCenter) + sceneRadius, 1f);
        if (settings.Reach > 0)
        {
            far = Math.Min(far, settings.Reach);
        }
        var depths = CascadeSplits(settings.Cascades).Select(s => Math.Max(s * far, 0.05f)).ToList();
        var size = (float)Math.Max(settings.Resolution, 1);
        var faces = new List<Matrix4x4>();
        for (var slice = 0; slice + 1 < depths.Count; slice++)
        {
            var corners = new[] { depths[slice], depths[slice + 1] }.SelectMany(d => rays.Select(ray => camera.Position + ray * (d / Math.Max(Vector3.Dot(ray, forward), 1e-4f)))).ToList();
            var center = corners.Aggregate(Vector3.Zero, (a, b) => a + b) / corners.Count;
            var radius = Math.Max(corners.Aggregate(0f, (m, c) => Math.Max(m, Vector3.Distance(c, center))), 0.5f);
            // Far enough back along the light to catch every shape that can shadow the slice.
            var back = Vector3.Distance(center, sceneCenter) + sceneRadius + radius;
            var up = UpFor(dir);
            var eye = center - dir * back;
            // Moving in whole texels keeps shadow edges from crawling as the camera moves.
            var view = FMat.LookAtRh(eye, center, up);
            var texel = 2f * radius / size;
            var local = FMat.TransformPoint(view, center);
            var snapped = new Vector3(Round(local.X / texel) * texel, Round(local.Y / texel) * texel, local.Z);
            eye += FMat.TransformVector(FMat.Inverse(view), snapped - local);
            view = FMat.LookAtRh(eye, eye + dir, up);
            faces.Add(view * FMat.OrthographicRh(-radius, radius, -radius, radius, 0.01f, back * 2f + radius));
        }
        // Where each later cascade starts, then where the last one ends.
        Span<float> splits = [float.MaxValue, float.MaxValue, float.MaxValue, float.MaxValue];
        for (var slot = 0; slot < 4 && slot + 1 < depths.Count; slot++)
        {
            splits[slot] = depths[slot + 1];
        }
        return new ShadowView(index, faces, 2, new Vector4(splits[0], splits[1], splits[2], splits[3]), 0, 0);
    }

    private static double RemEuclid(double a, double b)
    {
        var r = a % b;
        return r < 0 ? r + Math.Abs(b) : r;
    }

    /// <summary>The per-frame values, laid out as common.wgsl's Globals.</summary>
    internal static float[] GlobalsFor(Frame frame, IReadOnlyList<ShadowView> shadows, uint shadowSize)
    {
        var g = new float[GlobalsFloats];
        var at = 0;
        void Vec(float x, float y, float z, float w)
        {
            g[at] = x;
            g[at + 1] = y;
            g[at + 2] = z;
            g[at + 3] = w;
            at += 4;
        }
        void Mat(Matrix4x4 m)
        {
            FMat.Write(g.AsSpan(at, 16), m);
            at += 16;
        }
        var count = (float)Math.Min(frame.Lights.Count, Extraction.MaxLights);
        var sky = frame.Sky ?? new Sky(Vector3.Zero, Vector3.Zero, Vector3.Zero);
        var viewProj = frame.Camera.ViewProj;
        Mat(viewProj);
        Vec(frame.Camera.Position.X, frame.Camera.Position.Y, frame.Camera.Position.Z, frame.Time);
        Vec(frame.Ambient.X, frame.Ambient.Y, frame.Ambient.Z, count);
        Mat(shadows.Count > 0 ? shadows[0].Faces[0] : Matrix4x4.Identity);
        Vec(Math.Min(shadows.Count, MaxShadowLights), 1f / Math.Max(shadowSize, 1), 0, 0);
        Vec(frame.AmbientGround.X, frame.AmbientGround.Y, frame.AmbientGround.Z, 0);
        Mat(FMat.Inverse(viewProj));
        Vec(sky.Zenith.X, sky.Zenith.Y, sky.Zenith.Z, 0);
        // w: 1 when there is a sky to reflect.
        Vec(sky.Horizon.X, sky.Horizon.Y, sky.Horizon.Z, frame.Sky is null ? 0 : 1);
        Vec(sky.Ground.X, sky.Ground.Y, sky.Ground.Z, 0);
        Vec(frame.Fog.Color.X, frame.Fog.Color.Y, frame.Fog.Color.Z, frame.Fog.Density);
        // Per light casting shadows: its index, depth bias, normal offset and first layer; its layers and
        // mode; its cascades' splits; then every layer's matrix.
        var lights = new Vector4[MaxShadowLights];
        var modes = new Vector4[MaxShadowLights];
        var splits = new Vector4[MaxShadowLights];
        var layers = Enumerable.Repeat(Matrix4x4.Identity, MaxShadowLayers).ToArray();
        Array.Fill(lights, new Vector4(-1, 0, 0, 0));
        var first = 0;
        for (var slot = 0; slot < Math.Min(shadows.Count, MaxShadowLights); slot++)
        {
            var view = shadows[slot];
            lights[slot] = new Vector4(view.Index, view.Bias, view.NormalOffset, first);
            modes[slot] = new Vector4(view.Faces.Count, view.Mode, 0, 0);
            splits[slot] = view.Splits;
            for (var f = 0; f < view.Faces.Count && first + f < MaxShadowLayers; f++)
            {
                layers[first + f] = view.Faces[f];
            }
            first += view.Faces.Count;
        }
        foreach (var v in lights.Concat(modes).Concat(splits))
        {
            Vec(v.X, v.Y, v.Z, v.W);
        }
        foreach (var m in layers)
        {
            Mat(m);
        }
        var origin = frame.Camera.Origin;
        // The ground grid in view coordinates: the world's position modulo 10 under the eye (the grid's
        // pattern) and the ground's height (world y = 0) from the eye; then where the world's X and Z
        // axes are from the eye.
        Vec((float)RemEuclid(origin.X, 10), (float)-origin.Y, (float)RemEuclid(origin.Z, 10), 0);
        Vec((float)-origin.Z, (float)-origin.X, 0, 0);
        Vec((float)origin.X, (float)origin.Y, (float)origin.Z, 0);
        for (var slot = 0; slot < Extraction.MaxLights; slot++)
        {
            if (slot >= frame.Lights.Count)
            {
                Vec(0, 0, 0, 0);
                Vec(0, 0, 0, 0);
                Vec(0, 0, 0, 0);
                Vec(0, 0, 0, 0);
                continue;
            }
            var light = frame.Lights[slot];
            var kind = light.Kind switch
            {
                LightKind.Directional => 0f,
                LightKind.Point => 1f,
                _ => 2f,
            };
            Vec(light.Direction.X, light.Direction.Y, light.Direction.Z, kind);
            Vec(light.Position.X, light.Position.Y, light.Position.Z, light.Range);
            Vec(light.Radiance.X, light.Radiance.Y, light.Radiance.Z, light.CosOuter);
            Vec(light.CosInner, 0, 0, 0);
        }
        return g;
    }
}

using System.Runtime.InteropServices;

using Bayleaf.Assets;
using Bayleaf.Native;
using Bayleaf.Scene;

namespace Bayleaf.Render;

/// <summary>
/// Post-processing: the passes after the scene that make the displayed picture from its high-range
/// colour (<c>post.wgsl</c>): bloom down and up a chain of half-size pictures, then one pass with
/// exposure, tone mapping, colour adjustments, vignette, depth fog, ambient occlusion and antialiasing.
/// With <see cref="PostSettings.Default"/> the picture is the scene's colour, clamped.
/// </summary>
internal sealed class Post : IDisposable
{
    /// <summary>Steps of the bloom chain, each half the size of the one before.</summary>
    public const int BloomLevels = 5;

    /// <summary>Id of the stand-in lookup texture.</summary>
    private const ulong NoLookup = ulong.MaxValue;

    private readonly GpuDevice device;
    private readonly GpuBindGroupLayout layout;
    private readonly GpuSampler sampler;
    private readonly GpuBuffer parameters;
    private readonly GpuShader shader;
    private readonly GpuPipelineLayout pipelineLayout;
    private readonly GpuPipeline finish;
    private readonly GpuPipeline bright;
    private readonly GpuPipeline down;
    private readonly GpuPipeline up;

    /// <summary>The lookup image on the GPU and the id of the image it is (a 1-pixel stand-in while there is none).</summary>
    private (ulong Id, GpuTexture Texture, GpuTextureView View) lookup;

    /// <summary>For a target of <paramref name="samples"/> per pixel: its depth is read multisampled, or plain with 1 sample.</summary>
    public Post(GpuDevice device, GpuBindGroupLayout globalsLayout, uint samples)
    {
        this.device = device;
        var multisampled = samples > 1;
        var source = multisampled ? Shaders.Post : Shaders.Post.Replace("texture_depth_multisampled_2d", "texture_depth_2d", StringComparison.Ordinal);
        layout = device.CreateBindGroupLayout("bayleaf post layout",
        [
            new GpuTextureLayout(0, GpuStages.Fragment, GpuSampleType.Filterable),
            new GpuTextureLayout(1, GpuStages.Fragment, GpuSampleType.Filterable),
            new GpuTextureLayout(2, GpuStages.Fragment, GpuSampleType.Depth, GpuViewDimension.D2, multisampled),
            new GpuSamplerLayout(3, GpuStages.Fragment, GpuSamplerType.Filtering),
            new GpuUniformLayout(4, GpuStages.Fragment),
            new GpuTextureLayout(5, GpuStages.Fragment, GpuSampleType.Filterable),
        ]);
        sampler = device.CreateSampler(new GpuSamplerDesc("bayleaf post sampler") { Linear = true });
        parameters = device.CreateBuffer("bayleaf post params", 24 * sizeof(float), GpuBufferUsage.Uniform | GpuBufferUsage.CopyDestination);
        shader = device.CreateShader("bayleaf post shader", source);
        pipelineLayout = device.CreatePipelineLayout("bayleaf post pipeline layout", [globalsLayout, layout]);
        GpuPipeline Make(string entry, GpuTextureFormat format, GpuBlend blend) => device.CreatePipeline(new GpuPipelineDesc(entry, pipelineLayout, shader, "vs_full", [])
        {
            FragmentEntry = entry,
            Target = new GpuColorTarget(format, blend),
        });
        finish = Make("fs_post", RenderTarget.ColorFormat, GpuBlend.Replace);
        bright = Make("fs_bright", RenderTarget.HdrFormat, GpuBlend.Replace);
        down = Make("fs_down", RenderTarget.HdrFormat, GpuBlend.Replace);
        up = Make("fs_up", RenderTarget.HdrFormat, GpuBlend.Add);
        var standIn = device.CreateTexture(new GpuTextureDesc("bayleaf no colour lookup", 1, 1, GpuTextureFormat.Rgba8Unorm, GpuTextureUsage.Binding));
        lookup = (NoLookup, standIn, standIn.CreateView());
    }

    /// <summary>The steps per side of a lookup image, when it is one.</summary>
    private static uint? LookupSteps(Image image) => ColorAdjustment.IsLookupSize((uint)image.Width, (uint)image.Height) ? (uint)image.Height : null;

    private static float Finite(float v, float fallback) => float.IsFinite(v) ? v : fallback;

    private static float[] Raw(PostSettings s, uint width, uint height)
    {
        var (fogColor, start, end, on) = s.Fog is { } fog ? (fog.Color, fog.Start, fog.End, 1f) : (default, 0f, 1f, 0f);
        var steps = s.Lookup is { } image ? LookupSteps(image) : null;
        return
        [
            Math.Max(Finite(s.Exposure, 1), 0), s.ToneMapping ? 1 : 0, Math.Max(Finite(s.Bloom, 0), 0), Math.Max(Finite(s.BloomThreshold, 1), 0),
            Finite(s.Contrast, 1), Finite(s.Saturation, 1), Math.Clamp(Finite(s.Vignette, 0), 0, 1), Math.Clamp(Finite(s.VignetteSmoothness, 0.5f), 0, 1),
            fogColor.X, fogColor.Y, fogColor.Z, on,
            Finite(start, 0), Finite(end, 1), Math.Clamp(Finite(s.Occlusion, 0), 0, 1), Math.Max(Finite(s.OcclusionRadius, 0.5f), 0.01f),
            (float)s.Antialiasing, 1f / Math.Max(width, 1), 1f / Math.Max(height, 1), s.Bloom > 0 ? 1 : 0,
            steps is { } n ? Math.Clamp(Finite(s.LookupStrength, 1), 0, 1) : 0, steps ?? 0, 0, 0,
        ];
    }

    private GpuBindGroup Group(GpuTextureView source, GpuTextureView bloom, GpuTextureView depth) => device.CreateBindGroup("bayleaf post group", layout,
    [
        new GpuViewBinding(0, source),
        new GpuViewBinding(1, bloom),
        new GpuViewBinding(2, depth),
        new GpuSamplerBinding(3, sampler),
        new GpuBufferBinding(4, parameters),
        new GpuViewBinding(5, lookup.View),
    ]);

    private static void Pass(GpuEncoder encoder, GpuTextureView view, bool clear, GpuPipeline pipeline, GpuBindGroup globals, GpuBindGroup group)
    {
        using var pass = encoder.BeginRenderPass("bayleaf post pass", new GpuColorAttachment(view, null, clear ? System.Numerics.Vector4.Zero : null), null);
        pass.SetPipeline(pipeline);
        pass.SetBindGroup(0, globals);
        pass.SetBindGroup(1, group);
        pass.Draw(3);
    }

    /// <summary>Makes the target's displayed picture from its high-range colour, as <paramref name="settings"/> say; the groups it made are added to <paramref name="made"/>, to let go after the frame is sent.</summary>
    public void Run(GpuEncoder encoder, RenderTarget target, GpuBindGroup globals, PostSettings settings, List<GpuObject> made)
    {
        device.WriteBuffer(parameters, 0, MemoryMarshal.AsBytes(Raw(settings, target.Width, target.Height).AsSpan()));
        if (settings.Lookup is { } image && LookupSteps(image) is not null && image.Id != lookup.Id)
        {
            var texture = device.CreateTexture(new GpuTextureDesc("bayleaf colour lookup", (uint)Math.Max(image.Width, 1), (uint)Math.Max(image.Height, 1), GpuTextureFormat.Rgba8Unorm, GpuTextureUsage.Binding), image.Rgba);
            lookup.View.Dispose();
            lookup.Texture.Dispose();
            lookup = (image.Id, texture, texture.CreateView());
        }
        GpuBindGroup Made(GpuBindGroup group)
        {
            made.Add(group);
            return group;
        }
        var bloom = target.BloomViews;
        if (settings.Bloom > 0)
        {
            // Bright parts into the first half-size picture, down the chain, then back up, adding.
            Pass(encoder, bloom[0], true, bright, globals, Made(Group(target.HdrView, bloom[1], target.DepthView)));
            for (var level = 1; level < bloom.Count; level++)
            {
                Pass(encoder, bloom[level], true, down, globals, Made(Group(bloom[level - 1], target.HdrView, target.DepthView)));
            }
            for (var level = bloom.Count - 2; level >= 0; level--)
            {
                Pass(encoder, bloom[level], false, up, globals, Made(Group(bloom[level + 1], target.HdrView, target.DepthView)));
            }
        }
        Pass(encoder, target.ColorView, true, finish, globals, Made(Group(target.HdrView, bloom[0], target.DepthView)));
    }

    public void Dispose()
    {
        foreach (var o in new GpuObject[] { finish, bright, down, up, pipelineLayout, shader, parameters, sampler, layout, lookup.View, lookup.Texture })
        {
            o.Dispose();
        }
    }
}

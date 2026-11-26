using Bayleaf.Native;

namespace Bayleaf.Render;

/// <summary>Offscreen colour, multisample and depth textures of one size.</summary>
public sealed class RenderTarget : IDisposable
{
    /// <summary>Colour format of the finished picture. sRGB, so shaders write linear colour.</summary>
    public const GpuTextureFormat ColorFormat = GpuTextureFormat.Rgba8UnormSrgb;

    /// <summary>The scene's own colour, before post-processing makes the displayed picture: high range, linear.</summary>
    internal const GpuTextureFormat HdrFormat = GpuTextureFormat.Rgba16Float;

    internal const GpuTextureFormat DepthFormat = GpuTextureFormat.Depth32Float;

    /// <summary>The selection mask: one channel, set where a selected shape is.</summary>
    internal const GpuTextureFormat MaskFormat = GpuTextureFormat.R8Unorm;

    /// <summary>Samples per pixel of a target made without a count; 1 draws without multisampling.</summary>
    public const uint DefaultSamples = 4;

    private readonly List<GpuObject> owned = [];

    /// <summary>A target drawn with <paramref name="samples"/> per pixel: 1 for none, else 4 (the count every device supports).</summary>
    public RenderTarget(GpuDevice device, uint width, uint height, uint samples = DefaultSamples)
    {
        Samples = samples <= 1 ? 1u : 4u;
        Width = Math.Max(width, 1);
        Height = Math.Max(height, 1);
        GpuTexture Texture(string label, GpuTextureFormat format, uint count, GpuTextureUsage usage, uint w, uint h) =>
            Own(device.CreateTexture(new GpuTextureDesc(label, w, h, format, usage) { Samples = count }));
        GpuTextureView View(GpuTexture texture) => Own(texture.CreateView());
        Color = Texture("bayleaf color", ColorFormat, 1, GpuTextureUsage.RenderTarget | GpuTextureUsage.Binding | GpuTextureUsage.CopySource, Width, Height);
        ColorView = View(Color);
        MsaaView = Samples > 1 ? View(Texture("bayleaf msaa", HdrFormat, Samples, GpuTextureUsage.RenderTarget, Width, Height)) : null;
        HdrView = View(Texture("bayleaf scene colour", HdrFormat, 1, GpuTextureUsage.RenderTarget | GpuTextureUsage.Binding, Width, Height));
        DepthView = View(Texture("bayleaf depth", DepthFormat, Samples, GpuTextureUsage.RenderTarget | GpuTextureUsage.Binding, Width, Height));
        MaskView = View(Texture("bayleaf selection mask", MaskFormat, 1, GpuTextureUsage.RenderTarget | GpuTextureUsage.Binding, Width, Height));
        BloomViews = [.. Enumerable.Range(0, Post.BloomLevels).Select(level =>
            View(Texture("bayleaf bloom", HdrFormat, 1, GpuTextureUsage.RenderTarget | GpuTextureUsage.Binding, Math.Max(Width >> (level + 1), 1), Math.Max(Height >> (level + 1), 1))))];
    }

    private T Own<T>(T value)
        where T : GpuObject
    {
        owned.Add(value);
        return value;
    }

    public uint Width { get; }

    public uint Height { get; }

    /// <summary>Samples per pixel: 1 without multisampling.</summary>
    public uint Samples { get; }

    /// <summary>The resolved colour texture, for display or copying.</summary>
    public GpuTexture Color { get; }

    public GpuTextureView ColorView { get; }

    /// <summary>The multisampled colour, drawn into and resolved into <see cref="HdrView"/>; none with 1 sample.</summary>
    internal GpuTextureView? MsaaView { get; }

    /// <summary>The scene's colour resolved from the multisampled one, which post-processing reads.</summary>
    internal GpuTextureView HdrView { get; }

    internal GpuTextureView DepthView { get; }

    /// <summary>Where the selected shapes are, for their outline.</summary>
    internal GpuTextureView MaskView { get; }

    /// <summary>Bloom's chain of half-size pictures.</summary>
    internal IReadOnlyList<GpuTextureView> BloomViews { get; }

    /// <summary>
    /// A copy of what was last drawn, in a texture of its own: a picture kept without the target's
    /// multisampled and depth buffers, which the next drawing can reuse.
    /// </summary>
    public (GpuTexture Texture, GpuTextureView View) Snapshot(GpuDevice device)
    {
        var copy = device.CreateTexture(new GpuTextureDesc("bayleaf snapshot", Width, Height, ColorFormat, GpuTextureUsage.Binding | GpuTextureUsage.CopyDestination));
        using var encoder = device.CreateEncoder("bayleaf snapshot");
        encoder.CopyTextureToTexture(Color, copy);
        device.Submit(encoder.Finish());
        return (copy, copy.CreateView());
    }

    public void Dispose()
    {
        // Views before their textures.
        foreach (var o in Enumerable.Reverse(owned))
        {
            o.Dispose();
        }
        owned.Clear();
    }
}

using System.Numerics;

namespace Bayleaf.Native;

/// <summary>The texture formats the renderer uses.</summary>
public enum GpuTextureFormat
{
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Rgba16Float,
    Rgba32Float,
    R8Unorm,
    Depth32Float,
    Bc1RgbaUnorm,
    Bc1RgbaUnormSrgb,
    Bc3RgbaUnorm,
    Bc3RgbaUnormSrgb,
}

/// <summary>What a texture may be used for.</summary>
[Flags]
public enum GpuTextureUsage
{
    None = 0,
    CopySource = 1,
    CopyDestination = 2,
    Binding = 4,
    RenderTarget = 16,
}

/// <summary>What a buffer may be used for.</summary>
[Flags]
public enum GpuBufferUsage
{
    None = 0,
    MapRead = 1,
    CopySource = 4,
    CopyDestination = 8,
    Index = 16,
    Vertex = 32,
    Uniform = 64,
}

/// <summary>The shader stages that see a binding.</summary>
[Flags]
public enum GpuStages
{
    None = 0,
    Vertex = 1,
    Fragment = 2,
}

/// <summary>How a shader reads a bound texture.</summary>
public enum GpuSampleType
{
    /// <summary>Colour, filtered between texels.</summary>
    Filterable,

    /// <summary>Numbers read texel by texel.</summary>
    Unfilterable,

    Depth,
}

public enum GpuViewDimension
{
    D2,
    D2Array,
}

public enum GpuSamplerType
{
    Filtering,
    Comparison,
}

public enum GpuVertexFormat
{
    Float32x2,
    Float32x3,
    Float32x4,
    Uint16x4,
}

public enum GpuCompare
{
    LessEqual,
    Always,
}

public enum GpuCull
{
    None,
    Front,
}

/// <summary>How a pipeline's colour joins what is already drawn.</summary>
public enum GpuBlend
{
    /// <summary>Written over it.</summary>
    Replace,

    /// <summary>Over it by its opacity.</summary>
    Alpha,

    /// <summary>Its colour, times its opacity, added; the opacity there kept.</summary>
    Additive,

    /// <summary>Its colour added; its opacity written.</summary>
    Add,
}

public sealed record GpuTextureDesc(string Label, uint Width, uint Height, GpuTextureFormat Format, GpuTextureUsage Usage)
{
    public uint Layers { get; init; } = 1;

    public uint MipLevels { get; init; } = 1;

    public uint Samples { get; init; } = 1;

    /// <summary>The one other format views of it may read it as (its sRGB or plain twin).</summary>
    public GpuTextureFormat? ViewFormat { get; init; }
}

/// <summary>A view of a texture: its own format, a dimension and some of its layers; null fields keep the texture's.</summary>
public sealed record GpuViewDesc(GpuTextureFormat? Format = null, GpuViewDimension? Dimension = null, uint BaseLayer = 0, uint? Layers = null);

public sealed record GpuSamplerDesc(string Label)
{
    /// <summary>Repeats the image across and down (else its edge is kept).</summary>
    public bool Repeat { get; init; }

    /// <summary>Blends neighbouring pixels (else the nearest).</summary>
    public bool Linear { get; init; }

    /// <summary>Blends between the smaller copies too.</summary>
    public bool LinearMipmaps { get; init; }

    public ushort Anisotropy { get; init; } = 1;

    /// <summary>A comparison sampler (less or equal), for shadow maps.</summary>
    public bool Compare { get; init; }
}

public abstract record GpuLayoutEntry(uint Binding, GpuStages Visibility);

public sealed record GpuUniformLayout(uint Binding, GpuStages Visibility, bool DynamicOffset = false, ulong MinSize = 0) : GpuLayoutEntry(Binding, Visibility);

public sealed record GpuTextureLayout(uint Binding, GpuStages Visibility, GpuSampleType Sample, GpuViewDimension Dimension = GpuViewDimension.D2, bool Multisampled = false)
    : GpuLayoutEntry(Binding, Visibility);

public sealed record GpuSamplerLayout(uint Binding, GpuStages Visibility, GpuSamplerType Type) : GpuLayoutEntry(Binding, Visibility);

public abstract record GpuBinding(uint Binding);

/// <summary>A buffer bound from <paramref name="Offset"/>, <paramref name="Size"/> bytes of it (all of it when null).</summary>
public sealed record GpuBufferBinding(uint Binding, GpuBuffer Buffer, ulong Offset = 0, ulong? Size = null) : GpuBinding(Binding);

public sealed record GpuViewBinding(uint Binding, GpuTextureView View) : GpuBinding(Binding);

public sealed record GpuSamplerBinding(uint Binding, GpuSampler Sampler) : GpuBinding(Binding);

public sealed record GpuVertexInput(GpuVertexFormat Format, ulong Offset, uint Location);

/// <summary>A vertex buffer's layout: bytes per vertex (or per instance) and its attributes.</summary>
public sealed record GpuVertexBuffer(ulong Stride, bool PerInstance, IReadOnlyList<GpuVertexInput> Attributes);

public sealed record GpuDepthState(GpuTextureFormat Format, bool Write, GpuCompare Compare, int Bias = 0, float SlopeScale = 0);

public sealed record GpuColorTarget(GpuTextureFormat Format, GpuBlend Blend);

public sealed record GpuPipelineDesc(string Label, GpuPipelineLayout Layout, GpuShader Shader, string VertexEntry, IReadOnlyList<GpuVertexBuffer> Buffers)
{
    /// <summary>No fragment stage when null (a depth-only pass).</summary>
    public string? FragmentEntry { get; init; }

    public GpuColorTarget? Target { get; init; }

    public GpuDepthState? Depth { get; init; }

    public GpuCull Cull { get; init; }

    public uint Samples { get; init; } = 1;
}

/// <summary>A colour attachment: cleared to <paramref name="Clear"/> (else what is there is kept), resolved into <paramref name="Resolve"/>.</summary>
public sealed record GpuColorAttachment(GpuTextureView View, GpuTextureView? Resolve, Vector4? Clear, bool Store = true);

public sealed record GpuDepthAttachment(GpuTextureView View, float Clear = 1f, bool Store = true);

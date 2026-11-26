using Silk.NET.WebGPU;

namespace Bayleaf.Native;

/// <summary>Our descriptions in wgpu-native's numbers.</summary>
internal static class Conv
{
    public static TextureFormat Format(GpuTextureFormat format) => format switch
    {
        GpuTextureFormat.Rgba8Unorm => TextureFormat.Rgba8Unorm,
        GpuTextureFormat.Rgba8UnormSrgb => TextureFormat.Rgba8UnormSrgb,
        GpuTextureFormat.Rgba16Float => TextureFormat.Rgba16float,
        GpuTextureFormat.Rgba32Float => TextureFormat.Rgba32float,
        GpuTextureFormat.R8Unorm => TextureFormat.R8Unorm,
        GpuTextureFormat.Depth32Float => TextureFormat.Depth32float,
        GpuTextureFormat.Bc1RgbaUnorm => TextureFormat.BC1RgbaUnorm,
        GpuTextureFormat.Bc1RgbaUnormSrgb => TextureFormat.BC1RgbaUnormSrgb,
        GpuTextureFormat.Bc3RgbaUnorm => TextureFormat.BC3RgbaUnorm,
        GpuTextureFormat.Bc3RgbaUnormSrgb => TextureFormat.BC3RgbaUnormSrgb,
        _ => TextureFormat.Undefined,
    };

    /// <summary>Texels a block of the format is wide (and high), and its bytes.</summary>
    public static (uint Side, uint Bytes) Block(GpuTextureFormat format) => format switch
    {
        GpuTextureFormat.Rgba8Unorm or GpuTextureFormat.Rgba8UnormSrgb or GpuTextureFormat.Depth32Float => (1, 4),
        GpuTextureFormat.Rgba16Float => (1, 8),
        GpuTextureFormat.Rgba32Float => (1, 16),
        GpuTextureFormat.R8Unorm => (1, 1),
        GpuTextureFormat.Bc1RgbaUnorm or GpuTextureFormat.Bc1RgbaUnormSrgb => (4, 8),
        _ => (4, 16),
    };

    public static TextureUsage Usage(GpuTextureUsage usage) => (TextureUsage)(uint)usage;

    public static BufferUsage Usage(GpuBufferUsage usage) => (BufferUsage)(uint)usage;

    public static ShaderStage Stages(GpuStages stages) => (ShaderStage)(uint)stages;

    public static VertexFormat Vertex(GpuVertexFormat format) => format switch
    {
        GpuVertexFormat.Float32x2 => VertexFormat.Float32x2,
        GpuVertexFormat.Float32x3 => VertexFormat.Float32x3,
        GpuVertexFormat.Float32x4 => VertexFormat.Float32x4,
        _ => VertexFormat.Uint16x4,
    };

    public static CompareFunction Compare(GpuCompare compare) => compare == GpuCompare.Always ? CompareFunction.Always : CompareFunction.LessEqual;

    public static BlendState? Blend(GpuBlend blend)
    {
        static BlendComponent Part(BlendFactor src, BlendFactor dst) => new() { Operation = BlendOperation.Add, SrcFactor = src, DstFactor = dst };
        return blend switch
        {
            GpuBlend.Alpha => new BlendState { Color = Part(BlendFactor.SrcAlpha, BlendFactor.OneMinusSrcAlpha), Alpha = Part(BlendFactor.One, BlendFactor.OneMinusSrcAlpha) },
            GpuBlend.Additive => new BlendState { Color = Part(BlendFactor.SrcAlpha, BlendFactor.One), Alpha = Part(BlendFactor.Zero, BlendFactor.One) },
            GpuBlend.Add => new BlendState { Color = Part(BlendFactor.One, BlendFactor.One), Alpha = Part(BlendFactor.One, BlendFactor.Zero) },
            _ => null,
        };
    }
}

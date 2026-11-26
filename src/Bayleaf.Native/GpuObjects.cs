using Silk.NET.WebGPU;

namespace Bayleaf.Native;

/// <summary>An object wgpu-native made, released when disposed (its last user then lets it go).</summary>
public abstract class GpuObject : IDisposable
{
    private protected GpuObject(nint handle) => Handle = handle;

    internal nint Handle { get; private set; }

    public void Dispose()
    {
        Dispose(true);
        GC.SuppressFinalize(this);
    }

    protected virtual void Dispose(bool disposing)
    {
        if (Handle != 0)
        {
            Release(Handle);
            Handle = 0;
        }
    }

    private protected abstract void Release(nint handle);
}

public sealed class GpuBuffer : GpuObject
{
    internal GpuBuffer(nint handle, ulong size)
        : base(handle) => Size = size;

    public ulong Size { get; }

    // SAFETY: the handle is a live buffer made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.BufferRelease((Silk.NET.WebGPU.Buffer*)handle);
}

public sealed class GpuTexture : GpuObject
{
    internal GpuTexture(nint handle, GpuTextureDesc desc)
        : base(handle) => Desc = desc;

    public GpuTextureDesc Desc { get; }

    public uint Width => Desc.Width;

    public uint Height => Desc.Height;

    /// <summary>A view of the texture; the whole texture as it is with no description.</summary>
    public GpuTextureView CreateView(GpuViewDesc? desc = null)
    {
        // SAFETY: the texture is live (checked), and the descriptor lives on the stack for the call.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            if (desc is null)
            {
                return new GpuTextureView((nint)GpuDevice.Api.TextureCreateView((Texture*)Handle, null));
            }
            var view = new TextureViewDescriptor
            {
                Format = desc.Format is { } f ? Conv.Format(f) : TextureFormat.Undefined,
                Dimension = desc.Dimension switch
                {
                    GpuViewDimension.D2 => TextureViewDimension.Dimension2D,
                    GpuViewDimension.D2Array => TextureViewDimension.Dimension2DArray,
                    _ => TextureViewDimension.DimensionUndefined,
                },
                BaseMipLevel = 0,
                MipLevelCount = uint.MaxValue,
                BaseArrayLayer = desc.BaseLayer,
                ArrayLayerCount = desc.Layers ?? uint.MaxValue,
                Aspect = TextureAspect.All,
            };
            return new GpuTextureView((nint)GpuDevice.Api.TextureCreateView((Texture*)Handle, &view));
        }
    }

    // SAFETY: the handle is a live texture made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.TextureRelease((Texture*)handle);
}

public sealed class GpuTextureView : GpuObject
{
    internal GpuTextureView(nint handle)
        : base(handle)
    {
    }

    // SAFETY: the handle is a live view made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.TextureViewRelease((TextureView*)handle);
}

public sealed class GpuSampler : GpuObject
{
    internal GpuSampler(nint handle)
        : base(handle)
    {
    }

    // SAFETY: the handle is a live sampler made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.SamplerRelease((Sampler*)handle);
}

public sealed class GpuBindGroupLayout : GpuObject
{
    internal GpuBindGroupLayout(nint handle)
        : base(handle)
    {
    }

    // SAFETY: the handle is a live layout made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.BindGroupLayoutRelease((BindGroupLayout*)handle);
}

public sealed class GpuBindGroup : GpuObject
{
    internal GpuBindGroup(nint handle)
        : base(handle)
    {
    }

    // SAFETY: the handle is a live bind group made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.BindGroupRelease((BindGroup*)handle);
}

public sealed class GpuPipelineLayout : GpuObject
{
    internal GpuPipelineLayout(nint handle)
        : base(handle)
    {
    }

    // SAFETY: the handle is a live pipeline layout made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.PipelineLayoutRelease((PipelineLayout*)handle);
}

public sealed class GpuShader : GpuObject
{
    internal GpuShader(nint handle)
        : base(handle)
    {
    }

    // SAFETY: the handle is a live shader module made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.ShaderModuleRelease((ShaderModule*)handle);
}

public sealed class GpuPipeline : GpuObject
{
    internal GpuPipeline(nint handle)
        : base(handle)
    {
    }

    // SAFETY: the handle is a live render pipeline made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.RenderPipelineRelease((RenderPipeline*)handle);
}

/// <summary>Recorded commands, ready to submit once.</summary>
public sealed class GpuCommands : GpuObject
{
    internal GpuCommands(nint handle)
        : base(handle)
    {
    }

    // SAFETY: the handle is a live command buffer made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.CommandBufferRelease((CommandBuffer*)handle);
}

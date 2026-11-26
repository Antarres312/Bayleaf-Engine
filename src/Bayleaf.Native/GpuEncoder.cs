using Silk.NET.WebGPU;

namespace Bayleaf.Native;

/// <summary>Records the commands of a frame; <see cref="Finish"/> ends it.</summary>
public sealed class GpuEncoder : GpuObject
{
    /// <summary>Bytes a row of a texture copied into a buffer is padded to.</summary>
    public const uint RowAlignment = 256;

    internal GpuEncoder(nint handle)
        : base(handle)
    {
    }

    /// <summary>A render pass into <paramref name="color"/> and <paramref name="depth"/>; disposing it ends it.</summary>
    public GpuRenderPass BeginRenderPass(string label, GpuColorAttachment? color, GpuDepthAttachment? depth)
    {
        // SAFETY: the encoder is live (checked); the descriptor and the attachments live in the arena
        // for the call, and the views they name are live objects of ours.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            using var arena = new Arena();
            var desc = arena.Alloc<RenderPassDescriptor>();
            desc->Label = arena.Text(label);
            if (color is not null)
            {
                var attachment = arena.Alloc<RenderPassColorAttachment>();
                attachment->View = (TextureView*)color.View.Handle;
                attachment->DepthSlice = uint.MaxValue;
                attachment->ResolveTarget = color.Resolve is null ? null : (TextureView*)color.Resolve.Handle;
                attachment->LoadOp = color.Clear is null ? LoadOp.Load : LoadOp.Clear;
                attachment->StoreOp = color.Store ? StoreOp.Store : StoreOp.Discard;
                var c = color.Clear.GetValueOrDefault();
                attachment->ClearValue = new Color { R = c.X, G = c.Y, B = c.Z, A = c.W };
                desc->ColorAttachmentCount = 1;
                desc->ColorAttachments = attachment;
            }
            if (depth is not null)
            {
                var attachment = arena.Alloc<RenderPassDepthStencilAttachment>();
                attachment->View = (TextureView*)depth.View.Handle;
                attachment->DepthLoadOp = LoadOp.Clear;
                attachment->DepthStoreOp = depth.Store ? StoreOp.Store : StoreOp.Discard;
                attachment->DepthClearValue = depth.Clear;
                attachment->StencilLoadOp = LoadOp.Undefined;
                attachment->StencilStoreOp = StoreOp.Undefined;
                desc->DepthStencilAttachment = attachment;
            }
            return new GpuRenderPass((nint)GpuDevice.Api.CommandEncoderBeginRenderPass((CommandEncoder*)Handle, desc));
        }
    }

    /// <summary>Copies a texture's first level into a buffer, each row padded to <paramref name="bytesPerRow"/>.</summary>
    public void CopyTextureToBuffer(GpuTexture texture, GpuBuffer buffer, uint bytesPerRow, uint width, uint height)
    {
        // SAFETY: the encoder, the texture and the buffer are live; the descriptors live on the stack for the call.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            var source = new ImageCopyTexture { Texture = (Texture*)texture.Handle, MipLevel = 0, Aspect = TextureAspect.All };
            var destination = new ImageCopyBuffer { Buffer = (Silk.NET.WebGPU.Buffer*)buffer.Handle, Layout = new TextureDataLayout { Offset = 0, BytesPerRow = bytesPerRow, RowsPerImage = height } };
            var size = new Extent3D { Width = width, Height = height, DepthOrArrayLayers = 1 };
            GpuDevice.Api.CommandEncoderCopyTextureToBuffer((CommandEncoder*)Handle, &source, &destination, &size);
        }
    }

    /// <summary>Copies the first level of one texture into another of the same size and format.</summary>
    public void CopyTextureToTexture(GpuTexture source, GpuTexture destination)
    {
        // SAFETY: the encoder and both textures are live; the descriptors live on the stack for the call.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            var from = new ImageCopyTexture { Texture = (Texture*)source.Handle, MipLevel = 0, Aspect = TextureAspect.All };
            var to = new ImageCopyTexture { Texture = (Texture*)destination.Handle, MipLevel = 0, Aspect = TextureAspect.All };
            var size = new Extent3D { Width = source.Width, Height = source.Height, DepthOrArrayLayers = 1 };
            GpuDevice.Api.CommandEncoderCopyTextureToTexture((CommandEncoder*)Handle, &from, &to, &size);
        }
    }

    /// <summary>The recorded commands; the encoder cannot record more.</summary>
    public GpuCommands Finish()
    {
        // SAFETY: the encoder is live (checked); a null descriptor asks for the defaults.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            return new GpuCommands((nint)GpuDevice.Api.CommandEncoderFinish((CommandEncoder*)Handle, null));
        }
    }

    // SAFETY: the handle is a live encoder made by wgpu-native, released once here.
    private protected override unsafe void Release(nint handle) => GpuDevice.Api.CommandEncoderRelease((CommandEncoder*)handle);
}

/// <summary>A render pass being recorded; disposing it ends the pass.</summary>
public sealed class GpuRenderPass : GpuObject
{
    internal GpuRenderPass(nint handle)
        : base(handle)
    {
    }

    public void SetPipeline(GpuPipeline pipeline)
    {
        // SAFETY: the pass and the pipeline are live objects of ours.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            GpuDevice.Api.RenderPassEncoderSetPipeline((RenderPassEncoder*)Handle, (RenderPipeline*)pipeline.Handle);
        }
    }

    /// <summary>Binds a group at <paramref name="index"/>, at <paramref name="dynamicOffset"/> for a layout with a dynamic offset.</summary>
    public void SetBindGroup(uint index, GpuBindGroup group, uint? dynamicOffset = null)
    {
        // SAFETY: the pass and the group are live; the offset lives on the stack for the call.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            var offset = dynamicOffset.GetValueOrDefault();
            GpuDevice.Api.RenderPassEncoderSetBindGroup((RenderPassEncoder*)Handle, index, (BindGroup*)group.Handle, dynamicOffset is null ? 0u : 1u, dynamicOffset is null ? null : &offset);
        }
    }

    /// <summary>The whole of <paramref name="buffer"/> as vertex buffer <paramref name="slot"/>.</summary>
    public void SetVertexBuffer(uint slot, GpuBuffer buffer)
    {
        // SAFETY: the pass and the buffer are live objects of ours.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            GpuDevice.Api.RenderPassEncoderSetVertexBuffer((RenderPassEncoder*)Handle, slot, (Silk.NET.WebGPU.Buffer*)buffer.Handle, 0, buffer.Size);
        }
    }

    /// <summary>The whole of <paramref name="buffer"/> as the 32-bit index buffer.</summary>
    public void SetIndexBuffer(GpuBuffer buffer)
    {
        // SAFETY: the pass and the buffer are live objects of ours.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            GpuDevice.Api.RenderPassEncoderSetIndexBuffer((RenderPassEncoder*)Handle, (Silk.NET.WebGPU.Buffer*)buffer.Handle, IndexFormat.Uint32, 0, buffer.Size);
        }
    }

    public void Draw(uint vertices, uint instances = 1)
    {
        // SAFETY: the pass is a live object of ours.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            GpuDevice.Api.RenderPassEncoderDraw((RenderPassEncoder*)Handle, vertices, instances, 0, 0);
        }
    }

    /// <summary>Indices <paramref name="first"/> onwards, <paramref name="count"/> of them, for the instances from <paramref name="firstInstance"/>.</summary>
    public void DrawIndexed(uint first, uint count, uint firstInstance, uint instances)
    {
        // SAFETY: the pass is a live object of ours.
        unsafe
        {
            ObjectDisposedException.ThrowIf(Handle == 0, this);
            GpuDevice.Api.RenderPassEncoderDrawIndexed((RenderPassEncoder*)Handle, count, instances, first, 0, firstInstance);
        }
    }

    // SAFETY: the handle is a live pass encoder: it is ended (as every pass must be before its
    // encoder finishes) and released once here.
    private protected override unsafe void Release(nint handle)
    {
        GpuDevice.Api.RenderPassEncoderEnd((RenderPassEncoder*)handle);
        GpuDevice.Api.RenderPassEncoderRelease((RenderPassEncoder*)handle);
    }
}

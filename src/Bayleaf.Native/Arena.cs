using System.Runtime.InteropServices;
using System.Text;

namespace Bayleaf.Native;

// SAFETY: the arena hands out zeroed blocks of unmanaged memory that live until it is disposed; the
// callers only write within the counts they asked for and pass the blocks to wgpu-native, which reads
// descriptors during the call and keeps no pointer to them afterwards.
internal sealed unsafe class Arena : IDisposable
{
    private readonly List<nint> blocks = [];

    /// <summary><paramref name="count"/> zeroed values of <typeparamref name="T"/>.</summary>
    public T* Alloc<T>(int count = 1)
        where T : unmanaged
    {
        var block = NativeMemory.AllocZeroed((nuint)Math.Max(count, 1), (nuint)sizeof(T));
        blocks.Add((nint)block);
        return (T*)block;
    }

    /// <summary>One value, copied into the arena.</summary>
    public T* One<T>(T value)
        where T : unmanaged
    {
        var block = Alloc<T>();
        *block = value;
        return block;
    }

    /// <summary>The text as a zero-ended UTF-8 string; null for null.</summary>
    public byte* Text(string? text)
    {
        if (text is null)
        {
            return null;
        }
        var count = Encoding.UTF8.GetByteCount(text);
        var block = Alloc<byte>(count + 1);
        Encoding.UTF8.GetBytes(text, new Span<byte>(block, count));
        return block;
    }

    public void Dispose()
    {
        foreach (var block in blocks)
        {
            NativeMemory.Free((void*)block);
        }
        blocks.Clear();
    }
}

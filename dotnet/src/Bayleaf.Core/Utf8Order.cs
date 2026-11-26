using System.Text;

namespace Bayleaf.Core;

/// <summary>
/// Orders text by its UTF-8 bytes, as the Rust engine orders map keys. It differs from ordinal
/// (UTF-16) order only between characters above U+FFFF and those from U+E000 to U+FFFF.
/// </summary>
public sealed class Utf8Order : IComparer<string>
{
    public static readonly Utf8Order Instance = new();

    private Utf8Order()
    {
    }

    public int Compare(string? x, string? y)
    {
        if (ReferenceEquals(x, y))
        {
            return 0;
        }
        if (x is null)
        {
            return -1;
        }
        if (y is null)
        {
            return 1;
        }
        var (a, b) = (x.EnumerateRunes(), y.EnumerateRunes());
        while (true)
        {
            var (moreA, moreB) = (a.MoveNext(), b.MoveNext());
            if (!moreA || !moreB)
            {
                return moreA ? 1 : moreB ? -1 : 0;
            }
            var order = a.Current.Value.CompareTo(b.Current.Value);
            if (order != 0)
            {
                return order;
            }
        }
    }

    /// <summary>The UTF-8 length of <paramref name="text"/>, as the Rust engine measures text.</summary>
    public static int ByteLength(string text) => Encoding.UTF8.GetByteCount(text);
}

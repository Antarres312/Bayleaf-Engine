using System.Globalization;
using System.Text;

namespace Bayleaf.Core;

/// <summary>Text measured and quoted as the Rust engine measures and quotes it.</summary>
public static class Text
{
    /// <summary>Length in UTF-8 bytes (the Rust engine's <c>len</c>).</summary>
    public static int Bytes(string text) => Encoding.UTF8.GetByteCount(text);

    /// <summary>Length in characters (code points; the Rust engine's <c>chars().count()</c>).</summary>
    public static int Chars(string text) => text.EnumerateRunes().Count();

    /// <summary>Whether a control character (general category Cc) is in the text.</summary>
    public static bool HasControl(string text) => text.EnumerateRunes().Any(Rune.IsControl);

    /// <summary>Whether the text holds a NUL character.</summary>
    public static bool HasNul(string text) => text.Contains('\0', StringComparison.Ordinal);

    /// <summary>The text in quotes with escapes, as the Rust engine's messages quote it (<c>{:?}</c>).</summary>
    public static string Debug(string text)
    {
        var builder = new StringBuilder("\"");
        foreach (var rune in text.EnumerateRunes())
        {
            switch (rune.Value)
            {
                case '"':
                    builder.Append("\\\"");
                    break;
                case '\\':
                    builder.Append("\\\\");
                    break;
                case '\n':
                    builder.Append("\\n");
                    break;
                case '\r':
                    builder.Append("\\r");
                    break;
                case '\t':
                    builder.Append("\\t");
                    break;
                case 0:
                    builder.Append("\\0");
                    break;
                default:
                    if (Rune.IsControl(rune))
                    {
                        builder.Append("\\u{").Append(rune.Value.ToString("x", CultureInfo.InvariantCulture)).Append('}');
                    }
                    else
                    {
                        builder.Append(rune.ToString());
                    }
                    break;
            }
        }
        return builder.Append('"').ToString();
    }

    /// <summary>Whether the text ends with <paramref name="suffix"/>, ignoring the case of ASCII letters only.</summary>
    public static bool EndsWithAsciiNoCase(string text, string suffix) =>
        text.Length >= suffix.Length && string.Equals(AsciiLower(text[^suffix.Length..]), AsciiLower(suffix), StringComparison.Ordinal);

    /// <summary>The text with ASCII letters made lowercase and everything else as it is.</summary>
    public static string AsciiLower(string text) => string.Create(text.Length, text, static (span, source) =>
    {
        for (var i = 0; i < source.Length; i++)
        {
            span[i] = source[i] is >= 'A' and <= 'Z' ? (char)(source[i] + 32) : source[i];
        }
    });

    /// <summary>A number as the Rust engine shows one in messages (<c>{}</c> of a float: 0.5, 100, -3600).</summary>
    public static string Number(double value) =>
        double.IsNaN(value) ? "NaN" : double.IsInfinity(value) ? (value > 0 ? "inf" : "-inf") : value.ToString("R", CultureInfo.InvariantCulture);

    /// <summary>A single-precision number in its own shortest digits, as the Rust engine shows one.</summary>
    public static string Number(float value) =>
        float.IsNaN(value) ? "NaN" : float.IsInfinity(value) ? (value > 0 ? "inf" : "-inf") : value.ToString("R", CultureInfo.InvariantCulture);
}

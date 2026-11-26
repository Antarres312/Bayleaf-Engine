using System.Globalization;
using System.Text;

namespace Bayleaf.Render;

/// <summary>
/// Single-precision numbers written as the Rust engine's <c>{:?}</c> writes them, so both engines give
/// the same shader text: the shortest digits that read back to the same number, in plain form with at
/// least one decimal (<c>0.5</c>, <c>100.0</c>, <c>-0.0</c>) from 10^-4 up to 10^16, and as <c>1e-5</c> or
/// <c>1.5e16</c> outside; <c>NaN</c>, <c>inf</c> and <c>-inf</c> for the others.
/// </summary>
internal static class RustFloat
{
    public static string Debug(float value)
    {
        if (float.IsNaN(value))
        {
            return "NaN";
        }
        if (float.IsInfinity(value))
        {
            return value > 0 ? "inf" : "-inf";
        }
        var sign = float.IsNegative(value) ? "-" : "";
        if (value == 0)
        {
            return sign + "0.0";
        }
        // The shortest form from .NET: digits with an optional point, and an optional exponent.
        var text = value.ToString("R", CultureInfo.InvariantCulture).TrimStart('-');
        var exponent = 0;
        var e = text.IndexOfAny(['E', 'e']);
        if (e >= 0)
        {
            exponent = int.Parse(text.AsSpan(e + 1), NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture);
            text = text[..e];
        }
        var point = text.IndexOf('.', StringComparison.Ordinal);
        var whole = point >= 0 ? text[..point] : text;
        var fraction = point >= 0 ? text[(point + 1)..] : "";
        var all = whole + fraction;
        var digits = all.TrimStart('0');
        var leadingZeros = all.Length - digits.Length;
        digits = digits.TrimEnd('0');
        // The decimal exponent of the first digit: value = d.ddd * 10^decimalExponent.
        var decimalExponent = whole.Length - 1 - leadingZeros + exponent;
        var abs = Math.Abs(value);
        var builder = new StringBuilder(sign);
        if (abs < 1e-4f || abs >= 1e16f)
        {
            builder.Append(digits[0]);
            if (digits.Length > 1)
            {
                builder.Append('.').Append(digits.AsSpan(1));
            }
            return builder.Append('e').Append(decimalExponent.ToString(CultureInfo.InvariantCulture)).ToString();
        }
        if (decimalExponent < 0)
        {
            return builder.Append("0.").Append('0', -decimalExponent - 1).Append(digits).ToString();
        }
        if (digits.Length - 1 <= decimalExponent)
        {
            return builder.Append(digits).Append('0', decimalExponent - (digits.Length - 1)).Append(".0").ToString();
        }
        return builder.Append(digits.AsSpan(0, decimalExponent + 1)).Append('.').Append(digits.AsSpan(decimalExponent + 1)).ToString();
    }
}

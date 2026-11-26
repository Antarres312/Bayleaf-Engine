using System.Globalization;
using System.Text;

namespace Bayleaf.Core.Json;

/// <summary>
/// Numbers as the Rust engine writes and reads them, so a file written by either reads and writes
/// back the same. Writing gives the shortest digits that read back to the same number, in plain
/// form while the first digit is from 10^-5 to 10^15 (10^-6 to 10^12 for single precision) and as
/// <c>1.25e+20</c> outside, with <c>.0</c> on whole numbers. Reading multiplies or divides the digits
/// by a power of ten, as the Rust engine does, rather than rounding the decimal text exactly.
/// </summary>
public static class JsonNumbers
{
    private static readonly double[] Pow10 = Enumerable.Range(0, 309).Select(e => double.Parse($"1e{e}", CultureInfo.InvariantCulture)).ToArray();

    public static string Format(double value) => Format(value.ToString("R", CultureInfo.InvariantCulture), double.IsNegative(value), value == 0, -5, 15);

    public static string Format(float value) => Format(value.ToString("R", CultureInfo.InvariantCulture), float.IsNegative(value), value == 0, -6, 12);

    private static string Format(string shortest, bool negative, bool zero, int fixedFrom, int fixedTo)
    {
        var sign = negative ? "-" : "";
        if (zero)
        {
            return sign + "0.0";
        }
        // The shortest form from .NET: digits with an optional point, and an optional exponent.
        var text = shortest.TrimStart('-');
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
        var digits = (whole + fraction).TrimStart('0');
        var leadingZeros = (whole + fraction).Length - digits.Length;
        digits = digits.TrimEnd('0');
        if (digits.Length == 0)
        {
            return sign + "0.0";
        }
        // The decimal exponent of the first digit: value = d.ddd * 10^decimalExponent.
        var decimalExponent = whole.Length - 1 - leadingZeros + exponent;
        var builder = new StringBuilder(sign);
        if (decimalExponent >= fixedFrom && decimalExponent <= fixedTo)
        {
            if (digits.Length - 1 <= decimalExponent)
            {
                builder.Append(digits).Append('0', decimalExponent - (digits.Length - 1)).Append(".0");
            }
            else if (decimalExponent >= 0)
            {
                builder.Append(digits.AsSpan(0, decimalExponent + 1)).Append('.').Append(digits.AsSpan(decimalExponent + 1));
            }
            else
            {
                builder.Append("0.").Append('0', -decimalExponent - 1).Append(digits);
            }
            return builder.ToString();
        }
        builder.Append(digits[0]);
        if (digits.Length > 1)
        {
            builder.Append('.').Append(digits.AsSpan(1));
        }
        builder.Append('e').Append(decimalExponent < 0 ? '-' : '+').Append(Math.Abs(decimalExponent).ToString(CultureInfo.InvariantCulture));
        return builder.ToString();
    }

    /// <summary>
    /// The double the Rust engine reads for <paramref name="significand"/> × 10^<paramref name="exponent"/>;
    /// null when it is too large to hold.
    /// </summary>
    internal static double? FromParts(bool positive, ulong significand, int exponent)
    {
        double f = significand;
        while (true)
        {
            var abs = exponent == int.MinValue ? int.MaxValue : Math.Abs(exponent);
            if (abs < Pow10.Length)
            {
                var pow = Pow10[abs];
                if (exponent >= 0)
                {
                    f *= pow;
                    if (double.IsInfinity(f))
                    {
                        return null;
                    }
                }
                else
                {
                    f /= pow;
                }
                break;
            }
            if (f == 0)
            {
                break;
            }
            if (exponent >= 0)
            {
                return null;
            }
            f /= 1e308;
            exponent += 308;
        }
        return positive ? f : -f;
    }
}

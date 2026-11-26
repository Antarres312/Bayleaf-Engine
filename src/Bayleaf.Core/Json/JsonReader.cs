using System.Globalization;
using System.Text;

namespace Bayleaf.Core.Json;

/// <summary>
/// Reads JSON text as the Rust engine does: the same numbers (<see cref="JsonNumbers"/>), whole
/// numbers kept whole, no comments or trailing commas, no control characters in strings, at most
/// 128 levels of nesting, nothing but white space after the value.
/// </summary>
public static class JsonReader
{
    private const int MaxDepth = 128;

    public static Result<JsonValue, string> Parse(string text)
    {
        var parser = new Parser(text);
        try
        {
            parser.SkipWhitespace();
            var value = parser.ReadValue(0);
            parser.SkipWhitespace();
            if (!parser.AtEnd)
            {
                parser.Fail("trailing characters");
            }
            return value;
        }
        catch (JsonShapeException e)
        {
            return Result.Fail(e.Message);
        }
    }

    /// <summary>Decodes UTF-8 bytes as the Rust engine reads a text file: invalid bytes and all; a byte-order mark stays.</summary>
    public static Result<string, string> DecodeUtf8(ReadOnlySpan<byte> bytes)
    {
        try
        {
            return new UTF8Encoding(false, true).GetString(bytes);
        }
        catch (DecoderFallbackException e)
        {
            return Result.Fail($"stream did not contain valid UTF-8: {e.Message}");
        }
    }

    private sealed class Parser(string text)
    {
        private int at;

        public bool AtEnd => at >= text.Length;

        public void SkipWhitespace()
        {
            while (at < text.Length && text[at] is ' ' or '\t' or '\n' or '\r')
            {
                at++;
            }
        }

        public void Fail(string message)
        {
            var line = 1;
            var column = 0;
            for (var i = 0; i < Math.Min(at, text.Length); i++)
            {
                if (text[i] == '\n')
                {
                    line++;
                    column = 0;
                }
                else
                {
                    column++;
                }
            }
            throw new JsonShapeException($"{message} at line {line} column {column}");
        }

        private char Peek() => at < text.Length ? text[at] : '\0';

        public JsonValue ReadValue(int depth)
        {
            if (at >= text.Length)
            {
                Fail("EOF while parsing a value");
            }
            switch (text[at])
            {
                case 'n':
                    Expect("null");
                    return JsonValue.Null;
                case 't':
                    Expect("true");
                    return JsonBool.True;
                case 'f':
                    Expect("false");
                    return JsonBool.False;
                case '"':
                    return new JsonString(ReadString());
                case '[':
                    return ReadArray(depth + 1);
                case '{':
                    return ReadObject(depth + 1);
                case '-':
                    at++;
                    return ReadNumber(false);
                case >= '0' and <= '9':
                    return ReadNumber(true);
                default:
                    Fail("expected value");
                    return JsonValue.Null;
            }
        }

        private void Expect(string word)
        {
            if (string.CompareOrdinal(text, at, word, 0, word.Length) != 0)
            {
                Fail("expected ident");
            }
            at += word.Length;
        }

        private JsonArray ReadArray(int depth)
        {
            if (depth > MaxDepth)
            {
                Fail("recursion limit exceeded");
            }
            at++;
            var array = new JsonArray();
            SkipWhitespace();
            if (Peek() == ']')
            {
                at++;
                return array;
            }
            while (true)
            {
                SkipWhitespace();
                array.Add(ReadValue(depth));
                SkipWhitespace();
                switch (Peek())
                {
                    case ',':
                        at++;
                        SkipWhitespace();
                        if (Peek() == ']')
                        {
                            Fail("trailing comma");
                        }
                        continue;
                    case ']':
                        at++;
                        return array;
                    default:
                        Fail(AtEnd ? "EOF while parsing a list" : "expected `,` or `]`");
                        return array;
                }
            }
        }

        private JsonObject ReadObject(int depth)
        {
            if (depth > MaxDepth)
            {
                Fail("recursion limit exceeded");
            }
            at++;
            var obj = new JsonObject();
            SkipWhitespace();
            if (Peek() == '}')
            {
                at++;
                return obj;
            }
            while (true)
            {
                SkipWhitespace();
                if (Peek() != '"')
                {
                    Fail(AtEnd ? "EOF while parsing an object" : Peek() == '}' ? "trailing comma" : "key must be a string");
                }
                var key = ReadString();
                SkipWhitespace();
                if (Peek() != ':')
                {
                    Fail(AtEnd ? "EOF while parsing an object" : "expected `:`");
                }
                at++;
                SkipWhitespace();
                obj.Add(key, ReadValue(depth));
                SkipWhitespace();
                switch (Peek())
                {
                    case ',':
                        at++;
                        continue;
                    case '}':
                        at++;
                        return obj;
                    default:
                        Fail(AtEnd ? "EOF while parsing an object" : "expected `,` or `}`");
                        return obj;
                }
            }
        }

        private string ReadString()
        {
            at++;
            var builder = new StringBuilder();
            while (true)
            {
                if (at >= text.Length)
                {
                    Fail("EOF while parsing a string");
                }
                var c = text[at++];
                switch (c)
                {
                    case '"':
                        return builder.ToString();
                    case '\\':
                        ReadEscape(builder);
                        break;
                    case < ' ':
                        at--;
                        Fail("control character (\\u0000-\\u001F) found while parsing a string");
                        break;
                    default:
                        builder.Append(c);
                        break;
                }
            }
        }

        private void ReadEscape(StringBuilder builder)
        {
            if (at >= text.Length)
            {
                Fail("EOF while parsing a string");
            }
            var c = text[at++];
            switch (c)
            {
                case '"':
                    builder.Append('"');
                    break;
                case '\\':
                    builder.Append('\\');
                    break;
                case '/':
                    builder.Append('/');
                    break;
                case 'b':
                    builder.Append('\b');
                    break;
                case 'f':
                    builder.Append('\f');
                    break;
                case 'n':
                    builder.Append('\n');
                    break;
                case 'r':
                    builder.Append('\r');
                    break;
                case 't':
                    builder.Append('\t');
                    break;
                case 'u':
                    var unit = ReadHex4();
                    if (char.IsHighSurrogate(unit))
                    {
                        if (at + 1 < text.Length && text[at] == '\\' && text[at + 1] == 'u')
                        {
                            at += 2;
                            var low = ReadHex4();
                            if (!char.IsLowSurrogate(low))
                            {
                                Fail("lone leading surrogate in hex escape");
                            }
                            builder.Append(unit).Append(low);
                        }
                        else
                        {
                            Fail("lone leading surrogate in hex escape");
                        }
                    }
                    else if (char.IsLowSurrogate(unit))
                    {
                        Fail("lone leading surrogate in hex escape");
                    }
                    else
                    {
                        builder.Append(unit);
                    }
                    break;
                default:
                    Fail("invalid escape");
                    break;
            }
        }

        private char ReadHex4()
        {
            if (at + 4 > text.Length)
            {
                at = text.Length;
                Fail("EOF while parsing a string");
            }
            if (!ushort.TryParse(text.AsSpan(at, 4), NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture, out var unit))
            {
                Fail("invalid escape");
            }
            at += 4;
            return (char)unit;
        }

        private static bool Overflows(ulong value, ulong digit) => value >= ulong.MaxValue / 10 && (value > ulong.MaxValue / 10 || digit > ulong.MaxValue % 10);

        private static bool Overflows(int value, int digit) => value >= int.MaxValue / 10 && (value > int.MaxValue / 10 || digit > int.MaxValue % 10);

        private JsonNumber ReadNumber(bool positive)
        {
            if (at >= text.Length)
            {
                Fail("EOF while parsing a value");
            }
            var first = text[at++];
            if (first == '0')
            {
                if (Peek() is >= '0' and <= '9')
                {
                    Fail("invalid number");
                }
                return AfterInteger(positive, 0);
            }
            if (first is not (>= '1' and <= '9'))
            {
                at--;
                Fail("invalid number");
            }
            var significand = (ulong)(first - '0');
            while (Peek() is >= '0' and <= '9')
            {
                var digit = (ulong)(Peek() - '0');
                if (Overflows(significand, digit))
                {
                    return JsonNumber.Of(LongInteger(positive, significand));
                }
                at++;
                significand = significand * 10 + digit;
            }
            return AfterInteger(positive, significand);
        }

        private JsonNumber AfterInteger(bool positive, ulong significand)
        {
            switch (Peek())
            {
                case '.':
                    return JsonNumber.Of(Decimal(positive, significand, 0));
                case 'e' or 'E':
                    return JsonNumber.Of(Exponent(positive, significand, 0));
            }
            if (positive)
            {
                return JsonNumber.Of(significand);
            }
            var negative = unchecked(-(long)significand);
            // A whole number below the range, and -0, become doubles.
            return negative >= 0 ? JsonNumber.Of(-(double)significand) : JsonNumber.Of(negative);
        }

        private double LongInteger(bool positive, ulong significand)
        {
            var exponent = 0;
            while (true)
            {
                switch (Peek())
                {
                    case >= '0' and <= '9':
                        at++;
                        exponent++;
                        continue;
                    case '.':
                        return Decimal(positive, significand, exponent);
                    case 'e' or 'E':
                        return Exponent(positive, significand, exponent);
                    default:
                        return FromParts(positive, significand, exponent);
                }
            }
        }

        private double Decimal(bool positive, ulong significand, int exponentBeforePoint)
        {
            at++;
            var exponentAfterPoint = 0;
            while (Peek() is >= '0' and <= '9')
            {
                var digit = (ulong)(Peek() - '0');
                if (Overflows(significand, digit))
                {
                    // Further digits are ignored.
                    while (Peek() is >= '0' and <= '9')
                    {
                        at++;
                    }
                    return Peek() is 'e' or 'E'
                        ? Exponent(positive, significand, exponentBeforePoint + exponentAfterPoint)
                        : FromParts(positive, significand, exponentBeforePoint + exponentAfterPoint);
                }
                at++;
                significand = significand * 10 + digit;
                exponentAfterPoint--;
            }
            if (exponentAfterPoint == 0)
            {
                Fail(AtEnd ? "EOF while parsing a value" : "invalid number");
            }
            var exponent = exponentBeforePoint + exponentAfterPoint;
            return Peek() is 'e' or 'E' ? Exponent(positive, significand, exponent) : FromParts(positive, significand, exponent);
        }

        private double Exponent(bool positive, ulong significand, int startingExponent)
        {
            at++;
            var positiveExponent = true;
            if (Peek() == '+')
            {
                at++;
            }
            else if (Peek() == '-')
            {
                at++;
                positiveExponent = false;
            }
            if (at >= text.Length)
            {
                Fail("EOF while parsing a value");
            }
            var next = text[at++];
            if (next is not (>= '0' and <= '9'))
            {
                Fail("invalid number");
            }
            var exponent = next - '0';
            while (Peek() is >= '0' and <= '9')
            {
                at++;
                var digit = text[at - 1] - '0';
                if (Overflows(exponent, digit))
                {
                    if (significand != 0 && positiveExponent)
                    {
                        Fail("number out of range");
                    }
                    while (Peek() is >= '0' and <= '9')
                    {
                        at++;
                    }
                    return positive ? 0.0 : -0.0;
                }
                exponent = exponent * 10 + digit;
            }
            var final = positiveExponent ? SaturatingAdd(startingExponent, exponent) : SaturatingAdd(startingExponent, -exponent);
            return FromParts(positive, significand, final);
        }

        private static int SaturatingAdd(int a, int b) => (int)Math.Clamp((long)a + b, int.MinValue, int.MaxValue);

        private double FromParts(bool positive, ulong significand, int exponent)
        {
            var value = JsonNumbers.FromParts(positive, significand, exponent);
            if (value is null)
            {
                Fail("number out of range");
            }
            return value ?? 0;
        }
    }
}

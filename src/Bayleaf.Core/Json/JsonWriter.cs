using System.Globalization;
using System.Text;

namespace Bayleaf.Core.Json;

/// <summary>
/// Writes JSON as the Rust engine does: compact, or pretty with two spaces a level and <c>"key": value</c>;
/// strings escape only <c>"</c>, <c>\</c> and control characters (as <c>\n</c>, <c>\t</c>, ... or
/// <c>\u001f</c>), never other characters.
/// </summary>
public static class JsonWriter
{
    public static string Compact(JsonValue value)
    {
        var builder = new StringBuilder();
        Write(builder, value, null, 0);
        return builder.ToString();
    }

    public static string Pretty(JsonValue value)
    {
        var builder = new StringBuilder();
        Write(builder, value, "  ", 0);
        return builder.ToString();
    }

    private static void NewLine(StringBuilder builder, string? indent, int depth)
    {
        if (indent is null)
        {
            return;
        }
        builder.Append('\n');
        for (var i = 0; i < depth; i++)
        {
            builder.Append(indent);
        }
    }

    private static void Write(StringBuilder builder, JsonValue value, string? indent, int depth)
    {
        switch (value)
        {
            case JsonNull:
                builder.Append("null");
                break;
            case JsonBool b:
                builder.Append(b.Value ? "true" : "false");
                break;
            case JsonNumber n:
                builder.Append(n.Format());
                break;
            case JsonString s:
                WriteString(builder, s.Value);
                break;
            case JsonArray a:
                if (a.Count == 0)
                {
                    builder.Append("[]");
                    break;
                }
                builder.Append('[');
                for (var i = 0; i < a.Count; i++)
                {
                    if (i > 0)
                    {
                        builder.Append(',');
                    }
                    NewLine(builder, indent, depth + 1);
                    Write(builder, a[i], indent, depth + 1);
                }
                NewLine(builder, indent, depth);
                builder.Append(']');
                break;
            case JsonObject o:
                if (o.Count == 0)
                {
                    builder.Append("{}");
                    break;
                }
                builder.Append('{');
                var first = true;
                foreach (var (key, item) in o.Entries)
                {
                    if (!first)
                    {
                        builder.Append(',');
                    }
                    first = false;
                    NewLine(builder, indent, depth + 1);
                    WriteString(builder, key);
                    builder.Append(indent is null ? ":" : ": ");
                    Write(builder, item, indent, depth + 1);
                }
                NewLine(builder, indent, depth);
                builder.Append('}');
                break;
        }
    }

    public static void WriteString(StringBuilder builder, string text)
    {
        builder.Append('"');
        foreach (var c in text)
        {
            switch (c)
            {
                case '"':
                    builder.Append("\\\"");
                    break;
                case '\\':
                    builder.Append("\\\\");
                    break;
                case '\b':
                    builder.Append("\\b");
                    break;
                case '\f':
                    builder.Append("\\f");
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
                case < ' ':
                    builder.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
                    break;
                default:
                    builder.Append(c);
                    break;
            }
        }
        builder.Append('"');
    }
}

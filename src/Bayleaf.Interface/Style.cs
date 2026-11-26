using System.Globalization;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.GameInterface;

/// <summary>
/// A style file (<c>.bstyle</c>). A rule picks elements by one thing only, their kind, a class or
/// their name, and sets properties. The cascade is fixed: the kind's defaults, then kind rules,
/// class rules and name rules (each in file order, later files after earlier ones), then the
/// element's own style. Lengths always carry their unit: <c>px</c> (points, scaled with the
/// interface), <c>%</c> of the parent's inner size, or <c>auto</c>.
/// </summary>
public sealed class StyleSheet
{
    public const string StyleFormat = "bayleaf.style";
    public const string StyleExtension = "bstyle";

    /// <summary>Most rules in one style file.</summary>
    public const int MaxRules = 1024;

    /// <summary>Every style property, with a value to start from when one is added.</summary>
    public static readonly IReadOnlyList<(string Property, string Value)> Properties =
    [
        ("direction", "row"),
        ("gap", "8px"),
        ("padding", "8px"),
        ("width", "100px"),
        ("height", "40px"),
        ("minWidth", "0px"),
        ("minHeight", "0px"),
        ("maxWidth", "400px"),
        ("maxHeight", "400px"),
        ("grow", "1"),
        ("shrink", "1"),
        ("alignItems", "center"),
        ("alignSelf", "center"),
        ("justify", "center"),
        ("background", "#202020c0"),
        ("color", "#ffffff"),
        ("borderColor", "#808080"),
        ("borderWidth", "1px"),
        ("radius", "6px"),
        ("fontSize", "18px"),
        ("textAlign", "center"),
        ("visible", "true"),
        ("opacity", "1"),
        ("accent", "#5fb3f9"),
        ("pointer", "pass"),
    ];

    public StyleSheet()
        : this(StyleFormat, Layout.CurrentVersion, [])
    {
    }

    public StyleSheet(string format, uint version, List<Rule> rules)
    {
        Format = format;
        Version = version;
        Rules = rules;
    }

    public string Format { get; set; }

    public uint Version { get; set; }

    public List<Rule> Rules { get; }

    public static Result<StyleSheet, string> FromText(string text)
    {
        var read = JsonReader.Parse(text).TryGet(out var value, out var parseError)
            ? JsonRead.Try(() =>
            {
                var f = JsonFields.Of(value, "struct StyleSheet", true, "format", "version", "rules");
                return new StyleSheet(f.Required("format", JsonRead.Str), f.Required("version", JsonRead.U32), f.Required("rules", v => JsonRead.List(v, Rule.Read)));
            })
            : Result.Fail(parseError);
        if (!read.TryGet(out var sheet, out var error))
        {
            return Result.Fail($"not a style file: {error}");
        }
        var valid = sheet.Validate();
        return valid.IsOk ? sheet : valid.AsFailure();
    }

    /// <summary>The file's text.</summary>
    public string ToText() =>
        new JsonObject { { "format", J.S(Format) }, { "version", J.U(Version) }, { "rules", new JsonArray(Rules.Select(r => r.Write())) } }.ToPretty() + "\n";

    public static Result<StyleSheet, string> Load(string path) => FileText.Load(path).TryGet(out var text, out var error) ? FromText(text).MapError(e => $"{path}: {e}") : Result.Fail(error);

    public Result<Unit, string> Validate()
    {
        if (Format != StyleFormat)
        {
            return Result.Fail($"the format is {J.Debug(Format)}, not {J.Debug(StyleFormat)}");
        }
        if (Version > Layout.CurrentVersion)
        {
            return Result.Fail(JsonRead.Invariant($"style version {Version} is newer than this engine reads ({Layout.CurrentVersion})"));
        }
        if (Rules.Count > MaxRules)
        {
            return Result.Fail(JsonRead.Invariant($"a style file holds at most {MaxRules} rules"));
        }
        for (var i = 0; i < Rules.Count; i++)
        {
            var rule = Rules[i];
            var picks = (rule.Kind is null ? 0 : 1) + (rule.Class is null ? 0 : 1) + (rule.Name is null ? 0 : 1);
            if (picks != 1)
            {
                return Result.Fail(JsonRead.Invariant($"rule {i + 1}: picks elements by exactly one of kind, class or name"));
            }
            foreach (var (property, value) in rule.Set)
            {
                var valid = Style.CheckProperty(property, value);
                if (valid.IsError)
                {
                    return Result.Fail(JsonRead.Invariant($"rule {i + 1}: {valid.Error}"));
                }
            }
        }
        return J.Ok;
    }
}

/// <summary>Reads interface files as the Rust engine does: at most 4 MiB.</summary>
internal static class FileText
{
    public static Result<string, string> Load(string path)
    {
        var size = Bayleaf.Core.Files.Length(path);
        if (size.IsError)
        {
            return Result.Fail($"cannot read {path}: {size.Error}");
        }
        if (size.Value > 4 << 20)
        {
            return Result.Fail($"{path} is larger than 4 MiB");
        }
        var text = Bayleaf.Core.Files.ReadText(path);
        return text.IsOk ? text : Result.Fail($"cannot read {path}: {text.Error}");
    }
}

/// <summary>Sets properties on every element of a kind, with a class, or of a name: exactly one of them.</summary>
public sealed record Rule(Kind? Kind, string? Class, string? Name, SortedDictionary<string, string> Set)
{
    internal static Rule Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Rule", true, "kind", "class", "name", "set");
        return new(f.NullableValue("kind", Kinds.Read), f.Nullable("class", JsonRead.Str), f.Nullable("name", JsonRead.Str), f.Required("set", J.ReadMap));
    }

    internal JsonValue Write()
    {
        var o = new JsonObject();
        if (Kind is { } kind)
        {
            o.Add("kind", J.S(kind.Name()));
        }
        if (Class is not null)
        {
            o.Add("class", J.S(Class));
        }
        if (Name is not null)
        {
            o.Add("name", J.S(Name));
        }
        o.Add("set", J.Map(Set));
        return o;
    }
}

/// <summary>A length: <c>auto</c>, points or a share of the parent's inner size along the same axis.</summary>
public readonly record struct Length(LengthUnit Unit, float Value)
{
    public static readonly Length Auto = new(LengthUnit.Auto, 0f);

    public static Length Px(float value) => new(LengthUnit.Px, value);

    public static Length Percent(float value) => new(LengthUnit.Percent, value);
}

public enum LengthUnit
{
    Auto,
    Px,
    Percent,
}

public enum Direction
{
    Row,
    Column,
}

/// <summary>Where children sit across the box's direction.</summary>
public enum Align
{
    Start,
    Center,
    End,
    Stretch,
}

/// <summary>Where children sit along the box's direction, when they leave room.</summary>
public enum Justify
{
    Start,
    Center,
    End,
    SpaceBetween,
}

public enum TextAlign
{
    Start,
    Center,
    End,
}

/// <summary>sRGB colour with alpha, as written (<c>#rrggbb</c> or <c>#rrggbbaa</c>).</summary>
public readonly record struct Color(byte R, byte G, byte B, byte A)
{
    public static readonly Color None = new(0, 0, 0, 0);
}

/// <summary>Top, right, bottom and left.</summary>
public readonly record struct Sides(float Top, float Right, float Bottom, float Left)
{
    public static Sides All(float value) => new(value, value, value, value);

    public Sides Times(float scale) => new(Top * scale, Right * scale, Bottom * scale, Left * scale);
}

/// <summary>An element's look, worked out.</summary>
public sealed record Style
{
    public Direction Direction { get; set; }

    public float Gap { get; set; }

    public Sides Padding { get; set; }

    public Length Width { get; set; }

    public Length Height { get; set; }

    public Length MinWidth { get; set; }

    public Length MinHeight { get; set; }

    public Length MaxWidth { get; set; }

    public Length MaxHeight { get; set; }

    /// <summary>How much of the room left over it takes, against its siblings.</summary>
    public float Grow { get; set; }

    /// <summary>
    /// How much it gives up when there is too little room: text gives room (it wraps); boxes,
    /// controls and images keep their size unless their style says otherwise.
    /// </summary>
    public float Shrink { get; set; }

    public Align AlignItems { get; set; }

    /// <summary>Its own place across its parent, instead of the parent's <c>alignItems</c>.</summary>
    public Align? AlignSelf { get; set; }

    public Justify Justify { get; set; }

    public Color Background { get; set; }

    public Color Color { get; set; }

    public Color BorderColor { get; set; }

    public float BorderWidth { get; set; }

    public float Radius { get; set; }

    public float FontSize { get; set; }

    public TextAlign TextAlign { get; set; }

    public bool Visible { get; set; }

    public float Opacity { get; set; }

    /// <summary>A control's own colour: a toggle's mark, a slider's fill, a progress bar's fill, the focus ring.</summary>
    public Color Accent { get; set; }

    /// <summary>
    /// Whether it takes the pointer, so a click on it does not also reach the game; null for the
    /// default: controls, elements with On Click and boxes with a background take it.
    /// </summary>
    public bool? TakesPointer { get; set; }

    private static readonly Color White = new(255, 255, 255, 255);
    private static readonly Color AccentColor = new(95, 179, 249, 255);

    /// <summary>A kind's look before any rule.</summary>
    public static Style Defaults(Kind kind)
    {
        var style = new Style
        {
            Direction = Direction.Column,
            Gap = 0f,
            Padding = Sides.All(0f),
            Width = Length.Auto,
            Height = Length.Auto,
            MinWidth = Length.Auto,
            MinHeight = Length.Auto,
            MaxWidth = Length.Auto,
            MaxHeight = Length.Auto,
            Grow = 0f,
            Shrink = kind == Kind.Text ? 1f : 0f,
            AlignItems = Align.Stretch,
            AlignSelf = null,
            Justify = Justify.Start,
            Background = Color.None,
            Color = White,
            BorderColor = Color.None,
            BorderWidth = 0f,
            Radius = 0f,
            FontSize = 18f,
            TextAlign = TextAlign.Start,
            Visible = true,
            Opacity = 1f,
            Accent = AccentColor,
            TakesPointer = null,
        };
        switch (kind)
        {
            case Kind.Row:
                style.Direction = Direction.Row;
                break;
            case Kind.List:
                style.Gap = 4f;
                break;
            case Kind.Button:
                style.Padding = new(8f, 16f, 8f, 16f);
                style.Background = new(45, 52, 64, 230);
                style.Radius = 6f;
                style.TextAlign = TextAlign.Center;
                break;
            case Kind.Toggle:
                style.Gap = 8f;
                break;
            case Kind.Slider:
                style.Width = Length.Px(200f);
                style.Height = Length.Px(24f);
                style.Background = new(40, 40, 40, 200);
                style.Radius = 12f;
                break;
            case Kind.TextField:
                style.Width = Length.Px(220f);
                style.Padding = new(6f, 8f, 6f, 8f);
                style.Background = new(20, 20, 20, 220);
                style.BorderColor = new(120, 120, 120, 255);
                style.BorderWidth = 1f;
                style.Radius = 4f;
                break;
            case Kind.Progress:
                style.Width = Length.Px(200f);
                style.Height = Length.Px(16f);
                style.Background = new(40, 40, 40, 200);
                style.Radius = 8f;
                break;
            default:
                break;
        }
        return style;
    }

    /// <summary>
    /// The element's look: its kind's defaults, then the rules of <paramref name="sheets"/> in the
    /// fixed order, then its own style. Values were checked when the files were read; one that does
    /// not read here is left out.
    /// </summary>
    public static Style Of(Element element, IReadOnlyList<StyleSheet> sheets)
    {
        var style = Defaults(element.Kind);
        var rules = sheets.SelectMany(s => s.Rules).ToList();
        foreach (var rule in rules.Where(r => r.Kind == element.Kind))
        {
            style.ApplyAll(rule.Set);
        }
        foreach (var rule in rules.Where(r => r.Class is not null && element.Classes.Contains(r.Class)))
        {
            style.ApplyAll(rule.Set);
        }
        foreach (var rule in rules.Where(r => r.Name is { Length: > 0 } && r.Name == element.Name))
        {
            style.ApplyAll(rule.Set);
        }
        style.ApplyAll(element.Style);
        return style;
    }

    private void ApplyAll(SortedDictionary<string, string> set)
    {
        foreach (var (property, value) in set)
        {
            _ = Apply(property, value);
        }
    }

    private static Result<Unit, string> Set<T>(Result<T, string> parsed, Action<T> assign)
    {
        if (!parsed.TryGet(out var value, out var error))
        {
            return Result.Fail(error);
        }
        assign(value);
        return J.Ok;
    }

    /// <summary>Sets one property from its written value.</summary>
    public Result<Unit, string> Apply(string property, string value)
    {
        value = value.Trim();
        return property switch
        {
            "direction" => Set(Keyword(value, ("row", Direction.Row), ("column", Direction.Column)), v => Direction = v),
            "gap" => Set(PxOf(value), v => Gap = v),
            "padding" => Set(PaddingOf(value), v => Padding = v),
            "width" => Set(LengthOf(value), v => Width = v),
            "height" => Set(LengthOf(value), v => Height = v),
            "minWidth" => Set(LengthOf(value), v => MinWidth = v),
            "minHeight" => Set(LengthOf(value), v => MinHeight = v),
            "maxWidth" => Set(LengthOf(value), v => MaxWidth = v),
            "maxHeight" => Set(LengthOf(value), v => MaxHeight = v),
            "grow" => Set(Factor(value), v => Grow = v),
            "shrink" => Set(Factor(value), v => Shrink = v),
            "alignItems" => Set(AlignOf(value), v => AlignItems = v),
            "alignSelf" => Set(AlignOf(value), v => AlignSelf = v),
            "justify" => Set(Keyword(value, ("start", Justify.Start), ("center", Justify.Center), ("end", Justify.End), ("spaceBetween", Justify.SpaceBetween)), v => Justify = v),
            "background" => Set(ColorOf(value), v => Background = v),
            "color" => Set(ColorOf(value), v => Color = v),
            "borderColor" => Set(ColorOf(value), v => BorderColor = v),
            "borderWidth" => Set(PxOf(value), v => BorderWidth = v),
            "radius" => Set(PxOf(value), v => Radius = v),
            "fontSize" => Set(FontSizeOf(value), v => FontSize = v),
            "textAlign" => Set(Keyword(value, ("start", TextAlign.Start), ("center", TextAlign.Center), ("end", TextAlign.End)), v => TextAlign = v),
            "visible" => Set(Keyword(value, ("true", true), ("false", false)), v => Visible = v),
            "opacity" => Set(OpacityOf(value), v => Opacity = v),
            "accent" => Set(ColorOf(value), v => Accent = v),
            "pointer" => Set(Keyword(value, ("block", true), ("pass", false)), v => TakesPointer = v),
            _ => Result.Fail($"{J.Debug(property)} is not a style property; see the list in docs/interface.md"),
        };
    }

    /// <summary>Whether <paramref name="value"/> is a valid value of <paramref name="property"/>.</summary>
    internal static Result<Unit, string> CheckProperty(string property, string value)
    {
        if (Text.Bytes(value) > 128)
        {
            return Result.Fail($"{property}: a value is at most 128 characters");
        }
        return Defaults(Kind.Panel).Apply(property, value)
            .MapError(e => e.StartsWith(property, StringComparison.Ordinal) || e.StartsWith('"') ? e : $"{property}: {e}");
    }

    private static Result<T, string> Keyword<T>(string value, params (string Word, T Value)[] words)
    {
        foreach (var (word, v) in words)
        {
            if (word == value)
            {
                return v;
            }
        }
        return Result.Fail($"{J.Debug(value)} is not one of {string.Join(", ", words.Select(w => w.Word))}");
    }

    private static float? Number(string text) => J.ParseF32(text) is { } v && float.IsFinite(v) ? v : null;

    /// <summary>A length in points: <c>12px</c>.</summary>
    private static Result<float, string> PxOf(string value)
    {
        if (!value.EndsWith("px", StringComparison.Ordinal) || Number(value[..^2]) is not { } n)
        {
            return Result.Fail($"{J.Debug(value)} is a length in points with its unit, like 12px");
        }
        return n is >= 0f and <= 10_000f ? n : Result.Fail($"{J.Debug(value)} is from 0px to 10000px");
    }

    /// <summary><c>auto</c>, <c>12px</c> or <c>50%</c>.</summary>
    private static Result<Length, string> LengthOf(string value)
    {
        if (value == "auto")
        {
            return Length.Auto;
        }
        if (value.EndsWith('%') && Number(value[..^1]) is { } n)
        {
            return n is >= 0f and <= 1000f ? Length.Percent(n) : Result.Fail($"{J.Debug(value)} is from 0% to 1000%");
        }
        return PxOf(value).TryGet(out var px, out _) ? Length.Px(px) : Result.Fail($"{J.Debug(value)} is auto, a length like 12px, or a share like 50%");
    }

    /// <summary>One, two (top and bottom, left and right) or four (top, right, bottom, left) lengths in points.</summary>
    private static Result<Sides, string> PaddingOf(string value)
    {
        var parts = new List<float>();
        foreach (var part in value.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries))
        {
            if (!PxOf(part).TryGet(out var px, out var error))
            {
                return Result.Fail(error);
            }
            parts.Add(px);
        }
        return parts switch
        {
            [var all] => Sides.All(all),
            [var vertical, var horizontal] => new Sides(vertical, horizontal, vertical, horizontal),
            [var top, var right, var bottom, var left] => new Sides(top, right, bottom, left),
            _ => Result.Fail($"{J.Debug(value)} is one, two or four lengths, like 8px 16px"),
        };
    }

    /// <summary>A plain number 0 or more (a share, not a length).</summary>
    private static Result<float, string> Factor(string value) =>
        Number(value) is { } n && n is >= 0f and <= 1000f ? n : Result.Fail($"{J.Debug(value)} is a number from 0 to 1000");

    private static Result<float, string> FontSizeOf(string value)
    {
        if (!PxOf(value).TryGet(out var size, out var error))
        {
            return Result.Fail(error);
        }
        return size is >= 4f and <= 400f ? size : Result.Fail("fontSize is between 4px and 400px");
    }

    private static Result<float, string> OpacityOf(string value)
    {
        if (!Factor(value).TryGet(out var opacity, out var error))
        {
            return Result.Fail(error);
        }
        return opacity > 1f ? Result.Fail("opacity is from 0 to 1") : opacity;
    }

    private static Result<Align, string> AlignOf(string value) =>
        Keyword(value, ("start", Align.Start), ("center", Align.Center), ("end", Align.End), ("stretch", Align.Stretch));

    /// <summary><c>#rrggbb</c> or <c>#rrggbbaa</c>.</summary>
    private static Result<Color, string> ColorOf(string value)
    {
        var hex = value.StartsWith('#') ? value[1..] : null;
        if (hex is null || hex.Length is not (6 or 8) || !hex.All(char.IsAsciiHexDigit))
        {
            return Result.Fail($"{J.Debug(value)} is a colour like #ff8800 or #ff880080");
        }
        byte At(int i) => byte.Parse(hex.AsSpan(i, 2), NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture);
        return new Color(At(0), At(2), At(4), hex.Length == 8 ? At(6) : (byte)255);
    }
}

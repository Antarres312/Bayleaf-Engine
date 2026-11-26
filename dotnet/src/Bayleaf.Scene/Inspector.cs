using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;

namespace Bayleaf.Scene;

/// <summary>Shown only while a property has a given value.</summary>
public sealed record Condition(string Property, PropertyValue EqualsValue)
{
    /// <summary>Whether <paramref name="value"/> (the property's current value) meets the condition.</summary>
    public bool Holds(PropertyValue value) => (EqualsValue, value) switch
    {
        (PropertyValue.Number a, PropertyValue.Number b) => Math.Abs(a.Value - b.Value) < 1e-9,
        var (a, b) => a.Equals(b),
    };
}

/// <summary>How one declared property shows.</summary>
public sealed record PropertyLayout
{
    public string Name { get; init; } = "";

    /// <summary>Shown instead of the name.</summary>
    public string? Label { get; init; }

    /// <summary>Said on hover and to screen readers.</summary>
    public string Hint { get; init; } = "";

    /// <summary>The lowest and highest value a number may be set to.</summary>
    public (double Low, double High)? Range { get; init; }

    /// <summary>Shown as a slider across the range.</summary>
    public bool Slider { get; init; }

    /// <summary>Shown, not changed here.</summary>
    public bool ReadOnly { get; init; }

    /// <summary>A whole number chosen by its place in this list (0 first), or a text chosen from it.</summary>
    public IReadOnlyList<string> Choices { get; init; } = [];

    public Condition? ShowIf { get; init; }
}

/// <summary>
/// One item of a script's Inspector layout, as the script declares it: groups and foldouts, labels
/// and hints, ranges and sliders, lists of choices, read-only values, buttons that send the object a
/// message, notes, and items shown only while another property has a given value. It is data: no
/// script code runs in the editor.
/// </summary>
public abstract record InspectorItem
{
    private InspectorItem()
    {
    }

    public sealed record Row(PropertyLayout Layout) : InspectorItem;

    /// <summary>A titled group of items; closed groups start folded.</summary>
    public sealed record Group(string Title, bool Closed, IReadOnlyList<InspectorItem> Items, Condition? Shown) : InspectorItem;

    /// <summary>Sends the object the message (value 0) while the game plays.</summary>
    public sealed record Button(string Label, string Message, string Hint, Condition? Shown) : InspectorItem;

    /// <summary>Text shown in the card; a warning is marked as one.</summary>
    public sealed record Note(string Text, bool Warning, Condition? Shown) : InspectorItem;

    public Condition? ShowIf => this switch
    {
        Row p => p.Layout.ShowIf,
        Group g => g.Shown,
        Button b => b.Shown,
        Note n => n.Shown,
        _ => null,
    };

    /// <summary>Most items in a layout, groups' items included.</summary>
    public const int MaxItems = 256;

    /// <summary>Deepest nesting of groups.</summary>
    public const int MaxDepth = 4;

    /// <summary>Longest label, group title, button label or message name, in characters.</summary>
    public const int MaxLabel = 64;

    /// <summary>Longest hint or note, in characters.</summary>
    public const int MaxText = 512;

    /// <summary>Most choices of one property.</summary>
    public const int MaxChoices = 64;

    /// <summary>
    /// Reads and checks a layout against the script's declared properties: each item well formed, each
    /// property it names declared (and placed once), options that fit the property's type, and limits
    /// on size and nesting.
    /// </summary>
    public static Result<IReadOnlyList<InspectorItem>, string> Parse(JsonValue value, IReadOnlyList<DeclaredProperty> declared)
    {
        if (value is not JsonArray list)
        {
            return Result.Fail("inspector must be a list of items");
        }
        var checker = new Checker(declared);
        try
        {
            return checker.Items(list, 1);
        }
        catch (JsonShapeException e)
        {
            return Result.Fail(e.Message);
        }
    }

    private sealed class Checker(IReadOnlyList<DeclaredProperty> declared)
    {
        private readonly List<string> placed = [];
        private int count;

        private static readonly string[] Known = ["property", "group", "button", "note", "label", "hint", "range", "slider", "readOnly", "choices", "showIf", "closed", "items", "message", "warning"];

        private static JsonShapeException Problem(string message) => new(message);

        private static void Short(string what, string text, int limit)
        {
            var n = Core.Text.Chars(text);
            if (n == 0 || n > limit || Core.Text.HasControl(text))
            {
                throw Problem($"{what} {Core.Text.Debug(text)} needs 1 to {limit} characters and no control characters");
            }
        }

        public List<InspectorItem> Items(JsonArray list, int depth)
        {
            if (depth > MaxDepth)
            {
                throw Problem($"groups nest at most {MaxDepth} deep");
            }
            return list.Select(v => Item(v, depth)).ToList();
        }

        private DeclaredProperty Declared(string name) =>
            declared.FirstOrDefault(p => p.Name == name) ?? throw Problem($"the layout names {Core.Text.Debug(name)}, which the script does not declare");

        private Condition? ConditionOf(JsonValue? showIf)
        {
            if (showIf is null || showIf.IsNull)
            {
                return null;
            }
            var pair = JsonRead.Fixed(showIf, 2, v => v);
            var property = JsonRead.Str(pair[0]);
            var equals = pair[1];
            var property_ = Declared(property);
            var read = JsonRead.Try(() => PropertyValue.Read(equals));
            if (read.IsOk && (read.Value.Fits(property_.Type) || (property_.Type == PropertyType.SceneObject && read.Value is PropertyValue.Text)))
            {
                return new Condition(property, read.Value);
            }
            throw Problem($"showIf {Core.Text.Debug(property)}: {equals} is not a {property_.Type.Label()}");
        }

        private InspectorItem Item(JsonValue value, int depth)
        {
            count++;
            if (count > MaxItems)
            {
                throw Problem($"a layout has at most {MaxItems} items");
            }
            JsonFields fields;
            try
            {
                fields = JsonFields.Of(value, "struct Raw", true, Known);
            }
            catch (JsonShapeException e)
            {
                throw Problem($"item {value}: {e.Message}");
            }
            T? Opt<T>(string name, Func<JsonValue, T> read)
                where T : class
            {
                try
                {
                    return fields.Nullable(name, read);
                }
                catch (JsonShapeException e)
                {
                    throw Problem($"item {value}: {e.Message}");
                }
            }
            T? OptValue<T>(string name, Func<JsonValue, T> read)
                where T : struct
            {
                try
                {
                    return fields.NullableValue(name, read);
                }
                catch (JsonShapeException e)
                {
                    throw Problem($"item {value}: {e.Message}");
                }
            }
            var raw = new
            {
                Property = Opt("property", JsonRead.Str),
                Group = Opt("group", JsonRead.Str),
                Button = Opt("button", JsonRead.Str),
                Note = Opt("note", JsonRead.Str),
                Label = Opt("label", JsonRead.Str),
                Hint = Opt("hint", JsonRead.Str),
                Range = Opt("range", v => JsonRead.Fixed(v, 2, JsonRead.F64)),
                Slider = OptValue("slider", JsonRead.Bool),
                ReadOnly = OptValue("readOnly", JsonRead.Bool),
                Choices = Opt("choices", v => JsonRead.List(v, JsonRead.Str)),
                ShowIf = Opt("showIf", v => JsonRead.Fixed(v, 2, x => x)) is { } pair ? new JsonArray(pair) : null,
                Closed = OptValue("closed", JsonRead.Bool),
                Items = Opt("items", v => JsonRead.List(v, x => x)),
                Message = Opt("message", JsonRead.Str),
                Warning = OptValue("warning", JsonRead.Bool),
            };
            var kinds = new[] { raw.Property is not null, raw.Group is not null, raw.Button is not null, raw.Note is not null };
            if (kinds.Count(k => k) != 1)
            {
                throw Problem($"item {value}: each item is exactly one of property, group, button and note");
            }
            void Only(params string[] allowed)
            {
                var given = new (string Name, bool Set)[]
                {
                    ("label", raw.Label is not null),
                    ("hint", raw.Hint is not null),
                    ("range", raw.Range is not null),
                    ("slider", raw.Slider is not null),
                    ("readOnly", raw.ReadOnly is not null),
                    ("choices", raw.Choices is not null),
                    ("closed", raw.Closed is not null),
                    ("items", raw.Items is not null),
                    ("message", raw.Message is not null),
                    ("warning", raw.Warning is not null),
                };
                foreach (var (name, set) in given)
                {
                    if (set && !allowed.Contains(name))
                    {
                        throw Problem($"item {value}: {name} does not go with this kind of item");
                    }
                }
            }
            if (raw.Hint is not null)
            {
                Short("hint", raw.Hint, MaxText);
            }
            var showIf = ConditionOf(raw.ShowIf);
            var hint = raw.Hint ?? "";
            if (raw.Property is { } name)
            {
                Only("label", "hint", "range", "slider", "readOnly", "choices");
                return new Row(PropertyOf(name, raw.Label, raw.Range, raw.Slider, raw.ReadOnly, raw.Choices, hint, showIf));
            }
            if (raw.Group is { } title)
            {
                Only("closed", "items");
                Short("group", title, MaxLabel);
                var items = Items(new JsonArray(raw.Items ?? []), depth + 1);
                return new Group(title, raw.Closed ?? false, items, showIf);
            }
            if (raw.Button is { } label)
            {
                Only("hint", "message");
                Short("button", label, MaxLabel);
                var message = raw.Message ?? throw Problem($"button {Core.Text.Debug(label)} needs the message it sends");
                Short("message", message, MaxLabel);
                return new Button(label, message, hint, showIf);
            }
            var text = raw.Note ?? "";
            Only("warning");
            Short("note", text, MaxText);
            return new Note(text, raw.Warning ?? false, showIf);
        }

        private PropertyLayout PropertyOf(string name, string? label, double[]? range, bool? slider, bool? readOnly, List<string>? choices, string hint, Condition? showIf)
        {
            var type = Declared(name).Type;
            if (placed.Contains(name))
            {
                throw Problem($"the layout places {Core.Text.Debug(name)} twice");
            }
            placed.Add(name);
            if (label is not null)
            {
                Short("label", label, MaxLabel);
            }
            (double, double)? bounds = null;
            if (range is [var low, var high])
            {
                if (type is not (PropertyType.Number or PropertyType.WholeNumber))
                {
                    throw Problem($"{Core.Text.Debug(name)}: a range is for numbers, not a {type.Label()} ({Core.Text.Number(low)} to {Core.Text.Number(high)})");
                }
                if (!(double.IsFinite(low) && double.IsFinite(high) && low < high))
                {
                    throw Problem($"{Core.Text.Debug(name)}: the range needs two finite numbers, the lower first");
                }
                bounds = (low, high);
            }
            var isSlider = slider ?? false;
            if (isSlider && bounds is null)
            {
                throw Problem($"{Core.Text.Debug(name)}: a slider needs a range");
            }
            var list = choices ?? [];
            if (list.Count > 0)
            {
                if (type is not (PropertyType.WholeNumber or PropertyType.Text))
                {
                    throw Problem($"{Core.Text.Debug(name)}: choices are for a whole number (by place) or a text, not a {type.Label()}");
                }
                if (list.Count > MaxChoices)
                {
                    throw Problem($"{Core.Text.Debug(name)}: at most {MaxChoices} choices");
                }
                foreach (var choice in list)
                {
                    Short("choice", choice, MaxLabel);
                }
            }
            return new PropertyLayout { Name = name, Label = label, Hint = hint, Range = bounds, Slider = isSlider, ReadOnly = readOnly ?? false, Choices = list, ShowIf = showIf };
        }
    }
}

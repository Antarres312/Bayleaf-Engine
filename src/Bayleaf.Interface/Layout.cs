using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.GameInterface;

/// <summary>What an element is.</summary>
public enum Kind
{
    /// <summary>A box holding others, laid out as a column unless its style says otherwise.</summary>
    Panel,

    /// <summary>Children side by side.</summary>
    Row,

    /// <summary>Children one under another.</summary>
    Column,
    Text,
    Image,
    Button,

    /// <summary>On or off (<c>value</c> 1 or 0), with its text beside it.</summary>
    Toggle,

    /// <summary>A number from <c>min</c> to <c>max</c>.</summary>
    Slider,

    /// <summary>Text the player types.</summary>
    TextField,

    /// <summary>A column of entries that scrolls when it is taller than its box.</summary>
    List,

    /// <summary>A bar filled by <c>value</c>, 0 to 1.</summary>
    Progress,

    /// <summary>
    /// Another layout file (<c>layout</c>) put in its place: a part made once and used in bigger
    /// interfaces, such as a character menu inside the main display.
    /// </summary>
    Part,
}

public static class Kinds
{
    public static readonly IReadOnlyList<Kind> All =
        [Kind.Panel, Kind.Row, Kind.Column, Kind.Text, Kind.Image, Kind.Button, Kind.Toggle, Kind.Slider, Kind.TextField, Kind.List, Kind.Progress, Kind.Part];

    private static readonly string[] Names = ["panel", "row", "column", "text", "image", "button", "toggle", "slider", "textField", "list", "progress", "part"];

    public static string Name(this Kind kind) => Names[(int)kind];

    /// <summary>Whether the kind holds other elements.</summary>
    public static bool HoldsChildren(this Kind kind) => kind is Kind.Panel or Kind.Row or Kind.Column or Kind.List;

    /// <summary>Whether the player can press, change or type into it (and so move focus to it).</summary>
    public static bool IsControl(this Kind kind) => kind is Kind.Button or Kind.Toggle or Kind.Slider or Kind.TextField;

    public static Kind Read(JsonValue value) => (Kind)JsonRead.Variant(value, Names);
}

/// <summary>One element of a layout.</summary>
public sealed class Element(Kind kind)
{
    public Kind Kind { get; set; } = kind;

    /// <summary>How scripts and name rules find it; unique in the layout when set.</summary>
    public string Name { get; set; } = "";

    /// <summary>Class rules that apply to it.</summary>
    public List<string> Classes { get; private set; } = [];

    /// <summary>What text, a button, a toggle or a text field shows.</summary>
    public string Text { get; set; } = "";

    /// <summary>An image file (PNG or JPEG), relative to the project folder.</summary>
    public string Image { get; set; } = "";

    /// <summary>
    /// The message the object's scripts get when the player clicks this element (any kind: a panel,
    /// an image, a text...), such as <c>openInventory</c>; empty for none.
    /// </summary>
    public string OnClick { get; set; } = "";

    /// <summary>A part's layout file (<c>.bui</c>), relative to the project folder (files: <c>layout</c>).</summary>
    public string LayoutFile { get; set; } = "";

    /// <summary>A toggle's state (0 or 1), a slider's number, a progress bar's fill (0 to 1), how far a list is scrolled (points).</summary>
    public double Value { get; set; }

    /// <summary>A slider's range.</summary>
    public double Min { get; set; }

    public double Max { get; set; } = 1.0;

    /// <summary>Shows a game state value instead of <c>value</c> or <c>text</c>, with no script: <c>game:score</c>, or <c>player:health</c> for the first player.</summary>
    public string Bind { get; set; } = "";

    /// <summary>Its own style, applied last: property name to value.</summary>
    public SortedDictionary<string, string> Style { get; private set; } = J.EmptyMap();

    /// <summary>
    /// On a part: the text shown by named elements inside it, by their name in the part, instead of
    /// the part's own (a character card's name, say, different where each card is used).
    /// </summary>
    public SortedDictionary<string, string> Texts { get; private set; } = J.EmptyMap();

    public List<Element> Children { get; private set; } = [];

    /// <summary>A copy that shares nothing with this element.</summary>
    public Element Clone() => new(Kind)
    {
        Name = Name,
        Classes = [.. Classes],
        Text = Text,
        Image = Image,
        OnClick = OnClick,
        LayoutFile = LayoutFile,
        Value = Value,
        Min = Min,
        Max = Max,
        Bind = Bind,
        Style = new(Style, Utf8Order.Instance),
        Texts = new(Texts, Utf8Order.Instance),
        Children = [.. Children.Select(c => c.Clone())],
    };

    /// <summary>Takes everything of <paramref name="other"/>, as a part takes the place of its element.</summary>
    public void Become(Element other)
    {
        (Kind, Name, Classes, Text, Image, OnClick, LayoutFile) = (other.Kind, other.Name, other.Classes, other.Text, other.Image, other.OnClick, other.LayoutFile);
        (Value, Min, Max, Bind, Style, Texts, Children) = (other.Value, other.Min, other.Max, other.Bind, other.Style, other.Texts, other.Children);
    }

    /// <summary>Every element in this one's tree, this one first, depth first.</summary>
    public List<Element> Walk()
    {
        var all = new List<Element>();
        var pending = new Stack<Element>([this]);
        while (pending.TryPop(out var element))
        {
            all.Add(element);
            for (var i = element.Children.Count - 1; i >= 0; i--)
            {
                pending.Push(element.Children[i]);
            }
        }
        return all;
    }

    /// <summary>An element as files and commands write it; throws <see cref="JsonShapeException"/> for a wrong shape.</summary>
    public static Element Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Element", true, "kind", "name", "classes", "text", "image", "onClick", "layout", "value", "min", "max", "bind", "style", "texts", "children");
        return new Element(f.Required("kind", Kinds.Read))
        {
            Name = f.Optional("name", JsonRead.Str, ""),
            Classes = f.Optional("classes", v => JsonRead.List(v, JsonRead.Str), []),
            Text = f.Optional("text", JsonRead.Str, ""),
            Image = f.Optional("image", JsonRead.Str, ""),
            OnClick = f.Optional("onClick", JsonRead.Str, ""),
            LayoutFile = f.Optional("layout", JsonRead.Str, ""),
            Value = f.Optional("value", JsonRead.F64, 0.0),
            Min = f.Optional("min", JsonRead.F64, 0.0),
            Max = f.Optional("max", JsonRead.F64, 1.0),
            Bind = f.Optional("bind", JsonRead.Str, ""),
            Style = f.Optional("style", J.ReadMap, J.EmptyMap()),
            Texts = f.Optional("texts", J.ReadMap, J.EmptyMap()),
            Children = f.Optional("children", v => JsonRead.List(v, Read), []),
        };
    }

    /// <summary>The element as files write it: fields with their default values left out.</summary>
    public JsonObject Write()
    {
        var o = new JsonObject { { "kind", J.S(Kind.Name()) } };
        void Put(string key, string text)
        {
            if (text.Length > 0)
            {
                o.Add(key, J.S(text));
            }
        }
        Put("name", Name);
        if (Classes.Count > 0)
        {
            o.Add("classes", J.Strings(Classes));
        }
        Put("text", Text);
        Put("image", Image);
        Put("onClick", OnClick);
        Put("layout", LayoutFile);
        if (Value != 0.0)
        {
            o.Add("value", J.D(Value));
        }
        if (Min != 0.0)
        {
            o.Add("min", J.D(Min));
        }
        if (Max != 1.0)
        {
            o.Add("max", J.D(Max));
        }
        Put("bind", Bind);
        if (Style.Count > 0)
        {
            o.Add("style", J.Map(Style));
        }
        if (Texts.Count > 0)
        {
            o.Add("texts", J.Map(Texts));
        }
        if (Children.Count > 0)
        {
            o.Add("children", new JsonArray(Children.Select(c => c.Write())));
        }
        return o;
    }
}

/// <summary>A layout file: the element tree and the style files it uses.</summary>
public sealed class Layout(string format, uint version, List<string> styles, Element root)
{
    public const string LayoutFormat = "bayleaf.interface";
    public const string LayoutExtension = "bui";

    /// <summary>Version 2 lets a part change the text of named elements inside it (<c>texts</c>).</summary>
    public const uint CurrentVersion = 2;

    /// <summary>Most elements in one layout.</summary>
    public const int MaxElements = 4096;

    /// <summary>Deepest nesting.</summary>
    public const int MaxDepth = 32;

    /// <summary>Longest text in an element, in bytes.</summary>
    public const int MaxText = 4096;

    /// <summary>Longest element name, in bytes.</summary>
    public const int MaxName = 48;

    /// <summary>Most parts nested inside one another.</summary>
    public const int MaxPartDepth = 8;

    /// <summary>Longest name of an element inside a part, so <c>pressed:&lt;name&gt;</c> stays a message name.</summary>
    public const int MaxPartName = 56;

    public string Format { get; set; } = format;

    public uint Version { get; set; } = version;

    /// <summary>Style files (<c>.bstyle</c>), relative to the project folder, applied in this order.</summary>
    public List<string> Styles { get; } = styles;

    public Element Root { get; set; } = root;

    /// <summary>A layout of <paramref name="root"/> in the current format, with no style files.</summary>
    public static Layout Of(Element root) => new(LayoutFormat, CurrentVersion, [], root);

    public Layout Clone() => new(Format, Version, [.. Styles], Root.Clone());

    /// <summary>A new layout: a full-screen panel with a title and a button.</summary>
    public static Layout Starter()
    {
        var root = new Element(Kind.Panel) { Name = "Screen" };
        root.Style["width"] = "100%";
        root.Style["height"] = "100%";
        root.Style["alignItems"] = "center";
        root.Style["justify"] = "center";
        root.Style["gap"] = "16px";
        var title = new Element(Kind.Text) { Name = "Title", Text = "My Game" };
        title.Style["fontSize"] = "40px";
        var start = new Element(Kind.Button) { Name = "Start", Text = "Start" };
        root.Children.AddRange([title, start]);
        return Of(root);
    }

    /// <summary>A layout as files and commands write it, not yet checked; throws <see cref="JsonShapeException"/> for a wrong shape.</summary>
    public static Layout Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Layout", true, "format", "version", "styles", "root");
        return new Layout(f.Required("format", JsonRead.Str), f.Required("version", JsonRead.U32), f.Optional("styles", v => JsonRead.List(v, JsonRead.Str), []), f.Required("root", Element.Read));
    }

    public static Result<Layout, string> FromText(string text)
    {
        var read = JsonReader.Parse(text).TryGet(out var value, out var parseError)
            ? JsonRead.Try(() => Read(value))
            : Result.Fail(parseError);
        if (!read.TryGet(out var layout, out var error))
        {
            return Result.Fail($"not an interface layout: {error}");
        }
        var valid = layout.Validate();
        return valid.IsOk ? layout : valid.AsFailure();
    }

    /// <summary>The file's text.</summary>
    public string ToText() => ToJson().ToPretty() + "\n";

    /// <summary>The layout as its file holds it.</summary>
    public JsonObject ToJson()
    {
        var o = new JsonObject { { "format", J.S(Format) }, { "version", J.U(Version) } };
        if (Styles.Count > 0)
        {
            o.Add("styles", J.Strings(Styles));
        }
        o.Add("root", Root.Write());
        return o;
    }

    public static Result<Layout, string> Load(string path) => FileText.Load(path).TryGet(out var text, out var error) ? FromText(text).MapError(e => $"{path}: {e}") : Result.Fail(error);

    /// <summary>Checks the format, the tree's size and depth, names, kinds that hold children, values and every inline style.</summary>
    public Result<Unit, string> Validate()
    {
        if (Format != LayoutFormat)
        {
            return Result.Fail($"the format is {J.Debug(Format)}, not {J.Debug(LayoutFormat)}");
        }
        if (Version > CurrentVersion)
        {
            return Result.Fail(JsonRead.Invariant($"layout version {Version} is newer than this engine reads ({CurrentVersion})"));
        }
        foreach (var style in Styles)
        {
            if (!Bayleaf.Core.Text.EndsWithAsciiNoCase(style, $".{StyleSheet.StyleExtension}") || Bayleaf.Core.Text.Bytes(style) > 512)
            {
                return Result.Fail($"{J.Debug(style)} is not a .{StyleSheet.StyleExtension} file");
            }
        }
        var count = 0;
        var names = new HashSet<string>(StringComparer.Ordinal);
        var pending = new List<(Element Element, int Depth)> { (Root, 1) };
        while (pending.Count > 0)
        {
            var (element, depth) = pending[^1];
            pending.RemoveAt(pending.Count - 1);
            count++;
            if (count > MaxElements)
            {
                return Result.Fail(JsonRead.Invariant($"a layout holds at most {MaxElements} elements"));
            }
            if (depth > MaxDepth)
            {
                return Result.Fail(JsonRead.Invariant($"elements nest at most {MaxDepth} deep"));
            }
            var valid = CheckElement(element);
            if (valid.IsError)
            {
                return valid;
            }
            if (element.Name.Length > 0 && !names.Add(element.Name))
            {
                return Result.Fail($"two elements are named {J.Debug(element.Name)}; a name is used once");
            }
            pending.AddRange(element.Children.Select(c => (c, depth + 1)));
        }
        return J.Ok;
    }

    /// <summary>The element named <paramref name="name"/>.</summary>
    public Element? Find(string name) => Root.Walk().Find(e => e.Name == name);

    /// <summary>The element at <paramref name="at"/>: child indices from the root (empty for the root itself).</summary>
    public Element? ElementAt(IEnumerable<int> at)
    {
        var element = Root;
        foreach (var i in at)
        {
            if (i < 0 || i >= element.Children.Count)
            {
                return null;
            }
            element = element.Children[i];
        }
        return element;
    }

    private static Result<Unit, string> CheckElement(Element element)
    {
        static int Bytes(string text) => Bayleaf.Core.Text.Bytes(text);
        static bool EndsWith(string text, string suffix) => Bayleaf.Core.Text.EndsWithAsciiNoCase(text, suffix);
        var what = element.Name.Length == 0 ? element.Kind.Name() : $"{element.Kind.Name()} {J.Debug(element.Name)}";
        if (element.Children.Count > 0 && !element.Kind.HoldsChildren())
        {
            return Result.Fail($"{what} holds other elements; only panels, rows, columns and lists do");
        }
        // Short enough that `pressed:<name>` fits in a message name.
        if (Bytes(element.Name) > MaxName || Bayleaf.Core.Text.HasControl(element.Name) || element.Name.Contains(':', StringComparison.Ordinal))
        {
            return Result.Fail(JsonRead.Invariant($"{what}: a name has at most {MaxName} bytes, and no control characters or colons"));
        }
        if (Bytes(element.Text) > MaxText)
        {
            return Result.Fail(JsonRead.Invariant($"{what}: text is at most {MaxText} bytes"));
        }
        if (element.Texts.Count > 0 && element.Kind != Kind.Part)
        {
            return Result.Fail($"{what}: only a part changes the texts inside it");
        }
        if (element.Texts.Count > 256 || element.Texts.Any(t => t.Key.Length == 0 || Bytes(t.Key) > MaxPartName || Bytes(t.Value) > MaxText))
        {
            return Result.Fail(JsonRead.Invariant($"{what}: texts holds at most 256 names, each of an element inside the part, and texts of at most {MaxText} bytes"));
        }
        if (element.Classes.Count > 16 || element.Classes.Any(c => c.Length == 0 || Bytes(c) > 64 || c.Any(char.IsWhiteSpace)))
        {
            return Result.Fail($"{what}: at most 16 classes, each 1 to 64 characters without spaces");
        }
        if (Bytes(element.OnClick) > 64 || Bayleaf.Core.Text.HasControl(element.OnClick))
        {
            return Result.Fail($"{what}: onClick is a message name of at most 64 bytes, without control characters");
        }
        var isPart = element.Kind == Kind.Part;
        if (isPart && element.LayoutFile.Length == 0)
        {
            return Result.Fail($"{what}: a part names its layout file (.{LayoutExtension})");
        }
        if (isPart && (Bytes(element.LayoutFile) > 512 || !EndsWith(element.LayoutFile, $".{LayoutExtension}")))
        {
            return Result.Fail($"{what}: layout must be the path of a .{LayoutExtension} file");
        }
        if (!isPart && element.LayoutFile.Length > 0)
        {
            return Result.Fail($"{what}: only a part names a layout file");
        }
        if (element.Image.Length > 0 && (Bytes(element.Image) > 512 || !(EndsWith(element.Image, ".png") || EndsWith(element.Image, ".jpg") || EndsWith(element.Image, ".jpeg"))))
        {
            return Result.Fail($"{what}: image must be the path of a .png or .jpg file");
        }
        if (!(double.IsFinite(element.Value) && double.IsFinite(element.Min) && double.IsFinite(element.Max)) || element.Min > element.Max)
        {
            return Result.Fail($"{what}: value, min and max are finite numbers, min at most max");
        }
        if (element.Bind.Length > 0 && InterfaceRules.BindTarget(element.Bind) is null)
        {
            return Result.Fail($"{what}: bind is game:<name> or player:<name>, not {J.Debug(element.Bind)}");
        }
        foreach (var (property, value) in element.Style)
        {
            var valid = Style.CheckProperty(property, value);
            if (valid.IsError)
            {
                return Result.Fail($"{what}: {valid.Error}");
            }
        }
        return J.Ok;
    }
}

/// <summary>A layout with every part put in its place, and the files that took.</summary>
/// <param name="Styles">The style files the parts' layouts name, after the layout's own, each once.</param>
/// <param name="Parts">The parts' layout files, each once.</param>
public sealed record Expanded(Element Root, IReadOnlyList<string> Styles, IReadOnlyList<string> Parts);

/// <summary>Where a bound value comes from: the game's state or the first player's.</summary>
public enum BindScope
{
    Game,
    Player,
}

/// <summary>What the interface's files mean beyond their fields: parts, binds and the pointer.</summary>
public static class InterfaceRules
{
    /// <summary>
    /// Puts each part of <paramref name="layout"/> in its place: the part's layout root, named as the
    /// part element (and its classes and own style added, so the bigger layout sizes and places it),
    /// with every named element inside it named <c>&lt;part&gt;/&lt;element&gt;</c>, so the same part
    /// can be used twice. <paramref name="load"/> reads a part's layout file. A part that names
    /// itself, a missing file or a name grown too long for a message (<c>pressed:&lt;name&gt;</c>) is an error.
    /// </summary>
    public static Result<Expanded, string> Expand(Layout layout, Func<string, Result<Layout, string>> load)
    {
        var root = layout.Root.Clone();
        var styles = new List<string>(layout.Styles);
        var parts = new List<string>();
        var expanded = ExpandIn(root, load, styles, parts, []);
        if (expanded.IsError)
        {
            return expanded.AsFailure();
        }
        if (root.Walk().Count > Layout.MaxElements)
        {
            return Result.Fail(JsonRead.Invariant($"with its parts, the layout holds more than {Layout.MaxElements} elements"));
        }
        return new Expanded(root, styles, parts);
    }

    private static Result<Unit, string> ExpandIn(Element element, Func<string, Result<Layout, string>> load, List<string> styles, List<string> parts, List<string> open)
    {
        if (element.Kind != Kind.Part)
        {
            foreach (var child in element.Children)
            {
                var inner = ExpandIn(child, load, styles, parts, open);
                if (inner.IsError)
                {
                    return inner;
                }
            }
            return J.Ok;
        }
        var file = element.LayoutFile;
        if (open.Contains(file) || open.Count >= Layout.MaxPartDepth)
        {
            return Result.Fail(JsonRead.Invariant($"the part {file} is inside itself, or parts nest more than {Layout.MaxPartDepth} deep"));
        }
        if (!load(file).TryGet(out var part, out var error))
        {
            return Result.Fail($"part {file}: {error}");
        }
        foreach (var style in part.Styles.Where(s => !styles.Contains(s)))
        {
            styles.Add(style);
        }
        if (!parts.Contains(file))
        {
            parts.Add(file);
        }
        var root = part.Root.Clone();
        open.Add(file);
        var nested = ExpandIn(root, load, styles, parts, open);
        if (nested.IsError)
        {
            return nested;
        }
        open.RemoveAt(open.Count - 1);
        // Texts changed where the part is used, by the names inside it.
        foreach (var (name, text) in element.Texts)
        {
            var found = false;
            foreach (var inner in root.Walk().Where(e => e.Name == name))
            {
                inner.Text = text;
                found = true;
            }
            if (!found)
            {
                return Result.Fail($"part {file}: no element inside it is named {J.Debug(name)}, so its text cannot change");
            }
        }
        if (element.Name.Length > 0)
        {
            var prefix = element.Name;
            foreach (var child in root.Children)
            {
                var pending = new Stack<Element>([child]);
                while (pending.TryPop(out var inner))
                {
                    if (inner.Name.Length > 0)
                    {
                        inner.Name = $"{prefix}/{inner.Name}";
                        if (Text.Bytes(inner.Name) > Layout.MaxPartName)
                        {
                            return Result.Fail(JsonRead.Invariant($"{inner.Name} is too long a name inside a part (at most {Layout.MaxPartName} bytes)"));
                        }
                    }
                    foreach (var next in inner.Children)
                    {
                        pending.Push(next);
                    }
                }
            }
            root.Name = prefix;
        }
        root.Classes.AddRange(element.Classes);
        foreach (var (property, value) in element.Style)
        {
            root.Style[property] = value;
        }
        element.Become(root);
        return J.Ok;
    }

    /// <summary>
    /// Whether a placed element takes the pointer: its style's <c>pointer</c>, else by default
    /// controls, elements with On Click and boxes with a background (text, images and empty boxes let
    /// it through, so a display's layout never stops the game).
    /// </summary>
    public static bool BlocksPointer(Element element, Style style) =>
        style.TakesPointer ?? (element.Kind.IsControl() || element.OnClick.Length > 0 || style.Background.A > 0);

    /// <summary>A <c>bind</c> text read: its scope and the value's name.</summary>
    public static (BindScope Scope, string Name)? BindTarget(string bind)
    {
        var colon = bind.IndexOf(':', StringComparison.Ordinal);
        if (colon < 0)
        {
            return null;
        }
        var (scope, name) = (bind[..colon], bind[(colon + 1)..]);
        BindScope? read = scope switch
        {
            "game" => BindScope.Game,
            "player" => BindScope.Player,
            _ => null,
        };
        return read is { } s && name.Length > 0 && Text.Bytes(name) <= 64 ? (s, name) : null;
    }
}

using System.Numerics;

using Bayleaf.Core;

namespace Bayleaf.GameInterface.Tests;

/// <summary>Every character half the font size wide; lines wrap at whole characters.</summary>
internal sealed class Monospace : ITextMeasure
{
    public Vector2 Text(string text, float fontSize, float? wrap)
    {
        var per = fontSize * 0.5f;
        var chars = (float)Bayleaf.Core.Text.Chars(text);
        var line = fontSize * 1.25f;
        if (wrap is { } width && width >= per && chars * per > width)
        {
            var perLine = Math.Max(MathF.Floor(width / per), 1f);
            return new(perLine * per, MathF.Ceiling(chars / perLine) * line);
        }
        return new(chars * per, text.Length == 0 ? 0f : line);
    }

    public Vector2? Image(string path) => new Vector2(64f, 32f);
}

public class InterfaceTests
{
    private static readonly Monospace Measure = new();
    private static readonly Vector2 Screen = new(800f, 600f);

    private static Element Make(Kind kind, string name, params (string Property, string Value)[] style)
    {
        var e = new Element(kind) { Name = name };
        foreach (var (property, value) in style)
        {
            e.Style[property] = value;
        }
        return e;
    }

    private static List<Placed> Lay(Element root, float scale = 1f) => Placement.Place(root, [], Screen, scale, Measure);

    private static Placed At(List<Placed> all, string name) => all.Find(p => p.Element.Name == name) ?? throw new InvalidOperationException($"{name} is not placed");

    private static bool Inside(Rect inner, Rect outer) =>
        inner.X >= outer.X - 0.01f && inner.Y >= outer.Y - 0.01f && inner.Right <= outer.Right + 0.01f && inner.Bottom <= outer.Bottom + 0.01f;

    [Fact]
    public void LayoutFilesAreCheckedAndSayWhatIsWrong()
    {
        var starter = Layout.Starter();
        Assert.Equal(starter.ToText(), Layout.FromText(starter.ToText()).Value.ToText());

        var textWithChildren = Layout.Starter();
        textWithChildren.Root.Children[0].Children.Add(new Element(Kind.Text));
        Assert.Contains("holds other elements", textWithChildren.Validate().Error, StringComparison.Ordinal);

        var twice = Layout.Starter();
        twice.Root.Children[1].Name = "Title";
        Assert.Contains("named", twice.Validate().Error, StringComparison.Ordinal);

        foreach (var (property, value) in new[] { ("width", "12"), ("color", "red"), ("gap", "5%"), ("float", "left"), ("fontSize", "1px"), ("grow", "-1") })
        {
            var bad = Layout.Starter();
            bad.Root.Style[property] = value;
            Assert.True(bad.Validate().IsError, $"{property}: {value} is refused");
        }

        const string twoPicks = """{"format": "bayleaf.style", "version": 1, "rules": [{"kind": "button", "class": "big", "set": {}}]}""";
        Assert.Contains("exactly one", StyleSheet.FromText(twoPicks).Error, StringComparison.Ordinal);
        var deep = new Element(Kind.Text);
        for (var i = 0; i < Layout.MaxDepth + 1; i++)
        {
            var outer = new Element(Kind.Panel);
            outer.Children.Add(deep);
            deep = outer;
        }
        Assert.True(Layout.Of(deep).Validate().IsError, "nesting has a limit");
    }

    [Fact]
    public void StylesCascadeKindThenClassThenNameThenTheElementsOwn()
    {
        var sheet = StyleSheet.FromText("""
            {"format": "bayleaf.style", "version": 1, "rules": [
                {"name": "Go", "set": {"color": "#00ff00"}},
                {"class": "loud", "set": {"color": "#ff0000", "fontSize": "30px"}},
                {"kind": "button", "set": {"color": "#0000ff", "radius": "2px", "fontSize": "20px"}}
            ]}
            """).Value;
        var button = Make(Kind.Button, "Go");
        button.Classes.Add("loud");
        var style = Style.Of(button, [sheet]);
        Assert.Equal(new Color(0, 255, 0, 255), style.Color);
        Assert.True(style.FontSize == 30f, "the class rule wins over the kind rule");
        Assert.True(style.Radius == 2f, "the kind rule still sets what nothing later sets");
        button.Style["color"] = "#ffffff";
        Assert.Equal(new Color(255, 255, 255, 255), Style.Of(button, [sheet]).Color);
    }

    [Fact]
    public void ARowSharesTheRoomLeftByGrowAndKeepsFixedSizes()
    {
        var root = Make(Kind.Row, "Bar", ("width", "400px"), ("height", "50px"), ("gap", "10px"));
        root.Children.AddRange([Make(Kind.Panel, "Fixed", ("width", "100px")), Make(Kind.Panel, "One", ("grow", "1")), Make(Kind.Panel, "Three", ("grow", "3"))]);
        var all = Lay(root);
        var (bar, fixedBox, one, three) = (At(all, "Bar"), At(all, "Fixed"), At(all, "One"), At(all, "Three"));
        Assert.True(Math.Abs(fixedBox.Rect.W - 100f) < 0.01f, "a fixed size is kept");
        Assert.True(three.Rect.W > one.Rect.W * 2.9f && three.Rect.W < one.Rect.W * 3.1f, "grow 3 takes three times grow 1's share");
        Assert.True(one.Rect.X >= fixedBox.Rect.Right + 9.9f && three.Rect.X >= one.Rect.Right + 9.9f, "the gap lies between them");
        Assert.True(Math.Abs(three.Rect.Right - bar.Rect.Right) < 0.01f, "together they fill the row");
        Assert.True(one.Rect.H == bar.Rect.H, "they stretch across by default");
    }

    [Fact]
    public void AColumnCentresItsChildrenAndHiddenOnesTakeNoRoom()
    {
        var root = Make(Kind.Column, "Screen", ("width", "100%"), ("height", "100%"), ("alignItems", "center"), ("justify", "center"));
        var title = Make(Kind.Text, "Title");
        title.Text = "Hello";
        var gone = Make(Kind.Panel, "Gone", ("height", "200px"), ("visible", "false"));
        var start = Make(Kind.Button, "Start");
        start.Text = "Start";
        root.Children.AddRange([title, gone, start]);
        var all = Lay(root);
        Assert.DoesNotContain(all, p => p.Element.Name == "Gone");
        var (screen, t, s) = (At(all, "Screen"), At(all, "Title"), At(all, "Start"));
        Assert.True(screen.Rect.W == 800f && screen.Rect.H == 600f, "100% of the screen");
        static float Middle(Rect r) => r.X + r.W / 2f;
        Assert.True(Math.Abs(Middle(t.Rect) - 400f) < 0.5f && Math.Abs(Middle(s.Rect) - 400f) < 0.5f, "centred across");
        var topGap = t.Rect.Y;
        var bottomGap = 600f - s.Rect.Bottom;
        Assert.True(Math.Abs(topGap - bottomGap) < 0.5f, $"centred along: {topGap} and {bottomGap}");
        Assert.True(s.Rect.Y >= t.Rect.Bottom, "the button comes after the title, with no room kept for the hidden panel between");
        Assert.True(s.Rect.W > 5f * 9f, "a button is wider than its text, by its padding");
    }

    [Fact]
    public void TooLittleRoomShrinksChildrenToFitAndSharesFollowTheParent()
    {
        var root = Make(Kind.Row, "Narrow", ("width", "300px"), ("height", "40px"));
        root.Children.AddRange([Make(Kind.Panel, "A", ("width", "200px"), ("shrink", "1")), Make(Kind.Panel, "B", ("width", "200px"), ("shrink", "1"))]);
        var all = Lay(root);
        var narrow = At(all, "Narrow").Rect;
        Assert.All(all, p => Assert.True(Inside(p.Rect, narrow), "everything fits inside the row"));
        // Without a shrink of their own, boxes keep their size and run past the row instead of
        // cutting what they hold.
        root.Children.Clear();
        root.Children.AddRange([Make(Kind.Panel, "A", ("width", "200px")), Make(Kind.Panel, "B", ("width", "200px"))]);
        all = Lay(root);
        Assert.All(all.Where(p => p.Element.Kind == Kind.Panel), p => Assert.True(Math.Abs(p.Rect.W - 200f) < 0.01f));

        var half = Make(Kind.Panel, "Half", ("width", "50%"), ("height", "50%"), ("padding", "10px"));
        half.Children.Add(Make(Kind.Panel, "Quarter", ("width", "50%"), ("height", "20px")));
        all = Lay(half);
        var (h, q) = (At(all, "Half").Rect, At(all, "Quarter").Rect);
        Assert.True(h.W > 399f && h.W < 401f);
        Assert.True(q.W < h.W / 2f && q.W > h.W / 2f - 11f, "a share of the parent's inner width, inside its padding");
        Assert.True(Inside(q, h));
    }

    [Fact]
    public void TextWrapsInANarrowColumnAndABiggerScaleMakesEverythingBigger()
    {
        Element Text() => new(Kind.Text) { Name = "Words", Text = "a sentence long enough to wrap" };
        var wide = Make(Kind.Column, "Wide", ("width", "600px"));
        wide.Children.Add(Text());
        var narrow = Make(Kind.Column, "Narrow", ("width", "100px"));
        narrow.Children.Add(Text());
        var (w, n) = (At(Lay(wide), "Words").Rect, At(Lay(narrow), "Words").Rect);
        Assert.True(n.H > w.H * 2f, $"wrapped onto more lines: {n.H} against {w.H}");
        Assert.True(n.W <= 100f + 0.01f, "inside the column");

        var (small, big) = (Lay(wide), Lay(wide, 2f));
        Assert.True(At(big, "Wide").Rect.W > At(small, "Wide").Rect.W * 1.9f, "fixed sizes scale");
        Assert.True(At(big, "Words").Style.FontSize > At(small, "Words").Style.FontSize, "so do fonts");
    }

    private static Element Scores()
    {
        var list = Make(Kind.List, "Scores", ("height", "100px"), ("width", "200px"));
        list.Children.AddRange(Enumerable.Range(0, 10).Select(i => Make(Kind.Panel, $"Entry {i}", ("height", "30px"))));
        return list;
    }

    [Fact]
    public void AListCutsOffEntriesPastItsBottom()
    {
        var all = Lay(Scores());
        var box = At(all, "Scores").Rect;
        var last = At(all, "Entry 9");
        Assert.True(last.Rect.Y > box.Bottom, "the last entry lies below the list");
        Assert.True(last.Clip.H == 0f, "and none of it shows");
        Assert.True(At(all, "Entry 0").Clip.H > 0f);
    }

    [Fact]
    public void EveryListedPropertyStartsFromAValueItTakes()
    {
        foreach (var (property, value) in StyleSheet.Properties)
        {
            Assert.True(Style.Defaults(Kind.Panel).Apply(property, value).IsOk, $"{property}: {value}");
        }
        // And the list has every property the styles read: an unknown name says so.
        Assert.True(Style.Defaults(Kind.Panel).Apply("float", "left").IsError);
    }

    [Fact]
    public void APartTakesItsPlaceOncePerUseWithItsNamesKeptApart()
    {
        // A character card: a panel with a name and a health bar, its own style file.
        var cardRoot = Make(Kind.Panel, "Card", ("width", "200px"));
        cardRoot.Children.AddRange([new Element(Kind.Text) { Name = "Name", Text = "Hero" }, Make(Kind.Progress, "Health")]);
        var card = Layout.Of(cardRoot);
        card.Styles.Add("ui/card.bstyle");
        // A display using it twice, the second one wider.
        var left = Make(Kind.Part, "Left");
        left.LayoutFile = "ui/card.bui";
        var right = Make(Kind.Part, "Right", ("width", "300px"));
        right.LayoutFile = "ui/card.bui";
        var root = Make(Kind.Row, "Display");
        root.Children.AddRange([left, right]);
        var display = Layout.Of(root);
        Assert.True(display.Validate().IsOk);

        Result<Layout, string> Load(string path) => path == "ui/card.bui" ? card : Result.Fail($"{path} is not there");
        var expanded = InterfaceRules.Expand(display, Load).Value;
        Assert.Equal(["Display", "Left", "Left/Name", "Left/Health", "Right", "Right/Name", "Right/Health"], expanded.Root.Walk().Select(e => e.Name));
        Assert.DoesNotContain(expanded.Root.Walk(), e => e.Kind == Kind.Part);
        Assert.Equal("200px", expanded.Root.Children[0].Style["width"]);
        Assert.Equal("300px", expanded.Root.Children[1].Style["width"]);
        Assert.Equal(["ui/card.bstyle"], expanded.Styles);

        // A part inside itself, or a file that is not there, says so.
        var selfish = new Element(Kind.Part) { LayoutFile = "ui/loop.bui" };
        var looping = Layout.Of(selfish);
        Assert.Contains("inside itself", InterfaceRules.Expand(looping, _ => looping).Error, StringComparison.Ordinal);
        var missing = Make(Kind.Part, "Gone");
        missing.LayoutFile = "ui/gone.bui";
        Assert.True(InterfaceRules.Expand(Layout.Of(missing), path => Result.Fail($"{path} is not there")).IsError);
        Assert.True(Layout.Of(new Element(Kind.Part) { Name = "Empty" }).Validate().IsError, "a part names its file");
    }

    [Fact]
    public void ControlsAndBoxesWithABackgroundTakeThePointerAndTheStyleCanSayOtherwise()
    {
        static bool Blocks(Kind kind, string onClick, params (string, string)[] style)
        {
            var e = Make(kind, "E", style);
            e.OnClick = onClick;
            return InterfaceRules.BlocksPointer(e, Style.Of(e, []));
        }
        Assert.True(Blocks(Kind.Button, ""), "a control takes it");
        Assert.True(Blocks(Kind.Panel, "", ("background", "#00000080")), "a box with a background takes it");
        Assert.True(Blocks(Kind.Image, "open"), "anything with On Click takes it");
        Assert.False(Blocks(Kind.Panel, ""), "an empty layout box lets it through");
        Assert.False(Blocks(Kind.Text, ""), "text lets it through");
        Assert.False(Blocks(Kind.Panel, "", ("background", "#000000"), ("pointer", "pass")), "the style lets it through");
        Assert.True(Blocks(Kind.Text, "", ("pointer", "block")), "the style takes it");
    }

    [Fact]
    public void AListScrollsByItsValueNoFurtherThanWhatItHolds()
    {
        var list = Scores();
        (float Top, (float Offset, float Most)? Scroll) Where()
        {
            var all = Lay(list);
            return (At(all, "Entry 0").Rect.Y, At(all, "Scores").Scroll);
        }
        var (top, scroll) = Where();
        Assert.NotNull(scroll);
        var (offset, most) = scroll.Value;
        Assert.True(offset == 0f);
        Assert.True(most > 200f, "ten entries of 30 and their gaps, in 100");
        list.Value = 50;
        Assert.True(Math.Abs(top - Where().Top - 50f) < 0.01f, "scrolled up by 50");
        list.Value = 10_000;
        Assert.True(Where().Scroll?.Offset == most, "no further than the end");
    }

    [Fact]
    public void APartShowsTheTextsItsUseGivesToTheNamesInsideIt()
    {
        var cardRoot = Make(Kind.Panel, "Card");
        cardRoot.Children.Add(new Element(Kind.Text) { Name = "Name", Text = "Hero" });
        var card = Layout.Of(cardRoot);
        var left = Make(Kind.Part, "Left");
        left.LayoutFile = "card.bui";
        var right = left.Clone();
        right.Name = "Right";
        right.Texts["Name"] = "Rogue";
        var root = Make(Kind.Row, "Display");
        root.Children.AddRange([left, right]);
        var display = Layout.Of(root);
        Assert.True(display.Validate().IsOk);

        var expanded = InterfaceRules.Expand(display, _ => card).Value;
        string TextOf(string name) => expanded.Root.Walk().Find(e => e.Name == name)!.Text;
        Assert.Equal(("Hero", "Rogue"), (TextOf("Left/Name"), TextOf("Right/Name")));

        // A name that is not inside the part says so; texts belong to parts only.
        var wrong = display.Clone();
        wrong.Root.Children[1].Texts["Title"] = "?";
        Assert.Contains("Title", InterfaceRules.Expand(wrong, _ => card).Error, StringComparison.Ordinal);
        var textOnly = display.Clone();
        textOnly.Root.Texts["Name"] = "x";
        Assert.True(textOnly.Validate().IsError);
    }
}

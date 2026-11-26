using System.Numerics;

namespace Bayleaf.GameInterface;

/// <summary>A box on screen, in points from the top left.</summary>
public readonly record struct Rect(float X, float Y, float W, float H)
{
    public float Right => X + W;

    public float Bottom => Y + H;

    public bool Contains(float x, float y) => x >= X && x < Right && y >= Y && y < Bottom;

    /// <summary>The part of this box inside <paramref name="other"/> (empty when they do not meet).</summary>
    public Rect Intersect(Rect other)
    {
        var (x, y) = (J.Max(X, other.X), J.Max(Y, other.Y));
        var (r, b) = (J.Min(Right, other.Right), J.Min(Bottom, other.Bottom));
        return new(x, y, J.Max(r - x, 0f), J.Max(b - y, 0f));
    }

    /// <summary>This box less <paramref name="padding"/>.</summary>
    internal Rect Inset(Sides padding) =>
        new(X + padding.Left, Y + padding.Top, J.Max(W - padding.Right - padding.Left, 0f), J.Max(H - padding.Top - padding.Bottom, 0f));
}

/// <summary>How big text and images are, from whoever draws them (the fonts live there).</summary>
public interface ITextMeasure
{
    /// <summary>Width and height of <paramref name="text"/> at <paramref name="fontSize"/> points, wrapped at <paramref name="wrap"/> points when given.</summary>
    Vector2 Text(string text, float fontSize, float? wrap);

    /// <summary>An image's size in points, when it can be read.</summary>
    Vector2? Image(string path);
}

/// <summary>An element placed on screen, with its look (lengths already scaled).</summary>
/// <param name="Clip">What of it shows: its parents' boxes cut it (a list's entries past its bottom).</param>
/// <param name="Parent">The index of its parent in the placed list.</param>
public sealed record Placed(Element Element, Style Style, Rect Rect, Rect Clip, int? Parent)
{
    /// <summary>A list that holds more than it shows: how far it is scrolled, and how far it can be (in points along its direction).</summary>
    public (float Offset, float Most)? Scroll { get; internal set; }
}

/// <summary>
/// Flexible boxes, one line each: a box lays its children along its direction with a gap, gives
/// the room left over to those that grow (or takes it from those that shrink when there is too
/// little), and places them across by <c>alignItems</c> (or their own <c>alignSelf</c>). Sizes are
/// fixed (<c>px</c>), shares of the parent's inner size (<c>%</c>), or <c>auto</c>: what the content
/// needs. Hidden elements take no room. The same layout and screen size always give the same places.
/// </summary>
public static class Placement
{
    private sealed record Box(Element Element, Style Style, List<Box> Children);

    /// <summary>
    /// Places every visible element of <paramref name="root"/> on a screen <paramref name="screen"/>
    /// points wide and high, with every length times <paramref name="scale"/>. Parents come before
    /// their children, in the order they are drawn.
    /// </summary>
    public static List<Placed> Place(Element root, IReadOnlyList<StyleSheet> sheets, Vector2 screen, float scale, ITextMeasure measure)
    {
        scale = float.IsFinite(scale) && scale > 0f ? scale : 1f;
        var placed = new List<Placed>();
        if (Build(root, sheets, scale) is not { } box)
        {
            return placed;
        }
        var screenRect = new Rect(0f, 0f, J.Max(screen.X, 0f), J.Max(screen.Y, 0f));
        var size = OuterSize(box, screenRect.W, screenRect.W, screenRect.H, measure);
        PlaceBox(box, new Rect(0f, 0f, size.X, size.Y), screenRect, null, measure, placed);
        return placed;
    }

    private static Box? Build(Element element, IReadOnlyList<StyleSheet> sheets, float scale)
    {
        var style = Style.Of(element, sheets);
        if (!style.Visible)
        {
            return null;
        }
        ScaleStyle(style, scale);
        var children = element.Children.Select(c => Build(c, sheets, scale)).OfType<Box>().ToList();
        return new Box(element, style, children);
    }

    private static void ScaleStyle(Style style, float scale)
    {
        Length Scaled(Length l) => l.Unit == LengthUnit.Px ? Length.Px(l.Value * scale) : l;
        style.Width = Scaled(style.Width);
        style.Height = Scaled(style.Height);
        style.MinWidth = Scaled(style.MinWidth);
        style.MinHeight = Scaled(style.MinHeight);
        style.MaxWidth = Scaled(style.MaxWidth);
        style.MaxHeight = Scaled(style.MaxHeight);
        style.Gap *= scale;
        style.Padding = style.Padding.Times(scale);
        style.BorderWidth *= scale;
        style.Radius *= scale;
        style.FontSize *= scale;
    }

    /// <summary>A length against its parent's size along the same axis; null for <c>auto</c>.</summary>
    private static float? Resolve(Length length, float parent) => length.Unit switch
    {
        LengthUnit.Auto => null,
        LengthUnit.Px => length.Value,
        _ => parent * length.Value / 100f,
    };

    /// <summary><paramref name="size"/> kept between the style's smallest and largest sizes on one axis.</summary>
    private static float Clamp(float size, Style style, float parent, bool horizontal)
    {
        var (min, max) = horizontal ? (style.MinWidth, style.MaxWidth) : (style.MinHeight, style.MaxHeight);
        size = J.Max(size, 0f);
        if (Resolve(max, parent) is { } most)
        {
            size = J.Min(size, most);
        }
        if (Resolve(min, parent) is { } least)
        {
            size = J.Max(size, least);
        }
        return size;
    }

    private static float PadW(Style style) => style.Padding.Right + style.Padding.Left + style.BorderWidth * 2f;

    private static float PadH(Style style) => style.Padding.Top + style.Padding.Bottom + style.BorderWidth * 2f;

    /// <summary>What a box needs, padding included, when <paramref name="width"/> points are there for it (to wrap text in).</summary>
    private static Vector2 ContentSize(Box box, float? width, ITextMeasure measure)
    {
        var style = box.Style;
        var e = box.Element;
        float? innerWidth = width is { } w0 ? J.Max(w0 - PadW(style), 0f) : null;
        var line = style.FontSize * 1.25f;
        Vector2 inner;
        switch (e.Kind)
        {
            case Kind.Text:
                inner = measure.Text(e.Text, style.FontSize, innerWidth);
                break;
            case Kind.Button or Kind.TextField:
                {
                    var t = measure.Text(e.Text, style.FontSize, innerWidth);
                    inner = new(t.X, J.Max(t.Y, line));
                    break;
                }
            case Kind.Toggle:
                {
                    var t = measure.Text(e.Text, style.FontSize, innerWidth is { } iw ? J.Max(iw - line - style.Gap, 0f) : null);
                    inner = new(line + style.Gap + t.X, J.Max(t.Y, line));
                    break;
                }
            case Kind.Image:
                inner = measure.Image(e.Image) ?? Vector2.Zero;
                break;
            case Kind.Slider or Kind.Progress or Kind.Part:
                // A part left in place (one that could not be read) takes no room.
                inner = Vector2.Zero;
                break;
            default:
                {
                    var row = style.Direction == Direction.Row;
                    var (main, cross) = (0f, 0f);
                    for (var i = 0; i < box.Children.Count; i++)
                    {
                        var size = OuterSize(box.Children[i], row ? null : innerWidth, innerWidth ?? 0f, 0f, measure);
                        var (m, c) = row ? (size.X, size.Y) : (size.Y, size.X);
                        main += m + (i > 0 ? style.Gap : 0f);
                        cross = J.Max(cross, c);
                    }
                    inner = row ? new(main, cross) : new(cross, main);
                    break;
                }
        }
        return new(inner.X + PadW(style), inner.Y + PadH(style));
    }

    /// <summary>A child's size before growing and shrinking: fixed, a share of the parent, or its content.</summary>
    private static Vector2 OuterSize(Box box, float? width, float parentW, float parentH, ITextMeasure measure)
    {
        // A share of a size not known yet counts as `auto`.
        static float? Known(Length length, float parent) => length.Unit == LengthUnit.Percent && parent <= 0f ? null : Resolve(length, parent);
        var fixedW = Known(box.Style.Width, parentW);
        var fixedH = Known(box.Style.Height, parentH);
        var content = ContentSize(box, fixedW ?? width, measure);
        return new(Clamp(fixedW ?? content.X, box.Style, parentW, true), Clamp(fixedH ?? content.Y, box.Style, parentH, false));
    }

    private static void PlaceBox(Box box, Rect rect, Rect clip, int? parent, ITextMeasure measure, List<Placed> placed)
    {
        var index = placed.Count;
        clip = rect.Intersect(clip);
        placed.Add(new Placed(box.Element, box.Style with { }, rect, clip, parent));
        if (box.Children.Count == 0)
        {
            return;
        }
        var style = box.Style;
        var bw = style.BorderWidth;
        var inner = rect.Inset(new Sides(style.Padding.Top + bw, style.Padding.Right + bw, style.Padding.Bottom + bw, style.Padding.Left + bw));
        var row = style.Direction == Direction.Row;
        var (innerMain, innerCross) = row ? (inner.W, inner.H) : (inner.H, inner.W);
        // Each child's size along the box and across it, before the room left over is shared.
        var sizes = box.Children.Select(child =>
        {
            var stretch = (child.Style.AlignSelf ?? style.AlignItems) == Align.Stretch;
            // Across a column, a child that stretches wraps its text at the column's width.
            float? width = row ? null : stretch || child.Style.Width == Length.Auto ? inner.W : null;
            var size = OuterSize(child, width, inner.W, inner.H, measure);
            return row ? new[] { size.X, size.Y } : [size.Y, size.X];
        }).ToList();
        var count = box.Children.Count;
        var gaps = style.Gap * Math.Max(count - 1, 0);
        var used = J.Sum(sizes.Select(s => s[0])) + gaps;
        var free = innerMain - used;
        var totalGrow = J.Sum(box.Children.Select(c => c.Style.Grow));
        if (free > 0f && totalGrow > 0f)
        {
            for (var i = 0; i < count; i++)
            {
                sizes[i][0] += free * box.Children[i].Style.Grow / totalGrow;
            }
        }
        else if (free < 0f && box.Element.Kind != Kind.List)
        {
            // A list keeps its entries whole and cuts off what does not fit.
            var weight = J.Sum(sizes.Select((s, i) => s[0] * box.Children[i].Style.Shrink));
            if (weight > 0f)
            {
                for (var i = 0; i < count; i++)
                {
                    sizes[i][0] = J.Max(sizes[i][0] + free * sizes[i][0] * box.Children[i].Style.Shrink / weight, 0f);
                }
            }
        }
        // Sizes stay inside the smallest and largest each child allows.
        for (var i = 0; i < count; i++)
        {
            sizes[i][0] = Clamp(sizes[i][0], box.Children[i].Style, innerMain, row);
        }
        var leftOver = J.Max(innerMain - J.Sum(sizes.Select(s => s[0])) - gaps, 0f);
        var (cursor, between) = style.Justify switch
        {
            Justify.Center => (leftOver / 2f, 0f),
            Justify.End => (leftOver, 0f),
            Justify.SpaceBetween when count > 1 => (0f, leftOver / (count - 1)),
            _ => (0f, 0f),
        };
        // A list's entries past its box are cut off there, and it scrolls by its value (in points).
        var isList = box.Element.Kind == Kind.List;
        var childClip = isList ? inner.Intersect(clip) : clip;
        if (isList)
        {
            var content = J.Sum(sizes.Select(s => s[0])) + gaps;
            var most = J.Max(content - innerMain, 0f);
            var offset = Math.Clamp((float)box.Element.Value, 0f, most);
            cursor -= offset;
            if (most > 0f)
            {
                placed[index].Scroll = (offset, most);
            }
        }
        for (var i = 0; i < count; i++)
        {
            var child = box.Children[i];
            var size = sizes[i];
            var align = child.Style.AlignSelf ?? style.AlignItems;
            var fixedCross = row ? child.Style.Height : child.Style.Width;
            var cross = align == Align.Stretch && fixedCross == Length.Auto ? Clamp(innerCross, child.Style, innerCross, !row) : size[1];
            var offset = align switch
            {
                Align.Center => (innerCross - cross) / 2f,
                Align.End => innerCross - cross,
                _ => 0f,
            };
            var childRect = row ? new Rect(inner.X + cursor, inner.Y + offset, size[0], cross) : new Rect(inner.X + offset, inner.Y + cursor, cross, size[0]);
            PlaceBox(child, childRect, childClip, index, measure, placed);
            cursor += size[0] + style.Gap + between;
        }
    }
}

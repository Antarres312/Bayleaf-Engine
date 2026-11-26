using System.Numerics;

using Bayleaf.Core.Json;

namespace Bayleaf.Graph.Tests;

public class GraphTests
{
    private static readonly PinDef Num = new("value", "Value", PinType.Number);

    internal static readonly Catalog Catalog = new()
    {
        Domain = "test",
        Label = "Test",
        Extension = "btest",
        Nodes =
        [
            new("number", "Number", "Values", "", [], [Num], [new SettingDef("value", "Value", new SettingKind.Number(0f))], false),
            new("add", "Add", "Math", "", [new PinDef("a", "A", PinType.Any), new PinDef("b", "B", PinType.Any)], [new PinDef("result", "Result", PinType.Any)], [], false),
            new("vector", "Vector", "Values", "", [], [new PinDef("vector", "Vector", PinType.Vec3)], [], false),
            new("out", "Output", "Output", "",
                [new PinDef("flow", "Run", PinType.Flow), new PinDef("amount", "Amount", PinType.Number), new PinDef("on", "On", PinType.Bool)],
                [new PinDef("then", "Then", PinType.Flow)], [], true),
        ],
        Starter = [("out", new Vector2(300f, 0f))],
    };

    private static Link Link(uint from, string fromPin, uint to, string toPin) => new(from, fromPin, to, toPin);

    private static JsonValue Json(string text) => JsonReader.Parse(text).Value;

    private static uint Add(Graph g, string kind) => g.AddNode(Catalog, kind, Vector2.Zero).Value;

    [Fact]
    public void IncompatiblePinsAreRefusedWithBothTypesNamed()
    {
        var g = Graph.Create(Catalog);
        var output = g.Nodes[0].Id;
        var v = Add(g, "vector");
        var error = g.Connect(Catalog, Link(v, "vector", output, "amount")).Error.Message;
        Assert.True(error.Contains("Vector", StringComparison.Ordinal) && error.Contains("Number", StringComparison.Ordinal), error);
        Assert.True(g.Connect(Catalog, Link(v, "vector", output, "on")).IsError, "vectors are not true/false");
        var n = Add(g, "number");
        Assert.True(g.Connect(Catalog, Link(n, "value", output, "amount")).IsOk);
    }

    [Fact]
    public void ALinkThatWouldCloseALoopIsRefused()
    {
        var g = Graph.Empty(Catalog);
        var a = Add(g, "add");
        var b = Add(g, "add");
        Assert.True(g.Connect(Catalog, Link(a, "result", b, "a")).IsOk);
        Assert.Contains("loop", g.Connect(Catalog, Link(b, "result", a, "a")).Error.Message, StringComparison.Ordinal);
        Assert.True(g.Connect(Catalog, Link(a, "result", a, "b")).IsError, "no self links");
    }

    [Fact]
    public void ADataInputKeepsOnlyItsNewestLink()
    {
        var g = Graph.Empty(Catalog);
        var one = Add(g, "number");
        var two = Add(g, "number");
        var add = Add(g, "add");
        Assert.True(g.Connect(Catalog, Link(one, "value", add, "a")).IsOk);
        Assert.True(g.Connect(Catalog, Link(two, "value", add, "a")).IsOk);
        Assert.Single(g.Links);
        Assert.Equal(two, g.LinkInto(add, "a")?.From.Node);
        // One output may feed many inputs.
        Assert.True(g.Connect(Catalog, Link(two, "value", add, "b")).IsOk);
        Assert.Equal(2, g.LinksFrom(two, "value").Count());
    }

    [Fact]
    public void RemovingANodeRemovesItsLinksAndUniqueKindsStayUnique()
    {
        var g = Graph.Create(Catalog);
        var output = g.Nodes[0].Id;
        Assert.True(g.AddNode(Catalog, "out", Vector2.Zero).IsError);
        var n = Add(g, "number");
        Assert.True(g.Connect(Catalog, Link(n, "value", output, "amount")).IsOk);
        Assert.True(g.RemoveNode(n).IsOk);
        Assert.Empty(g.Links);
        Assert.True(g.RemoveNode(n).IsError);
    }

    [Fact]
    public void ValuesAreCheckedAgainstThePinOrSetting()
    {
        var g = Graph.Create(Catalog);
        var output = g.Nodes[0].Id;
        Assert.True(g.SetValue(Catalog, output, "amount", Json("2.5")).IsOk);
        Assert.True(g.SetValue(Catalog, output, "amount", Json("\"lots\"")).IsError);
        Assert.True(g.SetValue(Catalog, output, "on", Json("true")).IsOk);
        Assert.True(g.SetValue(Catalog, output, "flow", Json("1")).IsError, "flow inputs hold no value");
        Assert.True(g.SetValue(Catalog, output, "missing", Json("1")).IsError);
        Assert.True(g.SetValue(Catalog, output, "amount", JsonValue.Null).IsOk);
        Assert.Null(g.ValueOf(output, "amount"));
    }

    [Fact]
    public void FilesRoundTripAndBrokenFilesAreRefused()
    {
        var g = Graph.Create(Catalog);
        var n = g.AddNode(Catalog, "number", new Vector2(10f, 20f)).Value;
        Assert.True(g.SetValue(Catalog, n, "value", Json("3.0")).IsOk);
        Assert.True(g.Connect(Catalog, Link(n, "value", g.Nodes[0].Id, "amount")).IsOk);
        var text = g.ToJson();
        Assert.Equal(text, Graph.FromJson(text, Catalog).Value.ToJson());

        string Changed(Action<JsonObject> change)
        {
            var value = (JsonObject)Json(text);
            change(value);
            return value.ToCompact();
        }
        static JsonObject At(JsonValue value, string list, int index) => (JsonObject)((JsonArray)value.Get(list)!)[index];
        var intoBool = Changed(v => ((JsonObject)At(v, "links", 0).Get("to")!).Set("pin", JsonValue.From("on")));
        Assert.True(Graph.FromJson(intoBool, Catalog).IsError, "a number into a true/false input");
        var explode = Changed(v => At(v, "nodes", 1).Set("kind", JsonValue.From("explode")));
        Assert.Contains("explode", Graph.FromJson(explode, Catalog).Error.Message, StringComparison.Ordinal);
        Assert.True(Graph.FromJson(Changed(v => v.Set("domain", JsonValue.From("material"))), Catalog).IsError);
        Assert.True(Graph.FromJson("{", Catalog).IsError);
    }

    [Fact]
    public void CommentsKeepTheirLimitsAndSurviveAFileRoundTrip()
    {
        var graph = Graph.Empty(Catalog);
        var id = graph.AddComment("Movement", new Vector2(10f, 20f), new Vector2(1f, 1f)).Value;
        var small = graph.FindComment(id)!.Size;
        Assert.True(small.X >= Graph.MinCommentSize.X && small.Y >= Graph.MinCommentSize.Y, "a tiny box grows to the smallest size");
        var longText = new string('x', Graph.MaxCommentText + 1);
        Assert.True(graph.EditComment(id, longText, null, null).IsError);
        Assert.True(graph.EditComment(id, null, new Vector2(float.NaN, 0f), null).IsError);
        Assert.Equal("Movement", graph.FindComment(id)!.Text);
        Assert.True(graph.EditComment(id, "Jumping", new Vector2(5f, 5f), null).IsOk);

        var loaded = Graph.FromJson(graph.ToJson(), Catalog).Value;
        Assert.Equal(graph.ToJson(), loaded.ToJson());
        var bad = (JsonObject)Json(graph.ToJson());
        ((JsonObject)((JsonArray)bad.Get("comments")!)[0]).Set("size", Json("[1.0, 1.0]"));
        Assert.True(Graph.FromJson(bad.ToCompact(), Catalog).IsError, "a file with a box below the smallest size is refused");
        Assert.True(graph.RemoveComment(id).IsOk);
        Assert.True(graph.RemoveComment(id).IsError);
    }
}

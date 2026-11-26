using System.Numerics;

using Bayleaf.Core.Json;
using Bayleaf.Graph;

namespace Bayleaf.Script.Tests;

public class VisualTests
{
    private static readonly Catalog Catalog = Visual.Catalog;

    private static uint Add(Graph.Graph graph, string kind, params (string Name, JsonValue Value)[] values)
    {
        var id = graph.AddNode(Catalog, kind, Vector2.Zero).Value;
        foreach (var (name, value) in values)
        {
            Assert.True(graph.SetValue(Catalog, id, name, value).IsOk, $"{kind} takes {name}");
        }
        return id;
    }

    private static JsonArray Words(params string[] words) => new(words.Select(w => (JsonValue)new JsonString(w)));

    [Fact]
    public void EachTypesVariableAndPropertyNodesCarryThatType()
    {
        foreach (var type in VariableTypes.All)
        {
            var pin = type.Pin(Shape.One, VariableType.Text);
            foreach (var (get, set) in new[] { Visual.VariableKinds(type), Visual.PropertyKinds(type) })
            {
                Assert.Equal(pin, Catalog.Get(get)?.Output("value")?.Type);
                Assert.Equal(pin, Catalog.Get(set)?.Input("value")?.Type);
            }
            Assert.Equal(type, Visual.PropertyTypeOf(Visual.PropertyKinds(type).Set));
        }
    }

    [Fact]
    public void AListPinTakesOnlyAListOfItsTypeAndAListNodeTakesTheTypeLinkedIntoIt()
    {
        var graph = Graph.Graph.Empty(Catalog);
        var words = Add(graph, "getList", ("name", new JsonString("Words")), ("item", Words("text")));
        var count = Add(graph, "listLength");
        var link = new Link(words, "list", count, "list");
        Assert.Equal((count, VariableType.Text, VariableType.Text), Visual.AdoptedItem(graph, link));
        // A list of text does not go into a list of numbers as it is.
        Assert.True(graph.Connect(Catalog, link).IsError, "a fresh list node holds numbers until it takes another type");
        Assert.True(graph.SetValue(Catalog, count, "item", Words("text")).IsOk);
        Assert.Null(Visual.AdoptedItem(graph, link));
        Assert.True(graph.Connect(Catalog, link).IsOk);
        // Nor into a number input.
        var log = Add(graph, "log");
        Assert.True(graph.Connect(Catalog, new Link(words, "list", log, "value")).IsError);
        // A map's keys' type comes along with what it holds.
        var byLevel = Add(graph, "getMap", ("name", new JsonString("byLevel")), ("item", Words("vector", "whole")));
        var value = Add(graph, "mapGet");
        Assert.Equal((value, VariableType.Vector, VariableType.WholeNumber), Visual.AdoptedItem(graph, new Link(byLevel, "map", value, "map")));
    }

    [Fact]
    public void InspectorOptionsTheHostWouldRefuseAreFoundBeforeTheScriptIsSaved()
    {
        var graph = Graph.Graph.Empty(Catalog);
        Assert.True(graph.AddProperty("speed", VariableType.Number, JsonNumber.Of(90.0)).IsOk);
        Assert.Null(Visual.InspectorLayout(graph));
        Assert.True(Visual.CheckInspector(graph).IsOk, "plain rows need no checking");

        var options = PropertyOptions.Empty with { Label = "Turn rate", Group = "Motion", Range = (0, 360), Slider = true };
        Assert.True(graph.SetPropertyOptions("speed", options).IsOk);
        Assert.True(Visual.CheckInspector(graph).TryGet(out _, out var error), error);
        var layout = Assert.IsType<JsonArray>(Visual.InspectorLayout(graph));
        var group = Assert.IsType<JsonObject>(Assert.Single(layout));
        Assert.Equal(new JsonString("Motion"), group.Find("group"));

        Assert.True(graph.SetPropertyOptions("speed", PropertyOptions.Empty with { Choices = ["a"] }).IsOk);
        Assert.Contains("choices", Visual.CheckInspector(graph).Error, StringComparison.Ordinal);
    }
}

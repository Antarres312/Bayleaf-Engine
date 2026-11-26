using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Graph;

namespace Bayleaf.Render.Tests;

public class MaterialTests
{
    private static readonly Catalog Catalog = Material.Catalog;

    private static void Connect(Graph.Graph graph, uint from, string fromPin, uint to, string toPin) =>
        Assert.True(graph.Connect(Catalog, new Link(from, fromPin, to, toPin)).IsOk, $"{from}.{fromPin} into {to}.{toPin}");

    private static uint Add(Graph.Graph graph, string kind) => graph.AddNode(Catalog, kind, Vector2.Zero).Value;

    /// <summary>A material that uses every node kind, wired into the output.</summary>
    private static Graph.Graph EveryNodeMaterial()
    {
        var g = Graph.Graph.Create(Catalog);
        var output = g.Nodes[0].Id;
        Assert.True(g.SetValue(Catalog, output, "outlineWidth", JsonNumber.Of(0.02)).IsOk);
        uint? previous = null;
        foreach (var def in Catalog.Nodes.Where(d => d.Kind is not ("output" or "customLighting")))
        {
            var id = Add(g, def.Kind);
            if (def.Kind == "texture")
            {
                Assert.True(g.SetValue(Catalog, id, "image", new JsonString("any.png")).IsOk);
            }
            // Feed the previous node into the first input that accepts it, then chain onwards.
            if (previous is { } prev && Catalog.Get(g.FindNode(prev)!.Kind)!.Outputs is [var from, ..])
            {
                // Only links that are always valid: numbers anywhere, otherwise equal types.
                if (def.Inputs.FirstOrDefault(p => from.Type == PinType.Number || (from.Type == p.Type && from.Type != PinType.Any)) is { } to)
                {
                    _ = g.Connect(Catalog, new Link(prev, from.Name, id, to.Name));
                }
            }
            previous = id;
            // Every node also adds into the emission, so each one is compiled.
            if (def.Outputs is [var first, ..])
            {
                var make = Add(g, "vector");
                if (first.Type == PinType.Number)
                {
                    Connect(g, id, first.Name, make, "x");
                }
                else
                {
                    // Through Split Vector, so vector and colour results fit too.
                    var split = Add(g, "split");
                    Connect(g, id, first.Name, split, "vector");
                    Connect(g, split, "x", make, "y");
                }
                var sum = Add(g, "add");
                Connect(g, make, "vector", sum, "b");
                if (g.LinkInto(output, "emissive") is { } existing)
                {
                    Connect(g, existing.From.Node, existing.From.Pin, sum, "a");
                }
                Connect(g, sum, "result", output, "emissive");
            }
        }
        return g;
    }

    [Fact]
    public void AMaterialUsingEveryNodeCompiles()
    {
        var graph = EveryNodeMaterial();
        Assert.True(Material.Compile(graph).TryGet(out var compiled, out var problems), problems is null ? "" : Problem.Summary(problems));
        foreach (var def in Catalog.Nodes.Where(d => d.Kind is not ("output" or "customLighting")))
        {
            Assert.Contains(graph.Nodes, n => n.Kind == def.Kind);
        }
        Assert.True(compiled.Animated, "the graph uses Time");
        Assert.True(compiled.Outline is not null, "the outline is on");
    }

    [Fact]
    public void AVectorIntoANumberInputIsAProblemOnThatNode()
    {
        var g = Graph.Graph.Create(Catalog);
        var output = g.Nodes[0].Id;
        var position = Add(g, "position");
        var add = Add(g, "add");
        Connect(g, position, "position", add, "a");
        // Add gives a vector here, so linking it into a number input is refused.
        var intoRoughness = new Link(add, "result", output, "roughness");
        Assert.True(g.Connect(Catalog, intoRoughness).IsError);
        // A file saved with such a link still opens, and the compiler names the node to mend.
        g.Links.Add(intoRoughness);
        Assert.True(Graph.Graph.FromJson(g.ToJson(), Catalog).TryGet(out var opened, out _), "it opens");
        Assert.True(Material.Compile(opened).TryGet(out _, out var problems) is false);
        Assert.Equal(output, problems[0].Node);
        Assert.Contains("Split Vector", problems[0].Message, StringComparison.Ordinal);
    }

    [Fact]
    public void WhatAMaterialNeedsFromTheRendererIsTakenFromItsGraph()
    {
        var path = Path.Join(SharedFilesTests.RepositoryRoot(), "tests", "files", "material-every-node.bmaterial");
        Assert.True(new MaterialLibrary().Get(path).TryGet(out var every, out var error), error);
        Assert.Equal(["textures/wood.png", "stone.jpg"], every.Textures);
        Assert.True(every.Transparent, "its opacity is linked");
        Assert.True(every.Animated, "it uses Time");
        Assert.True(every.Outline is { Width: <= 1f and > 0f }, "an outline no wider than the widest");

        var plain = Graph.Graph.Create(Catalog);
        Assert.True(Material.Compile(plain).TryGet(out var still, out _));
        Assert.False(still.Transparent || still.Animated || still.Outline is not null || still.Textures.Count > 0, "a new material is plain");
        Assert.NotEqual(every.Key, still.Key);
    }

    [Fact]
    public void AnEditedGraphStandsInForItsFileUntilItIsLetGo()
    {
        var dir = Directory.CreateTempSubdirectory("bayleaf-material-");
        try
        {
            var path = Path.Join(dir.FullName, "shiny.bmaterial");
            var graph = Graph.Graph.Create(Catalog);
            Assert.True(Files.Write(path, System.Text.Encoding.UTF8.GetBytes(graph.ToJson())).IsOk);
            var library = new MaterialLibrary();
            Assert.False(library.Get(path).Value.Transparent);

            var edited = graph.Clone();
            Assert.True(edited.SetValue(Catalog, edited.Nodes[0].Id, "opacity", JsonNumber.Of(0.5)).IsOk);
            library.SetLive(path, edited);
            Assert.True(library.Get(path).Value.Transparent, "the unsaved change shows");
            library.SetLive(null, null);
            Assert.False(library.Get(path).Value.Transparent, "the file again once the graph is let go");

            // A file that changes on disk is read again.
            Assert.True(Files.Write(path, System.Text.Encoding.UTF8.GetBytes(edited.ToJson())).IsOk);
            File.SetLastWriteTimeUtc(path, DateTime.UtcNow.AddMinutes(1));
            Assert.True(library.Get(path).Value.Transparent);
            Assert.True(Files.Write(path, "{"u8).IsOk);
            File.SetLastWriteTimeUtc(path, DateTime.UtcNow.AddMinutes(2));
            Assert.Contains(path, library.Get(path).Error, StringComparison.Ordinal);
        }
        finally
        {
            dir.Delete(true);
        }
    }
}

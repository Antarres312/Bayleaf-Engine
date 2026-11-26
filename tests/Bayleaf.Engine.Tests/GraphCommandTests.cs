using Bayleaf.Core.Json;
using Bayleaf.Graph;

namespace Bayleaf.Engine.Tests;

/// <summary>The <c>graph.*</c> commands, as the Rust engine's behaviour tests check them (those that need no compiled script or play mode).</summary>
public class GraphCommandTests
{
    private readonly CommandRegistry registry = CommandRegistry.WithBuiltins();
    private readonly Session session = new();

    private static JsonValue Parse(string json) => JsonReader.Parse(json).Value;

    private JsonValue Run(string cmd, string json = "null") => registry.Execute(session, cmd, Parse(json)).Value;

    private bool Fails(string cmd, string json) => registry.Execute(session, cmd, Parse(json)).IsError;

    private string Refused(string cmd, string json) => registry.Execute(session, cmd, Parse(json)).Error.Message;

    private uint Node(string kind) => (uint)Run("graph.node.add", $$"""{"kind": "{{kind}}", "position": [0, 0]}""").Get("node")!.U();

    private static string Pins(uint from, string fromPin, uint to, string toPin) =>
        $$$"""{"from": {"node": {{{from}}}, "pin": "{{{fromPin}}}"}, "to": {"node": {{{to}}}, "pin": "{{{toPin}}}"}}""";

    private Bayleaf.Graph.Graph Open => session.Graph!.Graph;

    private static string TempDir(string name)
    {
        var dir = Path.Join(Path.GetTempPath(), $"bayleaf-{name}-cs-{Environment.ProcessId}");
        if (Directory.Exists(dir))
        {
            Directory.Delete(dir, true);
        }
        Directory.CreateDirectory(dir);
        return dir;
    }

    private static string Quoted(string path) => new JsonString(path).ToCompact();

    [Fact]
    public void GraphEditsShareTheUndoHistoryAndSavingClearsDirty()
    {
        Assert.True(Fails("graph.node.add", """{"kind": "number"}"""), "no graph open");
        Run("graph.new", """{"domain": "material"}""");
        Run("scene.object.create", """{"name": "Scene edit"}""");
        var color = Node("color");
        var output = Open.Nodes[0].Id;
        Run("graph.link.add", Pins(color, "color", output, "baseColor"));
        Assert.True(session.Graph!.Dirty);
        Assert.Equal(["scene.object.create", "graph.node.add", "graph.link.add"], session.History);

        // A refused link names the clash and changes nothing.
        Assert.Contains("Number", Refused("graph.link.add", Pins(color, "color", output, "roughness")), StringComparison.Ordinal);
        Assert.Equal(3, session.History.Count);

        Run("edit.undo");
        Assert.Empty(Open.Links);
        Run("edit.redo");
        Assert.Single(Open.Links);

        var dir = TempDir("graph-save");
        try
        {
            Assert.True(Fails("graph.save", "null"), "a new graph needs a path");
            Assert.True(Fails("graph.save", $$"""{"path": {{Quoted(Path.Join(dir, "red.bscript"))}}}"""), "materials end in .bmaterial");
            Run("graph.save", $$"""{"path": {{Quoted(Path.Join(dir, "red.bmaterial"))}}}""");
            Assert.False(session.Graph.Dirty);
            var saved = Open.ToJson();

            // Opening a graph that is open brings it forward, with its undo steps.
            Run("graph.open", $$"""{"path": {{Quoted(Path.Join(dir, "red.bmaterial"))}}}""");
            Assert.Equal(saved, Open.ToJson());
            Assert.Single(session.Graphs);
            Assert.Equal(3, session.History.Count);
            Assert.Empty(Run("graph.get").Get("problems")!.AsArray()!);
            // Closing a graph takes its undo steps with it, and keeps the scene's.
            Run("graph.close");
            Assert.Equal(["scene.object.create"], session.History);
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }

    [Fact]
    public void HostileGraphParamsChangeNothing()
    {
        Run("graph.new", """{"domain": "script"}""");
        var start = Open.Nodes[0].Id;
        string[] hostile =
        [
            """{"kind": "../../etc", "position": [0, 0]}""",
            """{"kind": "number", "position": [1e39, 0]}""",
            """{"node": 99999, "position": [0, 0]}""",
            """{"node": -1}""",
            $$"""{"node": {{start}}, "name": "missing", "value": 1}""",
            $$"""{"node": {{start}}, "pin": "then"}""",
            $$$"""{"from": {"node": {{{start}}}, "pin": "then"}, "to": {"node": {{{start}}}, "pin": "run"}}""",
            $$$"""{"from": {"node": {{{start}}}, "pin": "nope"}, "to": {"node": 7, "pin": "run"}}""",
            """{"domain": "unknown"}""",
            """{"path": "../../escape.bscript"}""",
        ];
        var ids = registry.Commands.Select(c => c.Id)
            .Where(id => id.StartsWith("graph.", StringComparison.Ordinal) && id is not ("graph.new" or "graph.close" or "graph.open" or "graph.save"))
            .ToList();
        Assert.NotEmpty(ids);
        foreach (var id in ids)
        {
            foreach (var bad in hostile)
            {
                var before = session.Graph?.Graph.ToJson();
                if (Fails(id, bad))
                {
                    Assert.True(before == session.Graph?.Graph.ToJson(), $"{id} changed the graph on failure with {bad}");
                }
            }
        }
    }

    [Fact]
    public void CopiedNodesPasteWithNewIdsAndTheirLinksAndOneOfAKindNodesStaySingle()
    {
        Run("graph.new", """{"domain": "material"}""");
        var output = Open.Nodes[0].Id;
        var color = Node("color");
        var blend = Node("blend");
        Run("graph.node.set", $$"""{"node": {{color}}, "name": "color", "value": [0.1, 0.2, 0.3]}""");
        Run("graph.link.add", Pins(color, "color", blend, "a"));
        Run("graph.link.add", Pins(blend, "result", output, "baseColor"));

        var text = Run("graph.node.copy", $$"""{"nodes": [{{color}}, {{blend}}, {{output}}]}""").Get("text")!.AsString()!;
        var pasted = Run("graph.node.paste", $$"""{"text": {{Quoted(text)}}}""");
        Assert.Equal(["Material Output"], pasted.Get("skipped")!.AsArray()!.Select(s => s.AsString()));
        var copies = pasted.Get("nodes")!.AsArray()!.Select(v => (uint)v.U()).ToList();
        Assert.Equal(5, Open.Nodes.Count);
        Assert.All(copies, id => Assert.DoesNotContain(id, new[] { color, blend, output }));
        Assert.Contains(Open.Links, l => l.From.Node == copies[0] && l.To.Node == copies[1]);
        Assert.Single(Open.Links, l => l.To.Node == output);
        Assert.Equal(Open.ValueOf(color, "color"), Open.ValueOf(copies[0], "color"));

        Run("edit.undo");
        Assert.Equal(3, Open.Nodes.Count);
        Assert.Single(Run("graph.node.duplicate", $$"""{"nodes": [{{blend}}]}""").Get("nodes")!.AsArray()!);
        Assert.True(Fails("graph.node.duplicate", $$"""{"nodes": [{{output}}]}"""), "nothing to duplicate");

        Run("graph.new", """{"domain": "script"}""");
        Assert.Contains("material nodes", Refused("graph.node.paste", $$"""{"text": {{Quoted(text)}}}"""), StringComparison.Ordinal);
    }

    [Fact]
    public void VisualScriptVariablesAreDeclaredRenamedWithTheirNodesUndoneAndSaved()
    {
        Run("graph.new", """{"domain": "script"}""");
        Run("graph.variable.add", """{"name": "lives", "type": "number", "start": 3}""");
        var node = Node("setVariable");
        Run("graph.node.set", $$"""{"node": {{node}}, "name": "name", "value": "lives"}""");
        Assert.True(Fails("graph.variable.add", """{"name": "lives", "type": "vector"}"""), "names are unique");
        Assert.True(Fails("graph.variable.add", """{"name": "on", "type": "trueFalse", "start": 2}"""), "the start fits the type");

        Run("graph.variable.set", """{"name": "lives", "rename": "hearts", "start": 5}""");
        var graph = Run("graph.get").Get("graph")!;
        Assert.Equal("""{"name":"hearts","start":5,"type":"number"}""", ((JsonArray)graph.Get("variables")!)[0].Canonical().ToCompact());
        var setNode = ((JsonArray)graph.Get("nodes")!).First(n => n.Get("kind")!.AsString() == "setVariable");
        Assert.Equal("hearts", setNode.Get("values")!.Get("name")!.AsString());
        Run("edit.undo");
        Assert.Equal("lives", ((JsonArray)Run("graph.get").Get("graph")!.Get("variables")!)[0].Get("name")!.AsString());

        var dir = TempDir("variables");
        try
        {
            var path = Quoted(Path.Join(dir, "variables.bscript"));
            Run("graph.save", $$"""{"path": {{path}}}""");
            Run("graph.open", $$"""{"path": {{path}}}""");
            Assert.Equal(3.0, ((JsonArray)Run("graph.get").Get("graph")!.Get("variables")!)[0].Get("start")!.AsDouble());
            Run("graph.variable.remove", """{"name": "lives"}""");
        }
        finally
        {
            Directory.Delete(dir, true);
        }

        Run("graph.new", """{"domain": "material"}""");
        Assert.True(Fails("graph.variable.add", """{"name": "x", "type": "number"}"""), "materials have no variables");
    }

    [Fact]
    public void AVisualScriptsMenuPathIsACheckedFieldThatUndoes()
    {
        Run("graph.new", """{"domain": "script"}""");
        Assert.Equal("Light/Flicker", Run("graph.menu.set", """{"path": " Light / Flicker "}""").Get("path")!.AsString());
        Assert.Equal("Light/Flicker", Run("graph.get").Get("graph")!.Get("menu")?.AsString());
        Assert.True(Fails("graph.menu.set", """{"path": "Light//Flicker"}"""), "an empty part is refused");
        Run("edit.undo");
        Assert.Null(Run("graph.get").Get("graph")!.Get("menu"));
        Run("graph.new", """{"domain": "material"}""");
        Assert.True(Fails("graph.menu.set", """{"path": "Light/Flicker"}"""), "materials have no menu path");
    }

    [Fact]
    public void AVisualScriptPropertyTakesInspectorOptionsTheHostAccepts()
    {
        Run("graph.new", """{"domain": "script"}""");
        Run("graph.property.add", """{"name": "speed", "type": "number"}""");
        Run("graph.property.set", """{"name": "speed", "inspector": {"label": "Top speed", "range": [0, 10], "slider": true, "group": "Motion"}}""");
        var kept = Open.FindProperty("speed")!.Inspector;
        Assert.Equal(("Top speed", true), (kept.Label, kept.Slider));
        // A slider with no range is refused, and the options stay as they were.
        var refused = registry.Execute(session, "graph.property.set", Parse("""{"name": "speed", "inspector": {"slider": true}}"""));
        Assert.Equal(CommandErrorKind.BadParams, refused.Error.Kind);
        Assert.Contains("range", refused.Error.Message, StringComparison.Ordinal);
        Assert.Equal(kept, Open.FindProperty("speed")!.Inspector);
    }

    [Fact]
    public void SeveralGraphsStayOpenAndUndoGoesBackInTheGraphEachStepWasMadeIn()
    {
        var material = Run("graph.new", """{"domain": "material"}""").Get("id")!.U();
        Node("color");
        var script = Run("graph.new", """{"domain": "script"}""").Get("id")!.U();
        Node("log");
        var open = ((JsonArray)Run("graph.list").Get("graphs")!).Select(g => (g.Get("id")!.U(), g.Get("inUse")!.AsBool()!.Value)).ToList();
        Assert.Equal([(material, false), (script, true)], open);

        // Undo takes back the script's node, then the material's, each in its own graph.
        int Nodes(ulong id) => session.GraphById(id)!.Graph.Nodes.Count;
        var (materialNodes, scriptNodes) = (Nodes(material), Nodes(script));
        Run("edit.undo");
        Assert.Equal((materialNodes, scriptNodes - 1), (Nodes(material), Nodes(script)));
        Run("edit.undo");
        Assert.Equal((materialNodes - 1, scriptNodes - 1), (Nodes(material), Nodes(script)));
        Assert.Equal(material, session.Graph!.Id);

        // Commands work on the graph in use; activate picks another.
        Run("graph.activate", $$"""{"id": {{script}}}""");
        Node("log");
        Assert.Equal(scriptNodes, Nodes(script));
        Assert.True(Fails("graph.activate", """{"id": 999}"""));
        // Closing one leaves the other in use.
        Run("graph.close", $$"""{"id": {{script}}}""");
        Assert.Single(session.Graphs);
        Assert.Equal(material, session.Graph!.Id);
    }

    [Fact]
    public void AnEventsNodesFollowItsNewNameAndValues()
    {
        Run("graph.new", """{"domain": "script"}""");
        Run("graph.event.add", """{"name": "Open"}""");
        Assert.True(Fails("graph.event.add", """{"name": "Open"}"""), "one event of a name");
        var on = Node("eventStart");
        var trigger = Node("triggerEvent");
        foreach (var node in new[] { on, trigger })
        {
            Run("graph.node.set", $$"""{"node": {{node}}, "name": "event", "value": "Open"}""");
        }
        Run("graph.event.set", """{"name": "Open", "rename": "OpenDoor", "params": [{"name": "speed", "type": "number"}]}""");
        foreach (var node in new[] { on, trigger })
        {
            Assert.Equal("OpenDoor", Open.ValueOf(node, "event")?.AsString());
            Assert.Equal("""["speed:number"]""", Open.ValueOf(node, "params")?.ToCompact());
        }
        Assert.True(Fails("graph.event.set", """{"name": "OpenDoor", "params": [{"name": "run", "type": "number"}]}"""), "a flow pin's name");
        Run("graph.event.remove", """{"name": "OpenDoor"}""");
        Assert.Empty(Open.Events);
    }

    [Fact]
    public void AVariablesNodesFollowANewTypeAndShapeAndLoseLinksThatNoLongerFit()
    {
        Run("graph.new", """{"domain": "script"}""");
        Run("graph.variable.add", """{"name": "count", "type": "number", "start": 3.0}""");
        var get = Node("variable");
        Run("graph.node.set", $$"""{"node": {{get}}, "name": "name", "value": "count"}""");
        var log = Node("log");
        Run("graph.link.add", Pins(get, "value", log, "value"));

        // A number becomes text: its Get node becomes a Get of text, and the number link goes.
        Run("graph.variable.set", """{"name": "count", "type": "text"}""");
        Assert.Equal("textVariable", Open.FindNode(get)!.Kind);
        Assert.Equal("\"\"", Open.FindVariable("count")!.Start.ToCompact());
        Assert.DoesNotContain(Open.Links, l => l.From.Node == get);

        // One value becomes a list, which starts with items and gets list nodes.
        Run("graph.variable.set", """{"name": "count", "shape": "list", "start": ["a", "b"]}""");
        Assert.True(Fails("graph.variable.set", """{"name": "count", "start": [1, 2]}"""), "a list of text holds text");
        var add = Node("listAdd");
        Run("graph.node.set", $$"""{"node": {{add}}, "name": "item", "value": ["text"]}""");
        Run("graph.node.set", $$"""{"node": {{add}}, "name": "name", "value": "count"}""");
        var name = Node("nameOf");
        Run("graph.link.add", Pins(name, "result", add, "value"));
        Run("graph.variable.set", """{"name": "count", "type": "number", "rename": "scores"}""");
        Assert.Equal("scores", Open.ValueOf(add, "name")?.AsString());
        Assert.Equal("""["number","text"]""", Open.ValueOf(add, "item")?.ToCompact());
        Assert.DoesNotContain(Open.Links, l => l.To.Node == add && l.To.Pin == "value");
        Assert.Equal("[]", Open.FindVariable("scores")!.Start.ToCompact());

        // A property's shape and type change the same way.
        Run("graph.property.add", """{"name": "waves", "type": "wholeNumber", "shape": "map", "default": {"easy": 3}}""");
        Run("graph.property.set", """{"name": "waves", "shape": "list"}""");
        Assert.Equal("[]", Open.FindProperty("waves")!.Default.ToCompact());
    }

    [Fact]
    public void LinkingAListIntoAListNodeGivesTheNodeItsTypeInOneStep()
    {
        Run("graph.new", """{"domain": "script"}""");
        Run("graph.variable.add", """{"name": "words", "type": "text", "shape": "list"}""");
        var get = Node("getList");
        Run("graph.node.set", $$"""{"node": {{get}}, "name": "item", "value": ["text"]}""");
        Run("graph.node.set", $$"""{"node": {{get}}, "name": "name", "value": "words"}""");
        // A fresh For Each holds numbers; a number goes into nothing it has, so no link is lost.
        var each = Node("listForEach");
        Run("graph.link.add", Pins(get, "list", each, "list"));
        Assert.Equal("""["text","text"]""", Open.ValueOf(each, "item")?.ToCompact());
        Assert.Contains(Open.Links, l => l.To.Node == each && l.To.Pin == "list");
        // Undo takes back both.
        Run("edit.undo");
        Assert.Null(Open.ValueOf(each, "item"));
    }

    [Fact]
    public void AMapsKeyTypeChangesAndItsNodesAndStartFollow()
    {
        Run("graph.new", """{"domain": "script"}""");
        Run("graph.variable.add", """{"name": "waves", "type": "text", "shape": "map", "start": {"1": "one", "easy": "?"}}""");
        Assert.True(Fails("graph.variable.add", """{"name": "bad", "type": "text", "shape": "map", "key": "wholeNumber", "start": {"easy": "?"}}"""), "easy is no whole number");
        var set = Node("mapSet");
        Run("graph.node.set", $$"""{"node": {{set}}, "name": "item", "value": ["text", "text"]}""");
        Run("graph.node.set", $$"""{"node": {{set}}, "name": "name", "value": "waves"}""");
        Run("graph.variable.set", """{"name": "waves", "key": "wholeNumber"}""");
        Assert.Equal("""["text","whole"]""", Open.ValueOf(set, "item")?.ToCompact());
        Assert.Equal("{}", Open.FindVariable("waves")!.Start.ToCompact());
        var def = Open.NodeWithDef(Script.Visual.Catalog, set).Value.Def;
        Assert.Equal(PinType.WholeNumber, def.Input("key")?.Type);
    }

    [Fact]
    public void AVisualScriptsProblemsSayItIsNotCompiledRatherThanThatNothingIsWrong()
    {
        Run("graph.new", """{"domain": "script"}""");
        var problems = Run("graph.get").Get("problems")!.AsArray()!;
        Assert.Contains("not compiled", Assert.Single(problems).Get("message")!.AsString(), StringComparison.Ordinal);
    }
}

internal static class JsonNumbers
{
    public static ulong U(this JsonValue value) => ((JsonNumber)value).AsUInt64()!.Value;
}
